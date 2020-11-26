use super::instruction_hooking::{hook_instruction, unhook_instruction};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::{cell::RefCell, error::Error};
use std::{
	collections::HashMap,
	net::{SocketAddr, TcpListener, TcpStream},
	thread::JoinHandle,
};

use super::server_types::*;
use auxtools::raw_types::values::{ValueData, ValueTag};
use auxtools::*;

#[derive(Clone, Hash, Eq, PartialEq)]
enum Variables {
	Arguments {
		frame: u32,
	},
	Locals {
		frame: u32,
	},
	ObjectVars {
		tag: u8,
		data: u32,
	},
	ListContents {
		tag: u8,
		data: u32,
	},
	ListPair {
		key_tag: u8,
		key_data: u32,
		value_tag: u8,
		value_data: u32,
	},
}

struct State {
	stacks: debug::CallStacks,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>,
}

impl State {
	fn new() -> Self {
		Self {
			stacks: debug::CallStacks::new(&DMContext {}),
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new()),
		}
	}

	fn get_ref(&self, vars: Variables) -> VariablesRef {
		let mut variables_to_refs = self.variables_to_refs.borrow_mut();
		let mut variables = self.variables.borrow_mut();
		(*variables_to_refs.entry(vars.clone()).or_insert_with(|| {
			let reference = VariablesRef(variables.len() as i32 + 1);
			variables.push(vars);
			reference
		}))
		.clone()
	}

	fn get_variables(&self, reference: VariablesRef) -> Option<Variables> {
		let variables = self.variables.borrow();
		variables
			.get(reference.0 as usize - 1)
			.map(|x| (*x).clone())
	}
}

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
	stream: ServerStream,
	_thread: JoinHandle<()>,
	should_catch_runtimes: bool,
	state: Option<State>,
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
			stream: ServerStream::Connected(stream),
			_thread: thread,
			should_catch_runtimes: true,
			state: None,
		})
	}

	pub fn listen(addr: &SocketAddr) -> std::io::Result<Server> {
		let (connection_sender, connection_receiver) = mpsc::channel();
		let (requests_sender, requests_receiver) = mpsc::channel();

		let thread = ServerThread {
			requests: requests_sender,
		}
		.spawn_listener(TcpListener::bind(addr)?, connection_sender);

		Ok(Server {
			requests: requests_receiver,
			stream: ServerStream::Waiting(connection_receiver),
			_thread: thread,
			should_catch_runtimes: true,
			state: None,
		})
	}

	fn get_line_number(&self, proc: ProcRef, offset: u32) -> Option<u32> {
		match auxtools::Proc::find_override(proc.path, proc.override_id) {
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

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		match auxtools::Proc::find_override(proc.path, proc.override_id) {
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

				return offset;
			}

			None => {
				return None;
			}
		}
	}

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.value.tag == ValueTag::World && unsafe { value.value.data.id == 1 } {
			return true;
		}

		value.get("vars").is_ok()
	}

	fn value_to_variable(&self, name: String, value: &Value) -> Variable {
		let mut variables = None;
		let state = self.state.as_ref().unwrap();

		if List::is_list(value) {
			variables = Some(state.get_ref(Variables::ListContents {
				tag: value.value.tag as u8,
				data: unsafe { value.value.data.id },
			}));

			// Early return for lists so we can include their length in the value
			let stringified = match List::from_value(value) {
				Ok(list) => format!("/list {{len = {}}}", list.len()),
				Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message),
			};

			return Variable {
				name,
				value: stringified,
				variables,
			};
		} else if Self::is_object(value) {
			variables = Some(state.get_ref(Variables::ObjectVars {
				tag: value.value.tag as u8,
				data: unsafe { value.value.data.id },
			}));
		}

		let stringified = match value.to_string() {
			Ok(value) => value,
			Err(Runtime { message }) => format!("failed to stringify value: {:?}", message),
		};

		Variable {
			name,
			value: stringified,
			variables,
		}
	}

	fn list_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();

		let mut variables = vec![];

		for i in 1..=len {
			let key = list.get(i)?;

			if let Ok(value) = list.get(&key) {
				if value.value.tag != raw_types::values::ValueTag::Null {
					// assoc entry
					variables.push(Variable {
						name: format!("[{}]", i),
						value: format!("{} = {}", key.to_string()?, value.to_string()?), // TODO: prettify these prints?
						variables: Some(state.get_ref(unsafe {
							Variables::ListPair {
								key_tag: key.value.tag as u8,
								key_data: key.value.data.id,
								value_tag: value.value.tag as u8,
								value_data: value.value.data.id,
							}
						})),
					});
					continue;
				}
			}

			// non-assoc entry
			variables.push(self.value_to_variable(format!("[{}]", i), &key));
		}

		return Ok(variables);
	}

	fn object_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		// Grab `value.vars`. We have a little hack for globals which use a special type.
		let vars = List::from_value(&unsafe {
			if value.value.tag == ValueTag::World && value.value.data.id == 1 {
				Value::new(ValueTag::GlobalVars, ValueData { id: 0 })
			} else {
				value.get("vars")?
			}
		})?;

		let mut variables = vec![];
		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(name.as_str())?;
			variables.push(self.value_to_variable(name, &value));
		}

		Ok(variables)
	}

	fn get_stack(&self, stack_id: u32) -> Option<&Vec<debug::StackFrame>> {
		let stack_id = stack_id as usize;
		let stacks = match &self.state {
			Some(state) => &state.stacks,
			None => return None,
		};

		if stack_id == 0 {
			return Some(&stacks.active);
		}

		stacks.suspended.get(stack_id - 1)
	}

	fn get_stack_base_frame_id(&self, stack_id: u32) -> u32 {
		let stack_id = stack_id as usize;
		let stacks = match &self.state {
			Some(state) => &state.stacks,
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
		let stacks = match &self.state {
			Some(state) => &state.stacks,
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

			frame_index -= frame.len();
		}

		None
	}

	fn get_args(&mut self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![];

				for (name, local) in &frame.args {
					let name = match name {
						Some(name) => String::from(name),
						None => "<unknown>".to_owned(),
					};
					vars.push(self.value_to_variable(name, &local));
				}

				vars
			}

			None => {
				self.notify(format!(
					"tried to read arguments from invalid frame id: {}",
					frame_index
				));
				vec![]
			}
		}
	}

	fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![
					self.value_to_variable(".".to_owned(), &frame.dot),
					self.value_to_variable("src".to_owned(), &frame.src),
					self.value_to_variable("usr".to_owned(), &frame.usr),
				];

				for (name, local) in &frame.locals {
					vars.push(self.value_to_variable(String::from(name), &local));
				}

				vars
			}

			None => {
				self.notify(format!(
					"tried to read locals from invalid frame id: {}",
					frame_index
				));
				vec![]
			}
		}
	}

	fn handle_breakpoint_set(&mut self, instruction: InstructionRef) {
		let line = self.get_line_number(instruction.proc.clone(), instruction.offset);

		match auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
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

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef) {
		match auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
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

	fn handle_stacks(&mut self) {
		let stacks = match &self.state {
			Some(state) => {
				let mut ret = vec![];
				ret.push(Stack {
					id: 0,
					name: state.stacks.active[0].proc.path.clone(),
				});

				for (idx, stack) in state.stacks.suspended.iter().enumerate() {
					ret.push(Stack {
						id: (idx + 1) as u32,
						name: stack[0].proc.path.clone(),
					});
				}

				ret
			}

			None => vec![],
		};

		self.send_or_disconnect(Response::Stacks { stacks });
	}

	fn handle_stack_frames(&mut self, stack_id: u32, start_frame: Option<u32>, count: Option<u32>) {
		let response = match self.get_stack(stack_id) {
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
						override_id: stack[i].proc.override_id(),
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
				self.notify("received StackFrames request when not paused");
				Response::StackFrames {
					frames: vec![],
					total_count: 0,
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_scopes(&mut self, frame_id: u32) {
		let response = match self.get_stack_frame(frame_id) {
			Some(frame) => {
				let state = self.state.as_ref().unwrap();
				let mut arguments = None;

				if !frame.args.is_empty() {
					arguments = Some(Variables::Arguments { frame: frame_id });
				}

				// Never empty because we're putting ./src/usr in here
				let locals = Some(Variables::Locals { frame: frame_id });

				let globals_value = Value::globals();
				let globals = unsafe {
					Variables::ObjectVars {
						tag: globals_value.value.tag as u8,
						data: globals_value.value.data.id,
					}
				};

				Response::Scopes {
					arguments: arguments.map(|x| state.get_ref(x)),
					locals: locals.map(|x| state.get_ref(x)),
					globals: Some(state.get_ref(globals)),
				}
			}

			None => {
				self.notify(format!(
					"Debug server received Scopes request for invalid frame_id ({})",
					frame_id
				));
				Response::Scopes {
					arguments: None,
					locals: None,
					globals: None,
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_variables(&mut self, vars: VariablesRef) {
		let response =
			match &self.state {
				Some(state) => {
					match state.get_variables(vars) {
						Some(vars) => match vars {
							Variables::Arguments { frame } => Response::Variables {
								vars: self.get_args(frame),
							},
							Variables::Locals { frame } => Response::Variables {
								vars: self.get_locals(frame),
							},
							Variables::ObjectVars { tag, data } => {
								let value = unsafe {
									Value::from_raw(raw_types::values::Value {
										tag: std::mem::transmute(tag),
										data: ValueData { id: data },
									})
								};

								match self.object_to_variables(&value) {
									Ok(vars) => Response::Variables { vars },

									Err(e) => {
										self.notify(format!("runtime occured while processing Variables request: {:?}", e));
										Response::Variables { vars: vec![] }
									}
								}
							}
							Variables::ListContents { tag, data } => {
								let value = unsafe {
									Value::from_raw(raw_types::values::Value {
										tag: std::mem::transmute(tag),
										data: ValueData { id: data },
									})
								};

								match self.list_to_variables(&value) {
									Ok(vars) => Response::Variables { vars },

									Err(e) => {
										self.notify(format!("runtime occured while processing Variables request: {:?}", e));
										Response::Variables { vars: vec![] }
									}
								}
							}

							Variables::ListPair {
								key_tag,
								key_data,
								value_tag,
								value_data,
							} => {
								let key = unsafe {
									Value::from_raw(raw_types::values::Value {
										tag: std::mem::transmute(key_tag),
										data: ValueData { id: key_data },
									})
								};

								let value = unsafe {
									Value::from_raw(raw_types::values::Value {
										tag: std::mem::transmute(value_tag),
										data: ValueData { id: value_data },
									})
								};

								Response::Variables {
									vars: vec![
										self.value_to_variable("key".to_owned(), &key),
										self.value_to_variable("value".to_owned(), &value),
									],
								}
							}
						},

						None => {
							self.notify("received unknown VariableRef in Variables request");
							Response::Variables { vars: vec![] }
						}
					}
				}

				None => {
					self.notify("recevied Variables request while not paused");
					Response::Variables { vars: vec![] }
				}
			};

		self.send_or_disconnect(response);
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { should_catch } => self.should_catch_runtimes = should_catch,
			Request::BreakpointSet { instruction } => self.handle_breakpoint_set(instruction),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction),
			Request::Stacks => self.handle_stacks(),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars } => self.handle_variables(vars),

			Request::StackFrames {
				stack_id,
				start_frame,
				count,
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::LineNumber { proc, offset } => {
				self.send_or_disconnect(Response::LineNumber {
					line: self.get_line_number(proc, offset),
				});
			}

			Request::Offset { proc, line } => {
				self.send_or_disconnect(Response::Offset {
					offset: self.get_offset(proc, line),
				});
			}

			Request::Continue { .. } => {
				self.send_or_disconnect(Response::Ack);
			}

			Request::Pause => {
				self.send_or_disconnect(Response::Ack);
				return true;
			}
		}

		false
	}

	fn check_connected(&mut self) -> bool {
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

	pub fn wait_for_connection(&mut self) {
		match &self.stream {
			ServerStream::Waiting(receiver) => {
				if let Ok(stream) = receiver.recv() {
					self.stream = ServerStream::Connected(stream);
				}
			}

			_ => (),
		}
	}

	fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		eprintln!("Debug Server: {:?}", message);

		if !self.check_connected() {
			return;
		}

		self.send_or_disconnect(Response::Notification { message });
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

		self.notify(format!("Pausing execution (reason: {:?})", reason));

		self.state = Some(State::new());

		self.send_or_disconnect(Response::BreakpointHit { reason });

		while let Ok(request) = self.requests.recv() {
			// Hijack and handle any Continue requests
			if let Request::Continue { kind } = request {
				self.send_or_disconnect(Response::Ack);
				self.state = None;
				return kind;
			}

			// if we get a pause request here we can ignore it
			self.handle_request(request);
		}

		// Client disappeared?
		self.state = None;
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
			ServerStream::Connected(_) => match self.send(response) {
				Ok(_) => {}
				Err(e) => {
					eprintln!("Debug server failed to send message: {}", e);
					self.disconnect();
				}
			},

			ServerStream::Waiting(_) | ServerStream::Disconnected => {
				unreachable!("Debug Server is not connected")
			}
		}
	}

	fn disconnect(&mut self) {
		if let ServerStream::Connected(stream) = &mut self.stream {
			eprintln!("Debug server disconnecting");
			let data = bincode::serialize(&Response::Disconnect).unwrap();
			let _ = stream.write_all(&(data.len() as u32).to_le_bytes());
			let _ = stream.write_all(&data[..]);
			let _ = stream.flush();
			let _ = stream.shutdown(std::net::Shutdown::Both);
		}

		self.stream = ServerStream::Disconnected;
	}

	fn send(&mut self, response: Response) -> Result<(), Box<dyn std::error::Error>> {
		if let ServerStream::Connected(stream) = &mut self.stream {
			let data = bincode::serialize(&response)?;
			stream.write_all(&(data.len() as u32).to_le_bytes())?;
			stream.write_all(&data[..])?;
			stream.flush()?;
			return Ok(());
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
	fn spawn_listener(
		self,
		listener: TcpListener,
		connection_sender: mpsc::Sender<TcpStream>,
	) -> JoinHandle<()> {
		thread::spawn(move || match listener.accept() {
			Ok((stream, _)) => {
				match connection_sender.send(stream.try_clone().unwrap()) {
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
		let request = bincode::deserialize::<Request>(data)?;

		if let Request::Disconnect = request {
			return Ok(true);
		}

		self.requests.send(request)?;
		Ok(false)
	}

	fn run(mut self, mut stream: TcpStream) {
		let mut buf = vec![];

		// The incoming stream is a u32 followed by a bincode-encoded Request.
		loop {
			let mut len_bytes = [0u8; 4];
			let len = match stream.read_exact(&mut len_bytes) {
				Ok(_) => u32::from_le_bytes(len_bytes),

				Err(e) => {
					eprintln!("Debug server thread read error: {}", e);
					break;
				}
			};

			buf.resize(len as usize, 0);
			match stream.read_exact(&mut buf) {
				Ok(_) => (),

				Err(e) => {
					eprintln!("Debug server thread read error: {}", e);
					break;
				}
			};

			match self.handle_request(&buf[..]) {
				Ok(requested_disconnect) => {
					if requested_disconnect {
						eprintln!("Debug client disconnected");
						break;
					}
				}

				Err(e) => {
					eprintln!("Debug server thread failed to handle request: {}", e);
					break;
				}
			}
		}

		eprintln!("Debug server thread finished");
	}
}
