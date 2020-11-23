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
use dm::raw_types::values::{ValueTag, ValueData};
use super::server_types::*;

//
// Server = main-thread code
// ServerThread = networking-thread code
//
// We've got a couple of channels going on between Server/ServerThread
// connection: a TcpStream sent from the ServerThread for the Server to send responses on
// requests: requests from the debug-client for the Server to handle
//
// Limitations: only ever accepts one connection
//

enum ServerStream {
	// The server is waiting for a Stream to be sent on the connection channel
	Waiting(mpsc::Receiver<TcpStream>),

	Connected(TcpStream),

	// The server has finished being used
	Disconnected,
}

pub struct Server {
	requests: mpsc::Receiver<Request>,
	stacks: Option<debug::CallStacks>,
	stream: ServerStream,
	_thread: JoinHandle<()>,
	should_catch_runtimes: bool,
}

struct ServerThread {
	requests: mpsc::Sender<Request>,
}

impl Server {
	pub fn connect(addr: &SocketAddr) -> std::io::Result<Server> {
		let stream = TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5))?;
		let (requests_sender, requests_receiver) = mpsc::channel();

		let server_thread = ServerThread {
			requests: requests_sender,
		};

		let cloned_stream = stream.try_clone().unwrap();
		let thread = thread::spawn(move || {
			server_thread.run(cloned_stream);
		});

		Ok(Server {
			requests: requests_receiver,
			stacks: None,
			stream: ServerStream::Connected(stream),
			_thread: thread,
			should_catch_runtimes: true,
		})
	}

	pub fn listen(addr: &SocketAddr) -> std::io::Result<Server> {
		let (connection_sender, connection_receiver) = mpsc::channel();
		let (requests_sender, requests_receiver) = mpsc::channel();

		let thread = ServerThread {
			requests: requests_sender,
		}.spawn_listener(TcpListener::bind(addr)?, connection_sender);

		Ok(Server {
			requests: requests_receiver,
			stacks: None,
			stream: ServerStream::Waiting(connection_receiver),
			_thread: thread,
			should_catch_runtimes: true,
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
					// If we're in the middle of executing an operand (like call), the offset might be between two instructions
					if instruction_offset > offset {
						reached_offset = true;
						break;
					}

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

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.value.tag == ValueTag::World && unsafe { value.value.data.id == 1 } {
			return true;
		}

		value.get("vars").is_ok()
	}

	fn value_to_variable(name: String, value: &Value) -> Variable {
		let mut variables = None;

		if List::is_list(value) || Self::is_object(value) {
			variables = Some(VariablesRef::Internal {
				tag: value.value.tag as u8,
				data: unsafe { value.value.data.id },
			})
		}

		match value.to_string() {
			Ok(value) => {
				Variable {
					name,
					kind: "TODO".to_owned(),
					value,
					variables,
				}
			},

			Err(Runtime { message }) => {
				Variable {
					name,
					kind: "TODO".to_owned(),
					value: format!("failed to read value: {:?}", message),
					variables,
				}
			}
		}
	}

	fn value_to_variables(value: &Value) -> Result<Vec<Variable>, Runtime> {
		// Lists are easy (TODO: asssoc)
		if List::is_list(value) {
			let list = List::from_value(value)?;
			let len = list.len();

			let mut variables = vec![
				Variable {
					name: "len".to_owned(),
					kind: "TODO".to_owned(),
					value: format!("{}", len),
					variables: None,
				}
			];
			
			for i in 1..=len {
				let value = list.get(i)?;
				variables.push(Self::value_to_variable(format!("[{}]", i), &value));
			}

			return Ok(variables);
		}

		if !Self::is_object(value) {
			return Ok(vec![]);
		}

		// Grab `value.vars`. We have a little hack for globals which use a special type.
		// TODO: vars is not always a list
		let vars = List::from_value(&unsafe {
			if value.value.tag == ValueTag::World && value.value.data.id == 1 {
				Value::new(
					ValueTag::GlobalVars,
					ValueData { id: 0 },
				)
			} else {
				value.get("vars")?
			}
		})?;

		let mut variables = vec![];
		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(name.as_str())?;
			variables.push(Self::value_to_variable(name, &value));
		}

		Ok(variables)
	}

	fn get_stack(&self, stack_id: u32) -> Option<&Vec<debug::StackFrame>> {
		let stack_id = stack_id as usize;
		let stacks = match self.stacks.as_ref() {
			Some(x) => x,
			None => return None,
		};

		if stack_id == 0 {
			return Some(&stacks.active);
		}

		stacks.suspended.get(stack_id - 1)
	}

	fn get_stack_base_frame_id(&self, stack_id: u32) -> u32 {
		let stack_id = stack_id as usize;
		let stacks = match self.stacks.as_ref() {
			Some(x) => x,
			None => return 0,
		};

		if stack_id == 0 {
			return 0;
		}

		let mut current_base = stacks.active.len();

		for frame in &stacks.suspended[..stack_id - 1] {
			current_base += frame.len();
		}

		current_base as u32
	}

	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let mut frame_index = frame_index as usize;
		let stacks = match self.stacks.as_ref() {
			Some(x) => x,
			None => return None,
		};

		if frame_index < stacks.active.len() {
			return Some(&stacks.active[frame_index]);
		}

		frame_index -= stacks.active.len();

		for frame in &stacks.suspended {
			if frame_index < frame.len() {
				return Some(&frame[frame_index]);
			}

			frame_index += frame.len();
		}

		None
	}

	fn get_args(&self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![];

				for (name, local) in &frame.args {
					let name = match name {
						Some(name) => String::from(name),
						None => "<unknown>".to_owned()
					};
					vars.push(Self::value_to_variable(name, &local));
				}

				vars
			}

			None => {
				eprintln!("Debug server tried to read arguments from invalid frame id: {}", frame_index);
				vec![]
			}
		}
	}

	fn get_locals(&self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![
					Self::value_to_variable(".".to_owned(), &frame.dot),
					Self::value_to_variable("src".to_owned(), &frame.src),
					Self::value_to_variable("usr".to_owned(), &frame.usr),
				];

				for (name, local) in &frame.locals {
					vars.push(Self::value_to_variable(String::from(name), &local));
				}

				vars
			}

			None => {
				eprintln!("Debug server tried to read locals from invalid frame id: {}", frame_index);
				vec![]
			}
		}
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::Disconnect => {
				unreachable!("Request::Disconnect should be handled by the network thread");
			}

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

			Request::SetCatchRuntimes(b) => {
				self.should_catch_runtimes = b
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

			Request::Stacks => {
				let stacks = match self.stacks.as_ref() {
					Some(stacks) => {
						let mut ret = vec![];
						ret.push(Stack {
							id: 0,
							name: stacks.active[0].proc.path.clone(),
						});

						for (idx, stack) in stacks.suspended.iter().enumerate() {
							ret.push(Stack {
								id: (idx + 1) as u32,
								name: stack[0].proc.path.clone(),
							});
						}

						ret
					}

					None => vec![],
				};

				self.send_or_disconnect(Response::Stacks{
					stacks
				});
			}

			Request::StackFrames {
				stack_id,
				start_frame,
				count,
			} => {
				self.send_or_disconnect(match self.get_stack(stack_id) {
					Some(stack) => {
						let frame_base = self.get_stack_base_frame_id(stack_id);
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
								id: frame_base + (i as u32),
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
						eprintln!("Debug server received StackFrames request when not paused");
						Response::StackFrames {
							frames: vec![],
							total_count: 0,
						}
					}
				});
			}

			Request::Scopes { frame_id } => {
				self.send_or_disconnect(match self.get_stack_frame(frame_id) {
					Some(frame) => {
						let mut arguments = None;

						if !frame.args.is_empty() {
							arguments = Some(VariablesRef::Arguments {
								frame: frame_id,
							});
						}

						// Never empty because we're putting ./src/usr in here
						let locals = Some(VariablesRef::Locals {
							frame: frame_id,
						});

						let globals_value = Value::globals();
						let globals = unsafe {
							VariablesRef::Internal {
								tag: globals_value.value.tag as u8,
								data: globals_value.value.data.id,
							}
						};

						Response::Scopes {
							arguments: arguments,
							locals: locals,
							globals: Some(globals),
						}
					}

					None => {
						eprintln!(
							"Debug server received Scopes request for invalid frame_id ({})",
							frame_id
						);
						Response::Scopes {
							arguments: None,
							locals: None,
							globals: None,
						}
					}
				});
			}

			Request::Variables { vars } => {
				let response = match vars {
					VariablesRef::Arguments { frame } => {
						Response::Variables {
							vars: self.get_args(frame)
						}
					}

					VariablesRef::Locals { frame } => {
						Response::Variables {
							vars: self.get_locals(frame)
						}
					}

					VariablesRef::Internal { tag, data } => {
						let value = unsafe {
							Value::from_raw(raw_types::values::Value {
								tag: std::mem::transmute(tag),
								data: ValueData { id: data },
							})
						};

						match Self::value_to_variables(&value) {
							Ok(vars) => Response::Variables { vars },

							Err(e) => {
								eprintln!("Debug server hit a runtime when processing Variables request: {:?}", e);
								Response::Variables { vars: vec![] }
							}
						}
					}
				};

				self.send_or_disconnect(response);
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

	pub fn check_connected(&mut self) -> bool {
		match &self.stream {
			ServerStream::Disconnected => false,
			ServerStream::Connected(_) => true,
			ServerStream::Waiting(receiver) => {
				if let Ok(stream) = receiver.try_recv() {
					self.stream = ServerStream::Connected(stream);
					true
				} else {
					false
				}
			}
		}
	}

	pub fn handle_breakpoint(
		&mut self,
		_ctx: *mut raw_types::procs::ExecutionContext,
		reason: BreakpointReason,
	) -> ContinueKind {
		// Ignore all breakpoints unless we're connected
		if !self.check_connected() {
			return ContinueKind::Continue;
		}

		if let BreakpointReason::Runtime(_) = reason {
			if !self.should_catch_runtimes {
				return ContinueKind::Continue;
			}
		}

		// Cache these now so nothing else has to fetch them
		// TODO: it'd be cool if all this data was fetched lazily
		self.stacks = Some(debug::CallStacks::new(&DMContext {}));

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
		// Don't do anything until we're connected
		if !self.check_connected() {
			return false;
		}

		let mut should_pause = false;

		while let Ok(request) = self.requests.try_recv() {
			should_pause = should_pause || self.handle_request(request);
		}

		should_pause
	}

	fn send_or_disconnect(&mut self, response: Response) {
		match self.stream {
			ServerStream::Connected(_) => {
				match self.send(response) {
					Ok(_) => {}
					Err(e) => {
						eprintln!("Debug server failed to send message: {}", e);
						self.disconnect();
					}
				}
			}

			ServerStream::Waiting(_) | ServerStream::Disconnected => panic!("Debug Server is not connected")
		}
	}

	fn disconnect(&mut self) {
		if let ServerStream::Connected(stream) = &mut self.stream {
			eprintln!("Debug server disconneting");
			let data = serde_json::to_vec(&Response::Disconnect).unwrap();
			let _ = stream.write_all(&data[..]);
			let _ = stream.write_all(&[0]);
			let _ = stream.flush();
			let _ = stream.shutdown(std::net::Shutdown::Both);
		}

		self.stream = ServerStream::Disconnected;
	}

	fn send(&mut self, response: Response) -> Result<(), Box<dyn std::error::Error>> {
		eprintln!("Debug response: {:?}", response);

		if let ServerStream::Connected(stream) = &mut self.stream {
			let data = serde_json::to_vec(&response)?;
			stream.write_all(&data[..])?;
			stream.write_all(&[0])?; // null-terminator
			stream.flush()?;
			return Ok(())
		}

		unreachable!();	
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect();
	}
}

impl ServerThread {
	fn spawn_listener(self, listener: TcpListener, connection_sender: mpsc::Sender<TcpStream>) -> JoinHandle<()> {
		thread::spawn(move || match listener.accept() {
			Ok((stream, _)) => {
				match connection_sender.send(stream.try_clone().unwrap())
				{
					Ok(_) => {}
					Err(e) => {
						eprintln!("Debug server thread failed to pass cloned TcpStream: {}", e);
						return;
					}
				}

				self.run(stream);
			}

			Err(e) => {
				eprintln!("Debug server failed to accept connection: {}", e);
			}
		})
	}

	// returns true if we should disconnect
	fn handle_request(&mut self, data: &[u8]) -> Result<bool, Box<dyn Error>> {
		let request = serde_json::from_slice::<Request>(data)?;

		eprintln!("Debug request: {:?}", request);

		if let Request::Disconnect = request {
			return Ok(true);
		}

		self.requests.send(request)?;
		Ok(false)
	}

	fn run(mut self, mut stream: TcpStream) {
		let mut buf = [0u8; 4096];
		let mut queued_data = vec![];

		// The incoming stream is JSON objects separated by null terminators.
		loop {
			match stream.read(&mut buf) {
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
					Ok(requested_disconnect) => {
						if requested_disconnect {
							eprintln!("Debug client disconnected");
							return;
						}
					}

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
