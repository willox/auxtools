use super::instruction_hooking::{hook_instruction, unhook_instruction};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::{error::Error};
use std::{
	net::{SocketAddr, TcpListener, TcpStream},
	thread::JoinHandle,
};

use super::server_types::*;
use dm::*;

//
// Server = main-thread code
// ServerThread = networking-thread code
//
// We've got a couple channels going on between Server/ServerThread
// connection: a TcpStream sent from the ServerThread for the Server to send responses on
// requests: requests from the debug-client for the Server to handle
//
// TODO: shutdown logic
//

pub struct Server {
	connection: mpsc::Receiver<TcpStream>,
	requests: mpsc::Receiver<Request>,
	stacks: Option<CallStacks>,
	stream: Option<TcpStream>,
	_thread: JoinHandle<()>,
}

struct ServerThread {
	connection: mpsc::Sender<TcpStream>,
	requests: mpsc::Sender<Request>,
	listener: TcpListener,
	stream: Option<TcpStream>,
}

impl Server {
	pub fn listen(addr: &SocketAddr) -> std::io::Result<Server> {
		let (connection_sender, connection_receiver) = mpsc::channel();
		let (requests_sender, requests_receiver) = mpsc::channel();

		let thread = ServerThread {
			connection: connection_sender,
			requests: requests_sender,
			listener: TcpListener::bind(addr)?,
			stream: None,
		};

		Ok(Server {
			connection: connection_receiver,
			requests: requests_receiver,
			stacks: None,
			stream: None,
			_thread: thread.start_thread(),
		})
	}

	fn get_line_number(&self, proc: ProcRef, offset: u32) -> Option<u32> {
		match dm::Proc::find_override(proc.path, proc.override_id) {
			Some(proc) => {
				// We're ignoring disassemble errors because any bytecode in the result is still valid
				// stepping over unknown bytecode still works, but trying to set breakpoints in it can fail
				let (dism, _) = proc.disassemble();
				let mut current_line_number = None;
				let mut reached_offset = false;

				for (instruction_offset, _, instruction) in dism {
					if let Instruction::DbgLine(line) = instruction {
						current_line_number = Some(line);
					}

					if instruction_offset == offset {
						reached_offset = true;
						break;
					}
				}

				if reached_offset {
					return current_line_number;
				} else {
					return None;
				}
			}

			None => None,
		}
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::BreakpointSet { instruction } => {
				let line = self.get_line_number(instruction.proc.clone(), instruction.offset);

				// TODO: better error handling
				match dm::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
					Some(proc) => match hook_instruction(&proc, instruction.offset) {
						Ok(()) => {
							self.send_or_disconnect(Response::BreakpointSet {
								result: BreakpointSetResult::Success { line },
							});
						}

						Err(_) => {
							self.send_or_disconnect(Response::BreakpointSet {
								result: BreakpointSetResult::Failed,
							});
						}
					},

					None => {
						self.send_or_disconnect(Response::BreakpointSet {
							result: BreakpointSetResult::Failed,
						});
					}
				}
			}

			Request::BreakpointUnset { instruction } => {
				match dm::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
					Some(proc) => match unhook_instruction(&proc, instruction.offset) {
						Ok(()) => {
							self.send_or_disconnect(Response::BreakpointUnset { success: true });
						}

						Err(_) => {
							self.send_or_disconnect(Response::BreakpointUnset { success: false });
						}
					},

					None => {
						self.send_or_disconnect(Response::BreakpointUnset { success: false });
					}
				}
			}

			Request::LineNumber { proc, offset } => {
				self.send_or_disconnect(Response::LineNumber {
					line: self.get_line_number(proc, offset),
				});
			}

			Request::Offset { proc, line } => {
				match dm::Proc::find_override(proc.path, proc.override_id) {
					Some(proc) => {
						// We're ignoring disassemble errors because any bytecode in the result is still valid
						// stepping over unknown bytecode still works, but trying to set breakpoints in it can fail
						let (dism, _) = proc.disassemble();
						let mut offset = None;
						let mut at_offset = false;

						for (instruction_offset, _, instruction) in dism {
							if at_offset {
								offset = Some(instruction_offset);
								break;
							}
							if let Instruction::DbgLine(current_line) = instruction {
								if current_line == line {
									at_offset = true;
								}
							}
						}

						self.send_or_disconnect(Response::Offset { offset });
					}

					None => {
						self.send_or_disconnect(Response::Offset { offset: None });
					}
				}
			}

			Request::StackFrames {
				thread_id,
				start_frame,
				count,
			} => {
				assert_eq!(thread_id, 0);

				self.send_or_disconnect(match &self.stacks {
					Some(stacks) => {
						let stack = &stacks.active;
						let start_frame = start_frame.unwrap_or(0);
						let end_frame = start_frame + count.unwrap_or(stack.len() as u32);

						let start_frame = start_frame as usize;
						let end_frame = end_frame as usize;

						let mut frames = vec![];

						for i in start_frame..end_frame {
							if i >= stack.len() {
								break;
							}

							let proc_ref = ProcRef {
								path: stack[i].proc.path.to_owned(),
								override_id: 0,
							};

							frames.push(StackFrame {
								instruction: InstructionRef {
									proc: proc_ref.clone(),
									offset: stack[i].offset as u32,
								},
								line: self.get_line_number(proc_ref, stack[i].offset as u32),
							});
						}

						Response::StackFrames {
							frames,
							total_count: stack.len() as u32,
						}
					}

					None => {
						Response::StackFrames {
							frames: vec![],
							total_count: 0,
						}
					}
				});
			}

			Request::Continue { .. } => {
				eprintln!("Debug server received a continue request when not paused. Ignoring.");
			}

			Request::Pause => {
				return true;
			}
		}

		false
	}

	pub fn handle_breakpoint(
		&mut self,
		_ctx: *mut raw_types::procs::ExecutionContext,
		reason: BreakpointReason,
	) -> ContinueKind {
		// Cache these now so nothing else has to fetch them
		// TODO: it'd be cool if all this data was fetched lazily
		self.stacks = Some(CallStacks::new(&DMContext {}));

		self.send_or_disconnect(Response::BreakpointHit { reason });

		while let Ok(request) = self.requests.recv() {
			// Hijack and handle any Continue requests
			if let Request::Continue { kind } = request {
				self.stacks = None;
				return kind;
			}

			// if we get a pause request here we can ignore it
			self.handle_request(request);
		}

		// Client disappeared?
		self.stacks = None;
		ContinueKind::Continue
	}

	// returns true if we need to pause
	pub fn process(&mut self) -> bool {
		// Don't do anything until we've got a stream
		if self.stream.is_none() {
			if let Ok(stream) = self.connection.try_recv() {
				self.stream = Some(stream);
			} else {
				return false;
			}
		}

		let mut should_pause = false;

		while let Ok(request) = self.requests.try_recv() {
			should_pause = should_pause || self.handle_request(request);
		}

		should_pause
	}

	fn send_or_disconnect(&mut self, response: Response) {
		if self.stream.is_none() {
			return;
		}

		match self.send(response) {
			Ok(_) => {},
			Err(e) => {
				eprintln!("Debug server failed to send message: {}", e);
				self.stream = None;
			}
		}
	}

	fn send(&mut self, response: Response) -> Result<(), Box<dyn std::error::Error>> {
		let mut message = serde_json::to_vec(&response)?;
		let stream = self.stream.as_mut().unwrap();
		message.push(0); // null-terminator
		stream.write_all(&message[..])?;
		stream.flush()?;
		Ok(())
	}

}

impl ServerThread {
	fn start_thread(mut self) -> JoinHandle<()> {
		thread::spawn(move || {
			match self.listener.accept() {
				Ok((stream, _)) => {
					self.stream = Some(stream);
					self.run();
				}

				Err(e) => {
					println!("Debug server failed to accept connection {}", e);
				}
			}
		})
	}

	fn handle_request(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
		let request = serde_json::from_slice::<Request>(data)?;
		self.requests.send(request)?;
		Ok(())
	}

	fn run(mut self) {
		match self.connection.send(self.stream.as_mut().unwrap().try_clone().unwrap()) {
			Ok(_) => {}
			Err(e) => {
				eprintln!("Debug server thread failed to pass cloned TcpStream: {}", e);
				return;
			}
		}

		let mut buf = [0u8; 4096];
		let mut queued_data = vec![];

		// The incoming stream is JSON objects separated by null terminators.
		loop {
			match self.stream.as_mut().unwrap().read(&mut buf) {
				Ok(0) => return,

				Ok(n) => {
					queued_data.extend_from_slice(&buf[..n]);
				}

				Err(e) => {
					eprintln!("Debug server thread read error: {}", e);
					return;
				}
			}

			for message in queued_data.split(|x| *x == 0) {
				// split can give us empty slices
				if message.is_empty() {
					continue;
				}

				match self.handle_request(message) {
					Ok(_) => {}

					Err(e) => {
						eprintln!("Debug server thread failed to handle request: {}", e);
						return;
					}
				}
			}

			// Clear any finished messages from the buffer
			if let Some(idx) = queued_data.iter().rposition(|x| *x == 0) {
				queued_data.drain(..idx);
			}
		}
	}
}
