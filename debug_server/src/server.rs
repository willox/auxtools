use super::instruction_hooking::{hook_instruction, unhook_instruction};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::{collections::HashMap, collections::HashSet, error::Error};
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
// We've got a few channels going on between Server/ServerThread
// requests: requests from the debug-client for the Server to handle
// responses: responses from the Server for the debug-client
// notifications: similar to responses, but separated because requests/responses are expected to be in-order by the client.
//
// TODO: shutdown logic
//

pub struct Server {
	requests: mpsc::Receiver<Request>,
	responses: mpsc::Sender<Response>,
	notifications: mpsc::Sender<Response>,
	stacks: Option<CallStacks>,
	_thread: JoinHandle<()>,
}

pub struct ServerThread {
	requests: mpsc::Sender<Request>,
	responses: mpsc::Receiver<Response>,
	notifications: mpsc::Receiver<Response>,
	listener: TcpListener,
	stream: Option<TcpStream>,
}

impl Server {
	pub fn new(addr: &SocketAddr) -> std::io::Result<Server> {
		let (requests_sender, requests_receiver) = mpsc::channel();
		let (responses_sender, responses_receiver) = mpsc::channel();
		let (notifications_sender, notifications_receiver) = mpsc::channel();

		let thread = ServerThread {
			requests: requests_sender,
			responses: responses_receiver,
			notifications: notifications_receiver,
			listener: TcpListener::bind(addr)?,
			stream: None,
		};

		Ok(Server {
			requests: requests_receiver,
			responses: responses_sender,
			notifications: notifications_sender,
			stacks: None,
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
							self.responses
								.send(Response::BreakpointSet {
									result: BreakpointSetResult::Success { line },
								})
								.unwrap();
						}

						Err(_) => {
							self.responses
								.send(Response::BreakpointSet {
									result: BreakpointSetResult::Failed,
								})
								.unwrap();
						}
					},

					None => {
						self.responses
							.send(Response::BreakpointSet {
								result: BreakpointSetResult::Failed,
							})
							.unwrap();
					}
				}
			}

			Request::BreakpointUnset { instruction } => {
				match dm::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
					Some(proc) => match unhook_instruction(&proc, instruction.offset) {
						Ok(()) => {
							self.responses
								.send(Response::BreakpointUnset { success: true })
								.unwrap();
						}

						Err(_) => {
							self.responses
								.send(Response::BreakpointUnset { success: false })
								.unwrap();
						}
					},

					None => {
						self.responses
							.send(Response::BreakpointUnset { success: false })
							.unwrap();
					}
				}
			}

			Request::LineNumber { proc, offset } => {
				let response = Response::LineNumber {
					line: self.get_line_number(proc, offset),
				};

				self.responses.send(response);
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

						self.responses.send(Response::Offset { offset }).unwrap();
					}

					None => {
						self.responses
							.send(Response::Offset { offset: None })
							.unwrap();
					}
				}
			}

			Request::StackFrames {
				thread_id,
				start_frame,
				count,
			} => {
				assert_eq!(thread_id, 0);

				match &self.stacks {
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

						self.responses
							.send(Response::StackFrames {
								frames,
								total_count: stack.len() as u32,
							})
							.unwrap();
					}

					None => {
						self.responses
							.send(Response::StackFrames {
								frames: vec![],
								total_count: 0,
							})
							.unwrap();
					}
				}
			}

			Request::Continue { kind } => {
				// TODO: Handle better
				panic!("Client sent a continue request when we weren't broken?");
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
		self.notifications
			.send(Response::BreakpointHit { reason })
			.unwrap();

		// Cache these now so nothing else has to fetch them
		self.stacks = Some(CallStacks::new(&DMContext {}));

		while let Ok(request) = self.requests.recv() {
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
		let mut should_pause = false;

		while let Ok(request) = self.requests.try_recv() {
			should_pause = should_pause || self.handle_request(request);
		}

		should_pause
	}
}

impl ServerThread {
	pub fn start_thread(mut self) -> JoinHandle<()> {
		thread::spawn(move || {
			for incoming in self.listener.incoming() {
				match incoming {
					Ok(stream) => {
						stream.set_nonblocking(true).unwrap();
						self.stream = Some(stream);
						self.run();
						return;
					}

					Err(e) => {
						println!("Connection failure {:?}", e);
					}
				}
			}
		})
	}

	fn send(&mut self, response: Response) {
		let mut message = serde_json::to_vec(&response).unwrap();
		message.push(0); // null-terminator
		self.stream
			.as_mut()
			.unwrap()
			.write_all(&message[..])
			.unwrap();
	}

	fn handle_message(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
		let request = serde_json::from_slice::<Request>(data)?;
		self.requests.send(request)?;
		Ok(())
	}

	fn run(mut self) {
		let mut buf = [0u8; 4096];
		let mut queued_data = vec![];

		// The incoming stream is JSON objects separated by null terminators.
		loop {
			let mut got_data = false;
			match self.stream.as_mut().unwrap().read(&mut buf) {
				Ok(0) => (),
				Ok(n) => {
					queued_data.extend_from_slice(&buf[..n]);
					got_data = true;
				}

				// This is a crutch
				Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
				Err(_) => panic!("Handle me!"),
			}

			for message in queued_data.split(|x| *x == 0) {
				// split can give us empty slices
				if message.is_empty() {
					continue;
				}

				self.handle_message(message).unwrap();
			}

			// Clear any finished messages from the buffer
			if let Some(idx) = queued_data.iter().rposition(|x| *x == 0) {
				queued_data.drain(..idx);
			}

			// Send any responses to the client
			while let Ok(response) = self.responses.try_recv() {
				self.send(response);
			}

			while let Ok(response) = self.notifications.try_recv() {
				self.send(response);
			}
		}
	}
}
