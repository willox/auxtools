use std::{
	cell::RefCell,
	collections::HashMap,
	error::Error,
	io::{Read, Write},
	net::{SocketAddr, TcpListener, TcpStream},
	sync::mpsc,
	thread,
	thread::JoinHandle
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use clap::{Arg, Command};
use instruction_hooking::disassemble_env;

use super::{
	instruction_hooking::{get_hooked_offsets, hook_instruction, unhook_instruction},
	server_types::*
};
use crate::mem_profiler;

#[derive(Clone, Hash, PartialEq, Eq)]
enum Variables {
	Arguments { frame: u32 },
	Locals { frame: u32 },
	ObjectVars(Value),
	ListContents(Value),
	ListPair { key: Value, value: Value }
}

struct State {
	stacks: debug::CallStacks,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>
}

impl State {
	fn new() -> Self {
		Self {
			stacks: debug::CallStacks::new(),
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new())
		}
	}

	fn invalidate_stacks(&mut self) {
		self.stacks = debug::CallStacks::new();
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
		variables.get(reference.0 as usize - 1).map(|x| (*x).clone())
	}
}

// Server = main-thread code
// ServerThread = networking-thread code
//
// We've got a couple of channels going on between Server/ServerThread
// connection: a TcpStream sent from the ServerThread for the Server to send
// responses on requests: requests from the debug-client for the Server to
// handle
//
// Limitations: only ever accepts one connection
//

enum ServerStream {
	// The server is waiting for a Stream to be sent on the connection channel
	Waiting(mpsc::Receiver<TcpStream>),

	Connected(TcpStream),

	// The server has finished being used
	Disconnected
}

pub struct Server {
	requests: mpsc::Receiver<Request>,
	stream: ServerStream,
	_thread: JoinHandle<()>,
	should_catch_runtimes: bool,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	conditional_breakpoints: HashMap<(raw_types::procs::ProcId, u16), String>,
	app: Command<'static>
}

struct ServerThread {
	requests: mpsc::Sender<Request>
}

impl Server {
	pub fn setup_app() -> Command<'static> {
		Command::new("Auxtools Debug Server")
			.version("2.2.2")
			.subcommand_required(true)
			.no_binary_name(true)
			.color(clap::ColorChoice::Never)
			.disable_version_flag(true)
			.disable_help_flag(true)
			.override_usage("#<SUBCOMMAND>")
			.subcommand(
				Command::new("disassemble")
					.alias("dis")
					.about("Disassembles a proc and displays its bytecode in an assembly-like format")
					.after_help("If no parameters are provided, the proc executing in the currently debugged stack frame will be disassembled")
					.arg(
						Arg::new("proc")
							.help("Path of the proc to disassemble (e.g. /proc/do_stuff)")
							.takes_value(true)
					)
					.arg(
						Arg::new("id")
							.help("Id of the proc to disassemble (for when multiple procs are defined with the same path)")
							.takes_value(true)
					)
			)
			.subcommand(
				Command::new("guest_override")
					.about("Override the CKey used by guest connections")
					.arg(Arg::new("ckey").takes_value(true))
			)
			.subcommand(
				Command::new("mem_profiler")
					.about("Memory profiler")
					.subcommand(
						Command::new("begin")
							.about("Begins memory profiling. Output goes to the specified file path")
							.arg(Arg::new("path").help("Where to output memory profiler results").takes_value(true))
					)
					.subcommand(Command::new("end").about("Finishes current memory profiler."))
			)
	}

	pub fn connect(addr: &SocketAddr) -> std::io::Result<Server> {
		let stream = TcpStream::connect_timeout(addr, std::time::Duration::from_secs(5))?;
		let (requests_sender, requests_receiver) = mpsc::channel();

		let server_thread = ServerThread { requests: requests_sender };

		let cloned_stream = stream.try_clone().unwrap();
		let thread = thread::spawn(move || {
			server_thread.run(cloned_stream);
		});

		let mut server = Server {
			requests: requests_receiver,
			stream: ServerStream::Connected(stream),
			_thread: thread,
			should_catch_runtimes: true,
			state: None,
			in_eval: false,
			eval_error: None,
			conditional_breakpoints: HashMap::new(),
			app: Self::setup_app()
		};

		server.process_until_configured();
		Ok(server)
	}

	pub fn listen(addr: &SocketAddr) -> std::io::Result<Server> {
		let (connection_sender, connection_receiver) = mpsc::channel();
		let (requests_sender, requests_receiver) = mpsc::channel();

		let thread = ServerThread { requests: requests_sender }.spawn_listener(TcpListener::bind(addr)?, connection_sender);

		Ok(Server {
			requests: requests_receiver,
			stream: ServerStream::Waiting(connection_receiver),
			_thread: thread,
			should_catch_runtimes: true,
			state: None,
			in_eval: false,
			eval_error: None,
			conditional_breakpoints: HashMap::new(),
			app: Self::setup_app()
		})
	}

	pub const fn is_in_eval(&self) -> bool {
		self.in_eval
	}

	pub fn set_eval_error(&mut self, err: String) {
		self.eval_error = Some(err);
	}

	fn get_line_number(&self, proc: ProcRef, offset: u32) -> Option<u32> {
		match auxtools::Proc::find_override(proc.path, proc.override_id) {
			Some(proc) => {
				let mut current_line_number = None;
				let mut reached_offset = false;

				let bytecode = unsafe { proc.bytecode() };

				let mut env = disassemble_env::DisassembleEnv;
				let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

				for node in nodes {
					if let dmasm::Node::Instruction(ins, debug) = node {
						if debug.offset > offset {
							reached_offset = true;
							break;
						}

						if let dmasm::Instruction::DbgLine(line) = ins {
							current_line_number = Some(line);
						}

						if debug.offset == offset {
							reached_offset = true;
							break;
						}
					}
				}

				if reached_offset {
					current_line_number
				} else {
					None
				}
			}

			None => None
		}
	}

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		let proc = auxtools::Proc::find_override(proc.path, proc.override_id)?;
		let mut offset = None;
		let mut at_offset = false;

		let bytecode = unsafe { proc.bytecode() };

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if at_offset {
					offset = Some(debug.offset);
					break;
				}

				if let dmasm::Instruction::DbgLine(current_line) = ins {
					if current_line == line {
						at_offset = true;
					}
				}
			}
		}

		offset
	}

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
			return true;
		}

		value.get(byond_string!("vars")).is_ok()
	}

	fn stringify(value: &Value) -> String {
		if List::is_list(value) {
			match List::from_value(value) {
				Ok(list) => format!("/list {{len = {}}}", list.len()),
				Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message)
			}
		} else {
			match value.to_string() {
				Ok(v) if v.is_empty() => value.raw.to_string(),
				Ok(value) => value,
				Err(Runtime { message }) => {
					format!("{} -- stringify error: {:?}", value.raw, message)
				}
			}
		}
	}

	fn value_to_variable(&self, name: String, value: &Value) -> Variable {
		let stringified = Self::stringify(value);
		let variables = self.value_to_variables_ref(value);

		Variable {
			name,
			value: stringified,
			variables
		}
	}

	fn value_to_variables_ref(&self, value: &Value) -> Option<VariablesRef> {
		match self.state.as_ref() {
			Some(state) if List::is_list(value) => Some(state.get_ref(Variables::ListContents(value.clone()))),

			Some(state) if Self::is_object(value) => Some(state.get_ref(Variables::ObjectVars(value.clone()))),

			_ => None
		}
	}

	fn list_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();

		let mut variables = vec![];

		for i in 1..=len {
			let key = list.get(i)?;

			// assoc entry
			if key.raw.tag != raw_types::values::ValueTag::Number {
				if let Ok(value) = list.get(&key) {
					if value.raw.tag != raw_types::values::ValueTag::Null {
						variables.push(Variable {
							name: format!("[{}]", i),
							value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
							variables: Some(state.get_ref(Variables::ListPair { key, value }))
						});

						continue;
					}
				}
			}

			// non-assoc entry
			variables.push(self.value_to_variable(format!("[{}]", i), &key));
		}

		Ok(variables)
	}

	fn object_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		// Grab `value.vars`. We have a little hack for globals which use a special
		// type.
		let vars = List::from_value(&unsafe {
			if value.raw.tag == ValueTag::World && value.raw.data.id == 1 {
				Value::new(ValueTag::GlobalVars, ValueData { id: 0 })
			} else {
				value.get(byond_string!("vars"))?
			}
		})?;

		let mut variables = vec![];
		let mut top_variables = vec![]; // These fields get displayed on top of all others

		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(StringRef::new(name.as_str())?)?;
			let variable = self.value_to_variable(name, &value);
			if variable.name == "type" {
				top_variables.push(variable);
			} else {
				variables.push(variable);
			}
		}

		// top_variables.sort_by_key(|a| a.name.to_lowercase());
		variables.sort_by_key(|a| a.name.to_lowercase());
		top_variables.append(&mut variables);

		Ok(top_variables)
	}

	fn get_stack(&self, stack_id: u32) -> Option<&Vec<debug::StackFrame>> {
		let stack_id = stack_id as usize;
		let stacks = match &self.state {
			Some(state) => &state.stacks,
			None => return None
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
			None => return 0
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
			None => return None
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
				let mut vars = vec![
					self.value_to_variable("src".to_owned(), &frame.src),
					self.value_to_variable("usr".to_owned(), &frame.usr),
				];

				let mut unnamed_count = 0;
				for (name, value) in &frame.args {
					let name = match name {
						Some(name) => String::from(name),
						None => {
							unnamed_count += 1;
							format!("undefined argument #{}", unnamed_count)
						}
					};
					vars.push(self.value_to_variable(name, value));
				}

				vars
			}

			None => {
				self.notify(format!("tried to read arguments from invalid frame id: {}", frame_index));
				vec![]
			}
		}
	}

	fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![self.value_to_variable(".".to_owned(), &frame.dot)];

				for (name, local) in &frame.locals {
					vars.push(self.value_to_variable(String::from(name), local));
				}

				vars
			}

			None => {
				self.notify(format!("tried to read locals from invalid frame id: {}", frame_index));
				vec![]
			}
		}
	}

	fn handle_breakpoint_set(&mut self, instruction: InstructionRef, condition: Option<String>) {
		let line = self.get_line_number(instruction.proc.clone(), instruction.offset);

		let proc = match auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
			Some(proc) => proc,
			None => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
				return;
			}
		};

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				if let Some(condition) = condition {
					self.conditional_breakpoints.insert((proc.id, instruction.offset as u16), condition);
				}

				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line }
				});
			}

			Err(_) => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef) {
		let proc = match auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id) {
			Some(proc) => proc,
			None => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
				return;
			}
		};

		self.conditional_breakpoints.remove(&(proc.id, instruction.offset as u16));

		match unhook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				self.send_or_disconnect(Response::BreakpointUnset { success: true });
			}

			Err(_) => {
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
					name: state.stacks.active[0].proc.path.clone()
				});

				for (idx, stack) in state.stacks.suspended.iter().enumerate() {
					ret.push(Stack {
						id: (idx + 1) as u32,
						name: stack[0].proc.path.clone()
					});
				}

				ret
			}

			None => vec![]
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
						override_id: stack[i].proc.override_id()
					};

					frames.push(StackFrame {
						id: frame_base + (i as u32),
						instruction: InstructionRef {
							proc: proc_ref.clone(),
							offset: stack[i].offset as u32
						},
						line: self.get_line_number(proc_ref, stack[i].offset as u32)
					});
				}

				Response::StackFrames {
					frames,
					total_count: stack.len() as u32
				}
			}

			None => {
				self.notify("received StackFrames request when not paused");
				Response::StackFrames {
					frames: vec![],
					total_count: 0
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_scopes(&mut self, frame_id: u32) {
		if self.state.is_none() {
			let response = Response::Scopes {
				arguments: None,
				locals: None,
				globals: None
			};

			self.send_or_disconnect(response);
			return;
		}

		let state = self.state.as_ref().unwrap();

		let arguments = Variables::Arguments { frame: frame_id };
		let locals = Variables::Locals { frame: frame_id };

		let globals = Variables::ObjectVars(Value::GLOBAL);

		let response = Response::Scopes {
			arguments: Some(state.get_ref(arguments)),
			locals: Some(state.get_ref(locals)),
			globals: Some(state.get_ref(globals))
		};

		self.send_or_disconnect(response);
	}

	fn handle_variables(&mut self, vars: VariablesRef) {
		let response = match &self.state {
			Some(state) => match state.get_variables(vars) {
				Some(vars) => match vars {
					Variables::Arguments { frame } => Response::Variables { vars: self.get_args(frame) },
					Variables::Locals { frame } => Response::Variables {
						vars: self.get_locals(frame)
					},
					Variables::ObjectVars(value) => match self.object_to_variables(&value) {
						Ok(vars) => Response::Variables { vars },

						Err(e) => {
							self.notify(format!("runtime occured while processing Variables request: {:?}", e));
							Response::Variables { vars: vec![] }
						}
					},
					Variables::ListContents(value) => match self.list_to_variables(&value) {
						Ok(vars) => Response::Variables { vars },

						Err(e) => {
							self.notify(format!("runtime occured while processing Variables request: {:?}", e));
							Response::Variables { vars: vec![] }
						}
					},

					Variables::ListPair { key, value } => Response::Variables {
						vars: vec![
							self.value_to_variable("key".to_owned(), &key),
							self.value_to_variable("value".to_owned(), &value),
						]
					}
				},

				None => {
					self.notify("received unknown VariableRef in Variables request");
					Response::Variables { vars: vec![] }
				}
			},

			None => {
				self.notify("recevied Variables request while not paused");
				Response::Variables { vars: vec![] }
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		// How many matches variables can you spot? It could be better...
		let response = match self.app.try_get_matches_from_mut(command.split_ascii_whitespace()) {
			Ok(matches) => {
				match matches.subcommand() {
					Some(("disassemble", matches)) => {
						if let Some(proc) = matches.value_of("proc") {
							// Default id to 0 in the worst way possible
							let id = matches.value_of("id").and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);

							self.handle_disassemble(proc, id)
						} else if let Some(frame_id) = frame_id {
							if let Some(frame) = self.get_stack_frame(frame_id) {
								let proc = frame.proc.path.clone();
								let id = frame.proc.override_id();
								self.handle_disassemble(&proc, id)
							} else {
								"couldn't find stack frame (is execution not paused?)".to_owned()
							}
						} else {
							"no execution frame selected".to_owned()
						}
					}

					Some(("guest_override", matches)) => match matches.value_of("ckey") {
						Some(ckey) => match crate::ckey_override::override_guest_ckey(ckey) {
							Ok(()) => "Success".to_owned(),

							Err(e) => {
								format!("Failed: {:?}", e)
							}
						},

						None => "no ckey provided".to_owned()
					},

					Some(("mem_profiler", matches)) => match matches.subcommand() {
						Some(("begin", matches)) => match matches.value_of("path") {
							Some(path) => mem_profiler::begin(path)
								.map(|_| "Memory profiler enabled".to_owned())
								.unwrap_or_else(|e| format!("Failed: {}", e)),

							None => "no path provided".to_owned()
						},

						Some(("end", _)) => {
							mem_profiler::end();
							"Memory profiler disabled".to_owned()
						}

						_ => "unknown memory profiler sub-command".to_owned()
					},

					_ => "unknown command".to_owned()
				}
			}
			Err(e) => e.to_string()
		};

		response
	}

	fn eval_expr(&mut self, frame_id: Option<u32>, command: &str) -> Option<Value> {
		enum ArgType {
			Dot,
			Usr,
			Src,
			Arg(u32),
			Local(u32)
		}

		let (ctx, instance, args) = match frame_id {
			// Global context
			None => (std::ptr::null_mut(), std::ptr::null_mut(), vec![]),

			Some(frame_id) => {
				let frame = match self.get_stack_frame(frame_id) {
					Some(x) => x,
					None => {
						self.notify(format!("tried to evaluate expression with invalid frame id: {}", frame_id));
						return None;
					}
				};

				let mut args = vec![
					(".".to_owned(), frame.dot.clone(), ArgType::Dot),
					("usr".to_owned(), frame.usr.clone(), ArgType::Usr),
					("src".to_owned(), frame.src.clone(), ArgType::Src),
				];

				for (idx, arg) in frame.args.iter().enumerate() {
					if let Some(name) = &arg.0 {
						args.push((name.into(), arg.1.clone(), ArgType::Arg(idx as u32)));
					}
				}

				for (idx, local) in frame.locals.iter().enumerate() {
					args.push(((&local.0).into(), local.1.clone(), ArgType::Local(idx as u32)));
				}

				(frame.context, frame.instance, args)
			}
		};

		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		let expr = match dmasm::compiler::compile_expr(command, &arg_names) {
			Ok(expr) => expr,
			Err(err) => {
				self.notify(format!("{}", err));
				return None;
			}
		};

		let assembly = match dmasm::assembler::assemble(&expr, &mut crate::assemble_env::AssembleEnv) {
			Ok(assembly) => assembly,
			Err(err) => {
				self.notify(format!("expression {} failed to assemble: {:#?}", command, err));
				return None;
			}
		};

		let proc = match Proc::find("/proc/auxtools_expr_stub") {
			Some(proc) => proc,
			None => {
				self.notify("Couldn't find /proc/auxtools_expr_stub! DM evaluation not available.");
				return None;
			}
		};

		proc.set_bytecode(assembly);

		self.in_eval = true;
		self.eval_error = None;

		let result = match proc.call(&arg_values) {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					// The rest are the potentially mutated parameters. We need to commit them to
					// the function that called us. TODO: This sucks, obviously.
					let len = list.len();
					for i in 2..=len {
						let value = list.get(i).unwrap();
						let slot = &args[i as usize - 2].2;

						unsafe {
							match slot {
								ArgType::Dot => {
									let _ = Value::from_raw_owned((*ctx).dot);
									(*ctx).dot = value.raw;
								}
								ArgType::Usr => {
									let _ = Value::from_raw_owned((*instance).usr);
									(*instance).usr = value.raw;
								}
								ArgType::Src => {
									let _ = Value::from_raw_owned((*instance).src);
									(*instance).src = value.raw;
								}
								ArgType::Arg(idx) => {
									let args = (*instance).args();
									let arg = args.add(*idx as usize);
									let _ = Value::from_raw_owned(*arg);
									(*arg) = value.raw;
								}
								ArgType::Local(idx) => {
									let locals = (*ctx).locals;
									let local = locals.add(*idx as usize);
									let _ = Value::from_raw_owned(*local);
									(*local) = value.raw;
								}
							}
						}

						std::mem::forget(value);
					}

					Some(list.get(1).unwrap())
				} else {
					None
				}
			}

			Err(_) => {
				self.notify(format!("Value::call failed when evaluating expression {}", command));
				None
			}
		};

		self.in_eval = false;

		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime occured when executing expression: {}", err));
		}

		result
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str, context: Option<String>) {
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.send_or_disconnect(Response::Eval(EvalResponse {
				value: response,
				variables: None
			}));
			return;
		}

		match self.eval_expr(frame_id, command) {
			Some(result) => {
				let variables = match context {
					Some(str) if str == "repl" => None,
					_ => self.value_to_variables_ref(&result)
				};

				self.send_or_disconnect(Response::Eval(EvalResponse {
					value: Self::stringify(&result),
					variables
				}));
			}

			None => {
				self.send_or_disconnect(Response::Eval(EvalResponse {
					value: "<no value>".to_owned(),
					variables: None
				}));
			}
		}
	}

	fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match auxtools::Proc::find_override(path, id) {
			Some(proc) => {
				// Make sure to temporarily remove all breakpoints in this proc
				let breaks = get_hooked_offsets(&proc);

				for offset in &breaks {
					unhook_instruction(&proc, *offset).unwrap();
				}

				let bytecode = unsafe { proc.bytecode() };

				let mut env = crate::DisassembleEnv;
				let (nodes, error) = dmasm::disassembler::disassemble(bytecode, &mut env);
				let dism = dmasm::format_disassembly(&nodes, None);

				for offset in &breaks {
					hook_instruction(&proc, *offset).unwrap();
				}

				match error {
					Some(error) => {
						format!("Dism for {:?}\n{}\n\tError: {:?}", proc, dism, error)
					}

					None => {
						format!("Dism for {:?}\n{}", proc, dism)
					}
				}
			}

			None => "Proc not found".to_owned()
		}
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { should_catch } => self.should_catch_runtimes = should_catch,
			Request::BreakpointSet { instruction, condition } => self.handle_breakpoint_set(instruction, condition),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction),
			Request::Stacks => self.handle_stacks(),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars } => self.handle_variables(vars),
			Request::Eval { frame_id, command, context } => self.handle_eval(frame_id, &command, context),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::LineNumber { proc, offset } => {
				self.send_or_disconnect(Response::LineNumber {
					line: self.get_line_number(proc, offset)
				});
			}

			Request::Offset { proc, line } => {
				self.send_or_disconnect(Response::Offset {
					offset: self.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.send_or_disconnect(Response::Ack);
				return true;
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.send_or_disconnect(Response::StdDef(stddef));
			}

			Request::CurrentInstruction { frame_id } => {
				let response = self.get_stack_frame(frame_id).map(|frame| InstructionRef {
					proc: ProcRef {
						path: frame.proc.path.to_owned(),
						override_id: frame.proc.override_id()
					},
					offset: frame.offset as u32
				});

				self.send_or_disconnect(Response::CurrentInstruction(response));
			}

			// The following requests are special cases and handled outside of this function
			Request::Configured | Request::Continue { .. } => {
				self.send_or_disconnect(Response::Ack);
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

	fn wait_for_connection(&mut self) {
		if let ServerStream::Waiting(receiver) = &self.stream {
			if let Ok(stream) = receiver.recv() {
				self.stream = ServerStream::Connected(stream);
			}
		}
	}

	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		eprintln!("Debug Server: {:?}", message);

		if !self.check_connected() {
			return;
		}

		self.send_or_disconnect(Response::Notification { message });
	}

	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> ContinueKind {
		// Ignore all breakpoints unless we're connected
		if !self.check_connected() || (matches!(reason, BreakpointReason::Runtime(_)) && !self.should_catch_runtimes) {
			return ContinueKind::Continue;
		}

		self.state = Some(State::new());

		// Exit now if this is a conditional breakpoint and the condition doesn't pass!
		if reason == BreakpointReason::Breakpoint {
			let proc = unsafe { (*(*_ctx).proc_instance).proc };
			let offset = unsafe { (*_ctx).bytecode_offset };
			let condition = self.conditional_breakpoints.get(&(proc, offset)).cloned();

			if let Some(condition) = condition {
				if let Some(result) = self.eval_expr(Some(0), &condition) {
					if !result.is_truthy() {
						self.state = None;
						return ContinueKind::Continue;
					}
				}

				// We might have just executed some code so invalidate the stacks we already
				// fetched
				self.state.as_mut().unwrap().invalidate_stacks();
			}
		}

		self.notify(format!("Pausing execution (reason: {:?})", reason));
		self.send_or_disconnect(Response::BreakpointHit { reason });

		while let Ok(request) = self.requests.recv() {
			// Hijack and handle any Continue requests
			if let Request::Continue { kind } = request {
				self.send_or_disconnect(Response::Ack);
				self.state = None;
				return kind;
			}

			// Hijack eval too so that we can refresh our state after it
			if let Request::Eval { frame_id, command, context } = request {
				self.handle_eval(frame_id, &command, context);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
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

	/// Block while processing all received requests normally until the debug
	/// client is configured
	pub fn process_until_configured(&mut self) {
		self.wait_for_connection();

		while let Ok(request) = self.requests.recv() {
			if let Request::Configured = request {
				self.send_or_disconnect(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
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
	fn spawn_listener(self, listener: TcpListener, connection_sender: mpsc::Sender<TcpStream>) -> JoinHandle<()> {
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
