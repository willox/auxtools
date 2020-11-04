// This whole thing is terrible but i've gotta see some output

use super::instruction_hooking::{hook_instruction, unhook_instruction};
use std::error::Error;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::{
	net::{SocketAddr, TcpListener, TcpStream},
	thread::JoinHandle,
};

use dm::*;
use super::server_types::*;

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
		let (notification_sender, notification_receiver) = mpsc::channel();

		let thread = ServerThread {
			requests: requests_sender,
			responses: responses_receiver,
			notifications: notification_receiver,
			listener: TcpListener::bind(addr)?,
			stream: None,
		};

		Ok(Server {
			requests: requests_receiver,
			responses: responses_sender,
			notifications: notification_sender,
			_thread: thread.start_thread(),
		})
	}

	fn handle_request(&mut self, request: Request) {
		match request {
			Request::BreakpointSet { proc, offset } => {
				// TODO: better error handling
				match dm::Proc::find_override(proc.path, proc.override_id) {
					Some(proc) => {
						match hook_instruction(&proc, offset) {
							Ok(()) => {
								self.responses.send(Response::BreakpointSet { success: true }).unwrap();
							},

							Err(_) => {
								self.responses.send(Response::BreakpointSet { success: false }).unwrap();
							}
						}
					}

					None => {
						self.responses.send(Response::BreakpointSet { success: false }).unwrap();
					}
				}
			},

			Request::BreakpointUnset { proc, offset } => {
				match dm::Proc::find_override(proc.path, proc.override_id) {
					Some(proc) => {
						match unhook_instruction(&proc, offset) {
							Ok(()) => {
								self.responses.send(Response::BreakpointUnset { success: true }).unwrap();
							},

							Err(_) => {
								self.responses.send(Response::BreakpointUnset { success: false }).unwrap();
							}
						}
					}

					None => {
						self.responses.send(Response::BreakpointUnset { success: false }).unwrap();
					}
				}
			},

			Request::LineNumber { proc, offset } => {
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
							self.responses.send(Response::LineNumber { line: current_line_number }).unwrap();
						} else {
							self.responses.send(Response::LineNumber { line: None }).unwrap();
						}
					}

					None => {
						self.responses.send(Response::LineNumber { line: None }).unwrap();
					}
				}
			}

			Request::Offset { proc, line } => {
				match dm::Proc::find_override(proc.path, proc.override_id) {
					Some(proc) => {
						// We're ignoring disassemble errors because any bytecode in the result is still valid
						// stepping over unknown bytecode still works, but trying to set breakpoints in it can fail
						let (dism, _) = proc.disassemble();
						let mut offset = None;

						for (instruction_offset, _, instruction) in dism {
							if let Instruction::DbgLine(current_line) = instruction {
								if current_line == line  {
									offset = Some(instruction_offset);
									break;
								}
							}
						}

						self.responses.send(Response::LineNumber { line: offset }).unwrap();
					}

					None => {
						self.responses.send(Response::LineNumber { line: None }).unwrap();
					}
				}
			}

			Request::Continue { kind } => {
				// TODO: Handle better
				panic!("Client sent a continue request when we weren't broken?");
			}
		}
	}

	pub fn handle_breakpoint(&mut self, ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> ContinueKind {
		let (proc_id, offset) = unsafe {
			let instance = (*ctx).proc_instance;
			((*instance).proc ,(*ctx).bytecode_offset as u32)
		};

		let proc = Proc::from_id(proc_id).unwrap();

		self.notifications.send(Response::BreakpointHit {
			proc: ProcRef {
				path: proc.path,
				override_id: 0, // TODO: We have no way to fetch this yet
			},
			offset,
			reason,
		}).unwrap();

		while let Ok(request) = self.requests.recv() {
			if let Request::Continue { kind } = request {
				return kind;
			}

			self.handle_request(request);
		}

		// Client disappeared?
		ContinueKind::Continue
	}

	pub fn process(&mut self) {
		while let Ok(request) = self.requests.try_recv() {
			self.handle_request(request);
		}
	}
}

impl ServerThread {
	pub fn start_thread(mut self) -> JoinHandle<()> {
		thread::spawn(move || {
			for incoming in self.listener.incoming() {
				match incoming {
					Ok(stream) => {
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
		self.stream.as_mut().unwrap().write_all(&message[..]).unwrap();
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
			match self.stream.as_mut().unwrap().read(&mut buf) {
				Ok(0) => (),
				Ok(n) => {
					queued_data.extend_from_slice(&buf[..n]);
				}
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
