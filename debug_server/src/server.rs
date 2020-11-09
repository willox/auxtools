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
// We've got a couple channels going on between Server/ServerThread
// connection: a TcpStream sent from the ServerThread for the Server to send responses on
// requests: requests from the debug-client for the Server to handle
//
// Limitations: only ever accepts one connection & doesn't fully stop processing once that connection dies
//

pub struct Server {
	connection: mpsc::Receiver<TcpStream>,
	requests: mpsc::Receiver<Request>,
	stacks: Option<debug::CallStacks>,
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

	// TODO: Replace this with a check for `value.get("vars") success when multiple runtime catching is fixed
	fn is_object(value: &Value) -> bool {
		match value.value.tag {
			ValueTag::Turf
			| ValueTag::Obj
			| ValueTag::Mob
			| ValueTag::Area
			| ValueTag::Client
			| ValueTag::World
			| ValueTag::Datum
			| ValueTag::SaveFile => true,
			_ => false,
		}
	}

	fn value_to_variable(name: String, value: &Value) -> Result<Variable, Runtime> {
		let mut variables = None;
		let is_list = List::is_list(value);
		let has_vars = Self::is_object(value);

		if is_list || has_vars {
			variables = Some(VariablesRef::Internal {
				tag: value.value.tag as u8,
				data: unsafe { value.value.data.id },
			})
		}

		Ok(Variable {
			name,
			kind: "TODO".to_owned(),
			value: value.to_string()?,
			variables,
		})
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
				variables.push(Self::value_to_variable(format!("[{}]", i), &value)?);
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
			variables.push(Self::value_to_variable(name, &value)?);
		}

		Ok(variables)
	}

	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let mut frame_index = frame_index as usize;
		let stacks = self.stacks.as_ref().unwrap();

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
					vars.push(Self::value_to_variable(name, &local).unwrap());
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
					Self::value_to_variable(".".to_owned(), &frame.dot).unwrap(),
					Self::value_to_variable("src".to_owned(), &frame.src).unwrap(),
					Self::value_to_variable("usr".to_owned(), &frame.usr).unwrap(),
				];

				for (name, local) in &frame.locals {
					vars.push(Self::value_to_variable(String::from(name), &local).unwrap());
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
						eprintln!("Debug server received StackFrames request when not paused");
						Response::StackFrames {
							frames: vec![],
							total_count: 0,
						}
					}
				});
			}

			Request::Scopes { frame_id } => self.send_or_disconnect(match &self.stacks {
				Some(stacks) => match stacks.active.get(frame_id as usize) {
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
				},

				None => {
					eprintln!("Debug server received Scopes request when not paused");
					Response::Scopes {
						arguments: None,
						locals: None,
						globals: None,
					}
				}
			}),

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

	pub fn handle_breakpoint(
		&mut self,
		_ctx: *mut raw_types::procs::ExecutionContext,
		reason: BreakpointReason,
	) -> ContinueKind {
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
			Ok(_) => {}
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
		thread::spawn(move || match self.listener.accept() {
			Ok((stream, _)) => {
				self.stream = Some(stream);
				self.run();
			}

			Err(e) => {
				println!("Debug server failed to accept connection {}", e);
			}
		})
	}

	fn handle_request(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
		let request = serde_json::from_slice::<Request>(data)?;
		self.requests.send(request)?;
		Ok(())
	}

	fn run(mut self) {
		match self
			.connection
			.send(self.stream.as_mut().unwrap().try_clone().unwrap())
		{
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
