use std::{
	cell::{OnceCell, RefCell},
	collections::{BTreeMap, HashMap, VecDeque},
	error::Error,
	io::{Read, Write},
	sync::{
		atomic::{AtomicU64, Ordering},
		mpsc, Mutex
	},
	thread,
	time::{Duration, Instant}
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use clap::{Arg, Command};
use instruction_hooking::disassemble_env;

use super::{
	instruction_hooking::{hook_instruction, original_bytecode, unhook_all, unhook_instruction},
	server_types::*
};
use crate::{
	connection::{self, Connection},
	hot_reload, journal, mem_profiler, unnamed, waker
};

mod render;
mod watches;

/// Next spawned evaluation ID; monotonic across reboots.
static NEXT_SPAWN: AtomicU64 = AtomicU64::new(1);

/// Compiled spawned evaluation waiting for the keepalive loop.
struct Spawn {
	number: u64,
	bytecode: Vec<u32>,
	/// The vars a proc's body declares
	locals: Vec<String>,
	/// Its code's arguments
	args: Vec<Value>
}

/// Expandable variable container. Retain its access expression so child
/// variables can build expressions such as `a.b[2]`.
#[derive(Clone, Hash, PartialEq, Eq)]
enum Variables {
	Arguments {
		frame: u32
	},
	Locals {
		frame: u32
	},
	/// The world's own global vars, and a row for BYOND's
	Globals,
	/// The constants BYOND itself declares (`NORTH`, `MOB_LAYER`, ...)
	Builtins,
	ObjectVars(Value, Option<String>),
	ListContents(Value, Option<String>),
	/// Associative list entry, accessed through expression `entry` in list
	/// `of`.
	ListPair {
		key: Value,
		value: Value,
		of: Value,
		entry: Option<String>
	},
	/// What a pointer names, and the expression the pointer is reached by
	Pointee(Value, Option<String>)
}

/// Pause-local inspection state. Read stacks lazily: most conditional
/// breakpoint hits need only the running stack, and worlds may have
/// thousands of sleeping procs.
struct State {
	active: OnceCell<Vec<debug::StackFrame>>,
	suspended: OnceCell<Vec<Vec<debug::StackFrame>>>,
	/// Number of sleeping stacks requested so far.
	sleeping: usize,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>,
	/// The sleeping stacks' frames a client has been given ids for, by id
	/// (less `SLEEPING_FRAMES`)
	sleeping_frames: RefCell<Vec<SleepingFrame>>,
	sleeping_frame_ids: RefCell<HashMap<SleepingFrame, u32>>
}

/// Frame IDs at or above this value identify sleeping frames. Lower IDs
/// are depths in the running stack, innermost first.
const SLEEPING_FRAMES: u32 = 1 << 30;

/// Stable sleeping-frame reference: stack ID, depth and proc instance.
/// Evaluations can end or create sleeping procs, changing queue positions.
/// The reference must still resolve to the same frame or become invalid.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct SleepingFrame {
	thread: u32,
	depth: usize,
	instance: usize
}

impl State {
	fn new() -> Self {
		Self {
			active: OnceCell::new(),
			suspended: OnceCell::new(),
			sleeping: DEFAULT_SLEEPING_STACKS as usize,
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new()),
			sleeping_frames: RefCell::new(vec![]),
			sleeping_frame_ids: RefCell::new(HashMap::new())
		}
	}

	/// The id of a sleeping stack's frame, which it keeps for the pause
	fn sleeping_frame_id(&self, frame: SleepingFrame) -> u32 {
		let mut frames = self.sleeping_frames.borrow_mut();
		*self.sleeping_frame_ids.borrow_mut().entry(frame).or_insert_with(|| {
			frames.push(frame);
			SLEEPING_FRAMES + (frames.len() as u32 - 1)
		})
	}

	fn sleeping_frame(&self, id: u32) -> Option<SleepingFrame> {
		self.sleeping_frames.borrow().get(id.checked_sub(SLEEPING_FRAMES)? as usize).copied()
	}

	/// Invalidate stack snapshots after evaluating code.
	fn invalidate_stacks(&mut self) {
		self.active = OnceCell::new();
		self.suspended = OnceCell::new();
	}

	/// The running proc's stack
	fn active(&self) -> &Vec<debug::StackFrame> {
		self.active.get_or_init(debug::CallStacks::active)
	}

	/// The sleeping procs' stacks, the first `sleeping` of them
	fn suspended(&self) -> &Vec<Vec<debug::StackFrame>> {
		self.suspended.get_or_init(|| debug::CallStacks::suspended_first(self.sleeping))
	}

	/// Read at least `sleeping` suspended stacks without changing existing
	/// frame IDs.
	fn read_sleeping(&mut self, sleeping: usize) {
		if sleeping > self.sleeping {
			self.sleeping = sleeping;
			self.suspended = OnceCell::new();
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
		// References start at 1: 0 is what a client has for "nothing to open"
		let index = usize::try_from(reference.0).ok()?.checked_sub(1)?;
		variables.get(index).cloned()
	}
}

// Server runs on the BYOND thread; ServerThread handles network input.
// A shared event channel preserves event order.
//
// Clients have independent breakpoints and error filters. A pause stops
// the whole world and any debugging client can resume it. When the last
// debugging client disconnects, execution resumes unless a pause is held.

type ClientId = u32;

/// Source markers used by [`Server::place_in`].
enum Mark {
	File(String),
	Line(u32),
	Other
}

enum Event {
	// A debug-client arrived. The Connection is for the Server to answer on.
	Connected(ClientId, Connection),

	// A request from a debug-client for the Server to handle
	Request(ClientId, Request),

	// The debug-client left, or its connection broke
	Disconnected(ClientId)
}

struct Client {
	// Runtime error filters for uncaught and caught errors.
	uncaught: Option<RuntimeFilter>,
	caught: Option<RuntimeFilter>,
	// Whether this is a debugging client rather than a watcher.
	debugs: bool,
	/// Journal event kinds to stream to this client.
	follow: Option<Vec<JournalKind>>,
	/// Outbound messages for the client writer (see `deliver`).
	outbox: mpsc::SyncSender<Vec<u8>>,
	/// Writer thread; drains queued messages before exiting.
	writer: Option<thread::JoinHandle<()>>,
	/// Maximum held-pause duration without a debugging client.
	hold_max: Option<Duration>
}

/// Disconnect slow clients when their outgoing queue reaches this limit.
const OUTBOX: usize = 1024;

/// How many procs of a runtime error's stack the record keeps
const JOURNAL_FRAMES: usize = 64;

/// What a client wants of a breakpoint it set
pub struct Claim {
	/// It only counts as hit when this holds: `None` for every time
	condition: Option<String>,
	/// Which of its hits to stop at: `None` for all of them
	hit: Option<HitCondition>,
	/// How many times it's been hit since it was set
	hits: u32,
	/// A message for the client at the hits it wants, in place of a stop
	log: Option<Vec<LogPart>>,
	/// How many hits were skipped because the condition failed with an error
	skipped: u32,
	/// The client has been told the condition fails, which it's told once
	told: bool
}

/// Breakpoint claims indexed by location and client. A client can claim
/// the same instruction independently as a line, function and instruction
/// breakpoint. Conditions and removal apply to each claim separately.
pub type Breakpoints = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<(ClientId, Placed), Claim>>;

/// What a client set a breakpoint on
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Placed {
	Line,
	Proc,
	Instruction
}

impl Placed {
	/// Classify the requested offset as an instruction or source breakpoint.
	fn of(offset: u32, exactly: bool) -> Self {
		match offset {
			_ if exactly => Self::Instruction,
			0 => Self::Proc,
			_ => Self::Line
		}
	}
}

/// Synthetic parameter containing `global.vars`. Its name cannot be written
/// in DM expressions, so it cannot shadow user names.
const GLOBALS: &str = "global vars";

/// How long to wait for a request
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
	No,
	/// While there's a client that debugs to send one
	WhileClients,
	/// For as long as it takes, clients coming and going
	Forever,
	/// Until then (if ever), or a client comes or goes
	Until(Option<Instant>)
}

/// Process-lifetime listener. Clients connecting between servers during
/// reboot wait for the next server.
struct Acceptor {
	/// Where the listener's thread sends what it hears
	events: mpsc::Sender<Event>,
	/// The other end, for the next Server to take
	waiting: Option<mpsc::Receiver<Event>>
}

static ACCEPTOR: Mutex<Option<Acceptor>> = Mutex::new(None);

// Bound message lengths before allocating buffers. Full reloads after
// a macro change can legitimately produce large requests.
const MAX_REQUEST_LEN: u32 = 256 * 1024 * 1024;

/// Maximum number of cached compiled evaluations.
const EVAL_CODE_KEPT: usize = 256;

/// Inputs that determine compiled evaluation code.
#[derive(PartialEq, Eq, Hash)]
struct EvalCodeKey {
	/// Code and macro generation at compilation.
	generation: u32,
	/// Code after macro expansion.
	command: String,
	/// Available names, including frame variables.
	names: Vec<String>,
	/// Type of src, used to resolve bare var and proc names.
	src_type: Option<String>
}

pub struct Server {
	events: mpsc::Receiver<Event>,
	clients: BTreeMap<ClientId, Client>,
	// The client whose request is being answered
	current: Option<ClientId>,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	/// Non-fatal callee runtime errors and the number omitted from full
	/// reporting.
	eval_callee_errors: Vec<String>,
	eval_callee_more: usize,
	/// Hot reload owner and its client description (see `Request::Reloader`).
	reloader: Option<(ClientId, String)>,
	/// Last executing evaluation frame; identifies the frame if it sleeps.
	eval_ran: *mut raw_types::procs::ExecutionContext,
	breakpoints: Breakpoints,
	/// Why and where the world is paused, while it is
	paused: Option<Stop>,
	/// The places clients watch for a change
	watches: Vec<watches::Watched>,
	next_watch: u32,
	/// Extra names available to the next evaluation.
	eval_given: Vec<(String, Value)>,
	/// Cache bytecode, local names and warnings by compilation inputs.
	/// Breakpoint conditions and logpoints reuse this code on every hit.
	eval_code: HashMap<EvalCodeKey, Result<(Vec<u32>, Vec<String>, Vec<String>), String>>,
	/// Compatibility warnings from the last evaluation.
	eval_warnings: Vec<String>,
	/// World is loaded and waiting for launcher configuration.
	waiting_to_start: bool,
	/// Object used as `src` for the next frameless evaluation.
	eval_src: Option<Value>,
	/// The number of the expression being compiled to be spawned, which is
	/// queued rather than run
	spawning: Option<u64>,
	/// Spawned expressions waiting for the world's loop
	spawns: VecDeque<Spawn>,
	/// The list the world's loop looks at to know there are some
	spawn_inbox: Option<Value>,
	/// Original spawn-stub locals table, restored after evaluating a body.
	spawn_stub_locals: Option<u32>,
	/// Active spawned evaluations, with source text and first runtime error.
	spawned: HashMap<u64, (String, Option<String>)>,
	/// Client retaining a pause after all clients disconnect.
	hold: Option<ClientId>,
	/// How long that pause lasts with no client that debugs connected
	hold_max: Option<Duration>,
	/// Stepping client to notify if its step ends without pausing.
	stepper: Option<ClientId>,
	/// Proc IDs indexed by debug source name, with the indexed proc count.
	procs_by_file: Option<(u32, HashMap<String, Vec<u32>>)>,
	moved_breakpoints: hot_reload::MovedBreakpoints<(ClientId, Placed)>,
	/// Previously observed bytecode buffers, indexed by `CodeRef::old - 1`.
	old_codes: RefCell<Vec<usize>>,
	app: Command
}

struct ServerThread {
	events: mpsc::Sender<Event>
}

impl Server {
	/// Maximum completion suggestions.
	const MOST_COMPLETIONS: usize = 200;

	pub fn setup_app() -> Command {
		Command::new("Auxtools Debug Server")
			.version(env!("CARGO_PKG_VERSION"))
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
					.arg(Arg::new("proc").help("Path of the proc to disassemble (e.g. /proc/do_stuff)"))
					.arg(Arg::new("id").help("Id of the proc to disassemble (for when multiple procs are defined with the same path)"))
			)
			.subcommand(
				Command::new("guest_override")
					.about("Override the CKey used by guest connections")
					.arg(Arg::new("ckey"))
			)
			.subcommand(
				Command::new("mem_profiler")
					.about("Memory profiler")
					.subcommand(
						Command::new("begin")
							.about("Begins memory profiling. Output goes to the specified file path")
							.arg(Arg::new("path").help("Where to output memory profiler results"))
					)
					.subcommand(Command::new("end").about("Finishes current memory profiler."))
			)
	}

	/// Create or reuse the process listener and accept its queued clients.
	pub fn listen() -> std::io::Result<Server> {
		let events = Self::take_events()?;
		Ok(Server {
			events,
			clients: BTreeMap::new(),
			current: None,
			state: None,
			in_eval: false,
			eval_error: None,
			eval_callee_errors: vec![],
			eval_callee_more: 0,
			reloader: None,
			eval_ran: std::ptr::null_mut(),
			breakpoints: HashMap::new(),
			moved_breakpoints: HashMap::new(),
			paused: None,
			watches: vec![],
			next_watch: 0,
			eval_given: vec![],
			eval_code: HashMap::new(),
			eval_warnings: vec![],
			waiting_to_start: false,
			eval_src: None,
			spawning: None,
			spawns: VecDeque::new(),
			spawn_inbox: None,
			spawn_stub_locals: None,
			spawned: HashMap::new(),
			hold: None,
			hold_max: None,
			stepper: None,
			procs_by_file: None,
			old_codes: RefCell::new(vec![]),
			app: Self::setup_app()
		})
	}

	/// What the listener hears, for one Server at a time
	fn take_events() -> std::io::Result<mpsc::Receiver<Event>> {
		let mut acceptor = ACCEPTOR.lock().unwrap();
		if acceptor.is_none() {
			let listener = connection::listen(&connection::world_name())?;
			let (events, waiting) = mpsc::channel();
			*acceptor = Some(Acceptor {
				events,
				waiting: Some(waiting)
			});
			thread::Builder::new()
				.name("debug server listener".to_owned())
				.spawn(move || Self::accept(&listener))?;
			waker::start()?;
		}
		acceptor
			.as_mut()
			.and_then(|acceptor| acceptor.waiting.take())
			.ok_or_else(|| std::io::Error::other("there's a debug server already"))
	}

	/// The listener's thread
	fn accept(listener: &connection::Listener) {
		let mut next_id = 0;
		loop {
			let Ok(connection) = listener.accept() else {
				continue;
			};
			next_id += 1;

			// Lock to route events to the current server or the next one after
			// reboot.
			let events = {
				let acceptor = ACCEPTOR.lock().unwrap();
				let Some(acceptor) = acceptor.as_ref() else {
					return;
				};
				let _ = acceptor.events.send(Event::Connected(next_id, connection.clone()));
				waker::event_sent();
				acceptor.events.clone()
			};

			// The reader exits when the client closes its connection; do not
			// join it here.
			let server_thread = ServerThread { events };
			let id = next_id;
			thread::spawn(move || server_thread.run(id, connection));
		}
	}

	/// Tells whoever connects that there's no debugger, and why
	pub fn refuse(message: String) {
		let Ok(events) = Self::take_events() else {
			return;
		};
		thread::spawn(move || {
			while let Ok(event) = events.recv() {
				if let Event::Connected(_, mut connection) = event {
					for response in [Response::Notification { message: message.clone() }, Response::Disconnect] {
						if Self::send(&mut connection, &response).is_err() {
							break;
						}
					}
				}
			}
		});
	}

	pub const fn is_in_eval(&self) -> bool {
		self.in_eval
	}

	/// Record the current evaluation frame.
	pub fn eval_runs(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		self.eval_ran = ctx;
	}

	/// Record a runtime error and stack. Only an error in the expression frame
	/// fails evaluation; callee errors end that proc and let evaluation
	/// continue.
	pub fn set_eval_error(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if !Self::called_by_expression(ctx) {
			self.eval_error = Some(Self::error_with_stack(message, ctx));
		} else if self.eval_callee_errors.len() < MAX_CALLEE_ERRORS {
			let error = Self::error_with_stack(message, ctx);
			self.eval_callee_errors.push(error);
		} else {
			self.eval_callee_more += 1;
		}
	}

	/// Whether `ctx` is a callee of the current debugger evaluation.
	fn called_by_expression(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		if ctx.is_null() || is_expression_frame(ctx) {
			return false;
		}
		let mut at = unsafe { *(*ctx).parent_context() };
		while !at.is_null() {
			if is_expression_frame(at) {
				return true;
			}
			at = unsafe { *(*at).parent_context() };
		}
		false
	}

	/// Send compiler warnings and non-fatal callee errors to the client.
	fn send_callee_errors(&mut self) {
		for warning in std::mem::take(&mut self.eval_warnings) {
			self.send_or_disconnect(Response::Notification {
				message: format!("warning: {}", warning)
			});
		}
		let errors = std::mem::take(&mut self.eval_callee_errors);
		let more = std::mem::take(&mut self.eval_callee_more);
		for error in errors {
			self.send_or_disconnect(Response::Notification {
				message: format!("runtime error in a called proc (the evaluation carried on): {}", error)
			});
		}
		if more > 0 {
			self.send_or_disconnect(Response::Notification {
				message: format!("and {} more runtime errors in called procs", more)
			});
		}
	}

	/// Format an evaluation runtime error with its call stack.
	fn error_with_stack(message: &str, ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let mut error = message.to_owned();
		let mut frames = vec![];
		let mut at = ctx;
		while !at.is_null() && !is_expression_frame(at) {
			frames.push(Self::context_place(at));
			at = unsafe { *(*at).parent_context() };
		}
		// Only include frames ending at the expression stub. Other stacks
		// belong to the world or reload machinery.
		if !at.is_null() {
			for (i, place) in frames.iter().enumerate() {
				error += &format!("\n  {} {}", if i == 0 { "in" } else { "called by" }, place);
			}
			if !frames.is_empty() {
				error += "\n  called by the expression";
			}
		}
		error
	}

	/// Record the first uncaught runtime error in a spawned evaluation's own
	/// code. One in a proc it called ends only that proc, as in DM; the world
	/// reports it as any other.
	pub fn spawn_runtime(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if self.spawned.is_empty() {
			return;
		}
		let (Some(stub), Some(spawned)) = (Proc::find("/proc/auxtools_spawn_stub"), Proc::find("/proc/auxtools_spawned")) else {
			return;
		};
		let proc_of = |at: *mut raw_types::procs::ExecutionContext| unsafe { (**(*at).proc_instance()).proc };
		let mut at = ctx;
		let mut in_stub = false;
		while !at.is_null() {
			let id = proc_of(at);
			if id == stub.id {
				// The spawned code's own `try` catches it
				if unsafe { *(*at).throw_handler() } != 0 {
					return;
				}
				if at != ctx {
					return;
				}
				in_stub = true;
			} else if in_stub && id == spawned.id {
				// Its number is its first argument
				let number = unsafe { Value::from_raw(*(**(*at).proc_instance()).args()) }.as_number().ok();
				if let Some((_, error @ None)) = number.and_then(|number| self.spawned.get_mut(&(number as u64))) {
					*error = Some(format!("runtime error: {}", Self::error_with_stack(message, ctx)));
				}
				return;
			} else if !in_stub && unsafe { *(*at).throw_handler() } != 0 {
				// Caught by the code it called
				return;
			}
			at = unsafe { *(*at).parent_context() };
		}
	}

	/// A running frame's proc, and its file and line: `/mob/proc/x  code.dm:12`
	fn context_place(ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
		let mut place = match &proc {
			Some(proc) => unnamed::proc_name(proc),
			None => "a proc the world doesn't have".to_owned()
		};
		// The world keeps the line it's on when it has its code's lines
		let line = unsafe { (*ctx).filename().valid().then(|| *(*ctx).line()) };
		match (proc.as_ref().and_then(Self::file_of), line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(None, Some(line)) => place += &format!("  line {}", line),
			_ => {}
		}
		place
	}

	/// Resolve duplicate local names using each slot's first and last bytecode
	/// use; BYOND stores no lexical scopes. Prefer an active slot, then the
	/// latest first use (the inner slot in nested blocks). If none is active,
	/// use the most recently started slot; before any use, expose none. Unique
	/// names remain visible throughout. Return `None` for hidden slots.
	pub(crate) fn visible_locals(frame: &debug::StackFrame) -> Vec<Option<String>> {
		let names: Vec<String> = frame.locals.iter().map(|(name, _)| String::from(name)).collect();
		let shared = |name: &String| names.iter().filter(|other| *other == name).count() > 1;
		if !names.iter().any(shared) {
			return names.into_iter().map(Some).collect();
		}
		let code = Self::old_code_of(frame).unwrap_or_else(|| original_bytecode(&frame.proc));
		let used = local_uses(&code);
		let at = frame.offset as u32;
		// Rank started slots by active use, then latest first use.
		let rank = |slot: usize| {
			used.get(&(slot as u32))
				.filter(|(first, _)| *first <= at)
				.map(|(first, last)| (at <= *last, *first))
		};
		names
			.iter()
			.enumerate()
			.map(|(slot, name)| {
				if !shared(name) {
					return Some(name.clone());
				}
				let mine = rank(slot)?;
				let meant = names
					.iter()
					.enumerate()
					.filter(|(other, other_name)| *other != slot && *other_name == name)
					.filter_map(|(other, _)| rank(other))
					.all(|theirs| theirs < mine);
				meant.then(|| name.clone())
			})
			.collect()
	}

	/// Find a frame's pre-reload bytecode. Return `None` if it matches current
	/// code or differs only in line numbers, which can use current mappings.
	fn old_code_of(frame: &debug::StackFrame) -> Option<Vec<u32>> {
		let running = unsafe { *(*frame.context).bytecode() };
		let (current, _) = unsafe { frame.proc.bytecode_mut_ptr() };
		if running == current {
			return None;
		}
		let old = auxtools::bytecode_at(running)?;

		let instructions = |bytecode: &[u32]| -> Vec<dmasm::Instruction> {
			let mut env = disassemble_env::DisassembleEnv;
			let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);
			nodes
				.into_iter()
				.filter_map(|node| match node {
					dmasm::Node::Instruction(dmasm::Instruction::DbgLine(_), _) => Some(dmasm::Instruction::DbgLine(0)),
					dmasm::Node::Instruction(ins, _) => Some(ins),
					_ => None
				})
				.collect()
		};
		let now = original_bytecode(&frame.proc);
		if old.len() == now.len() && instructions(&old) == instructions(&now) {
			return None;
		}
		Some(old)
	}

	/// Identify a frame's bytecode, assigning a reference to old code on first
	/// use.
	fn code_of(&self, frame: &debug::StackFrame, old: bool) -> CodeRef {
		let mut code = CodeRef {
			proc: frame.proc.id.0,
			old: 0
		};
		if old {
			let running = unsafe { *(*frame.context).bytecode() } as usize;
			let mut old_codes = self.old_codes.borrow_mut();
			let number = match old_codes.iter().position(|&old| old == running) {
				Some(at) => at + 1,
				None => {
					old_codes.push(running);
					old_codes.len()
				}
			};
			code.old = u16::try_from(number).unwrap_or(u16::MAX);
		}
		code
	}

	/// The instructions of some code, as `Request::Disassemble` gives them
	fn disassemble(&self, code: CodeRef) -> Result<Disassembly, String> {
		let proc = Proc::from_id(raw_types::procs::ProcId(code.proc)).ok_or_else(|| format!("the world has no proc {:#x}", code.proc))?;
		let bytecode = if code.old == 0 {
			original_bytecode(&proc)
		} else {
			let running = self.old_codes.borrow().get(code.old as usize - 1).copied();
			let old = running.and_then(|running| Some((running, auxtools::bytecode_at(running as *const u32)?)));
			match old {
				Some((running, old)) => crate::instruction_hooking::unpatched(running as *const u32, old),
				None => return Err(format!("{} has no old code {}", proc.path, code.old))
			}
		};

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		// Before its first line, an instruction is on that line, as a frame is
		let first_line = nodes.iter().find_map(|node| match node {
			dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) => Some(*line),
			_ => None
		});
		let mut line = first_line;
		let mut instructions = vec![];
		let mut read = 0;
		for node in &nodes {
			let dmasm::Node::Instruction(ins, debug) = node else {
				continue;
			};
			if let dmasm::Instruction::DbgLine(at) = ins {
				line = Some(*at);
			}
			instructions.push(BytecodeInstruction {
				offset: debug.offset as u16,
				words: debug.bytecode.to_vec(),
				text: ins.to_string(),
				line
			});
			read = debug.offset as usize + debug.bytecode.len();
		}
		// What couldn't be read is there all the same
		for (at, word) in bytecode.iter().enumerate().skip(read) {
			instructions.push(BytecodeInstruction {
				offset: at as u16,
				words: vec![*word],
				text: String::new(),
				line: None
			});
		}

		Ok(Disassembly {
			proc: unnamed::proc_ref(&proc),
			file: Self::file_of(&proc),
			instructions,
			error: error.map(|error| format!("{:?}", error))
		})
	}

	/// Where a proc is, how many definitions its path has, and what hot
	/// reloads did to it (`Request::ProcInfo`)
	fn proc_info(wanted: &ProcRef) -> Result<ProcInfo, String> {
		let definitions = (0..).take_while(|&n| unnamed::find(&wanted.path, n).is_some()).count() as u32;
		let proc = match unnamed::find(&wanted.path, wanted.override_id) {
			Some(proc) => proc,
			None if definitions == 0 => return Err(format!("the world has no proc {}", wanted.path)),
			None if definitions == 1 => return Err(format!("{} has one definition, #0: there's no #{}", wanted.path, wanted.override_id)),
			None => {
				return Err(format!(
					"{} has {} definitions, #0 to #{}: there's no #{}",
					wanted.path,
					definitions,
					definitions - 1,
					wanted.override_id
				))
			}
		};
		Ok(ProcInfo {
			proc: unnamed::proc_ref(&proc),
			code: CodeRef { proc: proc.id.0, old: 0 },
			file: Self::file_of(&proc),
			line: Self::lines_of(&proc).into_iter().next(),
			definitions,
			reloaded: hot_reload::reloaded(proc.id.0)
		})
	}

	/// Resolve a frame's source position. A sleeping frame points after the
	/// sleep instruction; if that is the next DbgLine, report the preceding
	/// line.
	fn frame_place(&self, frame: &debug::StackFrame, asleep: bool) -> (Option<String>, Option<u32>) {
		let offset = if asleep {
			(frame.offset as u32).saturating_sub(1)
		} else {
			frame.offset as u32
		};

		if let Some(bytecode) = Self::old_code_of(frame) {
			return match Self::place_in(&bytecode, offset, false) {
				Some((file, line)) => (file.or_else(|| Self::file_of(&frame.proc)), Some(line)),
				None => (Self::file_of(&frame.proc), None)
			};
		}
		Self::place_of(&frame.proc, offset)
	}

	/// A frame as a client is shown it, known to it by `id`
	fn frame_info(&self, frame: &debug::StackFrame, id: u32, asleep: bool) -> StackFrame {
		let old_code = Self::old_code_of(frame).is_some();
		let (file, line) = self.frame_place(frame, asleep);
		StackFrame {
			id,
			instruction: InstructionRef {
				proc: unnamed::proc_ref(&frame.proc),
				offset: frame.offset as u32
			},
			line,
			file,
			old_code,
			code: self.code_of(frame, old_code)
		}
	}

	/// The file a proc is in, if its code starts with DbgFile
	fn file_of(proc: &Proc) -> Option<String> {
		let dbg_file = dmasm::Opcode::DbgFile.word();
		let bytecode = unsafe { proc.bytecode() };
		let id = match bytecode {
			[op, id, ..] if *op == dbg_file => *id,
			// A breakpoint at its start hides it
			[_, ..] => match crate::instruction_hooking::original_bytecode(proc).as_slice() {
				[op, id, ..] if *op == dbg_file => *id,
				_ => return None
			},
			[] => return None
		};
		let name = unsafe { StringRef::from_id(raw_types::strings::StringId(id)) };
		Some(String::from_utf8_lossy(name.data()).into_owned())
	}

	/// Index every source file named by each proc's DbgFile instructions.
	/// Initializer procs may refer to several files.
	fn file_index(&mut self) -> &HashMap<String, Vec<u32>> {
		// Counted again when procs were added
		let count = auxtools::runtime_tables::proc_count();
		if self.procs_by_file.as_ref().is_none_or(|(counted, _)| *counted != count) {
			let dbg_file = dmasm::Opcode::DbgFile.word();
			let mut by_file: HashMap<String, Vec<u32>> = HashMap::new();
			for id in 0..count {
				let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
					continue;
				};
				let Some(file) = Self::file_of(&proc) else { continue };
				// Avoid full disassembly unless the bytecode contains a
				// possible DbgFile opcode.
				if unsafe { proc.bytecode() }.iter().skip(2).any(|&word| word == dbg_file) {
					for other in Self::files_of(&proc) {
						if other != file {
							by_file.entry(other).or_default().push(id);
						}
					}
				}
				by_file.entry(file).or_default().push(id);
			}
			self.procs_by_file = Some((count, by_file));
		}
		&self.procs_by_file.as_ref().unwrap().1
	}

	/// The files a proc's code names (its DbgFiles), each once
	fn files_of(proc: &Proc) -> Vec<String> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let mut files: Vec<String> = vec![];
		for node in nodes {
			if let dmasm::Node::Instruction(dmasm::Instruction::DbgFile(name), _) = node {
				let name = String::from_utf8_lossy(&name.0).into_owned();
				if !files.contains(&name) {
					files.push(name);
				}
			}
		}
		files
	}

	/// A file's name as two of them are compared
	fn normalized(path: &str) -> String {
		path.replace('\\', "/").to_ascii_lowercase()
	}

	/// Locate procs and executable lines for `file`, and return matching
	/// normalized debug names.
	fn procs_in(&mut self, file: &str) -> (Vec<String>, Vec<(ProcRef, Vec<u32>)>) {
		let normalize = Self::normalized;
		let wanted = normalize(file);
		// Prefer an exact debug name. Only fall back to suffix matching if
		// none exists: `map.dm` and `code/map.dm` can be distinct files.
		let same = |file: &str| file == wanted;
		let alike = |file: &str| file.ends_with(&format!("/{}", wanted)) || wanted.ends_with(&format!("/{}", file));

		let index = self.file_index();
		let matching = |test: &dyn Fn(&str) -> bool| -> (Vec<String>, Vec<u32>) {
			let files: Vec<(&String, &Vec<u32>)> = index.iter().filter(|(file, _)| test(&normalize(file))).collect();
			let mut ids: Vec<u32> = files.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
			ids.sort_unstable();
			ids.dedup();
			(files.iter().map(|(file, _)| normalize(file)).collect(), ids)
		};
		let (mut files, mut ids) = matching(&same);
		if ids.is_empty() {
			(files, ids) = matching(&alike);
		}
		let procs = ids
			.into_iter()
			.filter_map(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.map(|proc| {
				let lines = Self::lines_in(&proc, &files);
				let proc = unnamed::proc_ref(&proc);
				(proc, lines)
			})
			.collect();
		(files, procs)
	}

	/// The lines from `start` to `end` of `file` that some proc has code on
	fn lines_with_code(&mut self, file: &str, start: u32, end: u32) -> Vec<u32> {
		let mut lines: Vec<u32> = self
			.procs_in(file)
			.1
			.into_iter()
			.flat_map(|(_, lines)| lines)
			.filter(|line| (start..=end).contains(line))
			.collect();
		lines.sort_unstable();
		lines.dedup();
		lines
	}

	/// Where each proc with code on `line` of `file` has it
	fn locate(&mut self, file: &str, line: u32) -> Vec<InstructionRef> {
		// Move non-executable line breakpoints to the next executable line.
		let (files, procs) = self.procs_in(file);
		let Some(line) = procs.iter().flat_map(|(_, lines)| lines.iter().copied()).filter(|&l| l >= line).min() else {
			return vec![];
		};

		let mut found = vec![];
		for (proc, lines) in procs {
			if !lines.contains(&line) {
				continue;
			}
			if let Some(offset) = self.offset_of(proc.clone(), line, Some(&files)) {
				found.push(InstructionRef { proc, offset });
			}
		}
		found
	}

	/// The lines of `files` (`normalized`) a proc has code on: its DbgLines
	/// while its code is of one of them
	fn lines_in(proc: &Proc, files: &[String]) -> Vec<u32> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let mut inside = false;
		let mut lines = vec![];
		for node in nodes {
			match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgFile(name), _) => {
					inside = files.contains(&Self::normalized(&String::from_utf8_lossy(&name.0)));
				}
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) if inside => lines.push(line),
				_ => {}
			}
		}
		lines
	}

	/// The lines a proc has code on (its DbgLines)
	fn lines_of(proc: &Proc) -> Vec<u32> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		nodes
			.into_iter()
			.filter_map(|node| match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) => Some(line),
				_ => None
			})
			.collect()
	}

	/// Resolve source file and line at a bytecode offset. A proc can switch
	/// files through DbgFile, especially in initializers and `::` defaults.
	///
	/// If `before` is set and the offset points to DbgLine, report the
	/// preceding line. Non-executable line breakpoints can stop there.
	/// A DbgFile at this offset has not run yet; keep the preceding line's
	/// file. DreamMaker can insert the .dme's location between repeated source
	/// locations at the start of an included file's last proc.
	fn place_in(bytecode: &[u32], offset: u32, before: bool) -> Option<(Option<String>, u32)> {
		let mut current_line_number = None;
		let mut current_file = None;
		let mut reached_offset = false;

		for (at, mark) in Self::marks(bytecode).iter() {
			if *at > offset && !reached_offset {
				reached_offset = true;
				if current_line_number.is_some() {
					break;
				}
			}

			match mark {
				Mark::File(name) if !((reached_offset || *at == offset) && current_line_number.is_some()) => {
					current_file = Some(name.clone());
				}
				Mark::Line(_) if before && *at == offset => {}
				Mark::Line(line) => {
					current_line_number = Some(*line);
					// Before its first line, a proc is at its first line
					if reached_offset {
						break;
					}
				}
				_ => {}
			}

			if *at == offset {
				reached_offset = true;
				if current_line_number.is_some() {
					break;
				}
			}
		}

		if reached_offset {
			current_line_number.map(|line| (current_file, line))
		} else {
			None
		}
	}

	/// Cache instruction offsets and source markers by bytecode content.
	/// Runtime stack traces would otherwise disassemble each frame's proc.
	fn marks(bytecode: &[u32]) -> std::sync::Arc<[(u32, Mark)]> {
		use std::hash::{Hash, Hasher};
		type Marks = std::sync::Arc<[(u32, Mark)]>;
		static MARKS: std::sync::LazyLock<Mutex<HashMap<(u64, usize), Marks>>> = std::sync::LazyLock::new(Default::default);
		// Bound cache memory during long sessions.
		const KEEP: usize = 8192;

		let key = {
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			bytecode.hash(&mut hasher);
			(hasher.finish(), bytecode.len())
		};
		if let Some(marks) = MARKS.lock().unwrap().get(&key) {
			return marks.clone();
		}

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);
		let marks: Marks = nodes
			.into_iter()
			.filter_map(|node| match node {
				dmasm::Node::Instruction(ins, debug) => Some((debug.offset, match ins {
					dmasm::Instruction::DbgFile(name) => Mark::File(String::from_utf8_lossy(&name.0).into_owned()),
					dmasm::Instruction::DbgLine(line) => Mark::Line(line),
					_ => Mark::Other
				})),
				_ => None
			})
			.collect();

		let mut cache = MARKS.lock().unwrap();
		if cache.len() >= KEEP {
			cache.clear();
		}
		cache.insert(key, marks.clone());
		marks
	}

	/// The file and line of the code at `offset` of a proc. The file is the
	/// one the proc starts in when its code doesn't say there.
	fn place_of(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		// Read unpatched bytecode. Only a breakpoint at this offset requires
		// the preceding line position.
		let original = original_bytecode(proc);
		let here = offset as usize;
		let breakpoint_here = unsafe { proc.bytecode() }.get(here) != original.get(here);
		Self::place_at(proc, &original, offset, breakpoint_here)
	}

	/// The file and line a breakpoint at `offset` of a proc is on: where the
	/// world is when it stops there
	fn breakpoint_place(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		Self::place_at(proc, &original_bytecode(proc), offset, true)
	}

	fn place_at(proc: &Proc, original: &[u32], offset: u32, before: bool) -> (Option<String>, Option<u32>) {
		match Self::place_in(original, offset, before) {
			Some((file, line)) => (file.or_else(|| Self::file_of(proc)), Some(line)),
			None => (Self::file_of(proc), None)
		}
	}

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		self.offset_of(proc, line, None)
	}

	/// Where a proc's code for `line` starts: the line of one of `files`
	/// (`normalized`) when they're given, as a proc can have code of several
	/// (see `place_in`)
	fn offset_of(&self, proc: ProcRef, line: u32, files: Option<&[String]>) -> Option<u32> {
		let proc = unnamed::find(&proc.path, proc.override_id)?;
		let mut offset = None;
		let mut at_offset = false;
		let mut other_file = false;
		let mut inside = files.is_none();

		// Without the breakpoints in it: a line's breakpoint can be where the
		// next line's DbgLine was
		let bytecode = original_bytecode(&proc);

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				// Skip intervening file markers before placing the breakpoint
				// (see `place_in`).
				if at_offset {
					match ins {
						dmasm::Instruction::DbgFile(_) => {
							other_file = true;
							continue;
						}
						dmasm::Instruction::DbgLine(_) if other_file => continue,
						_ => {
							offset = Some(debug.offset);
							break;
						}
					}
				}

				match ins {
					dmasm::Instruction::DbgFile(name) => {
						inside = files.is_none_or(|files| files.contains(&Self::normalized(&String::from_utf8_lossy(&name.0))));
					}
					dmasm::Instruction::DbgLine(current_line) if inside && current_line == line => at_offset = true,
					_ => {}
				}
			}
		}

		offset
	}

	// 516 primitives have no `vars` list; enumerate their fields explicitly.
	// Callee fields are readable only while the proc runs. Do not retain
	// a savefile's `vars` list: BYOND does not count its references and
	// reports "Bad ref" when released.
	const fn primitive_fields(value: &Value) -> Option<&'static [&'static str]> {
		match value.raw.tag {
			ValueTag::Vector => Some(&["x", "y", "z", "len", "size"]),
			ValueTag::PixLoc => Some(&["x", "y", "z", "step_x", "step_y", "loc"]),
			ValueTag::Callee => Some(&["proc", "file", "line", "src", "usr", "args", "caller"]),
			ValueTag::SaveFile => Some(&["type", "name", "cd", "dir", "eof", "byond_version", "byond_build"]),
			_ => None
		}
	}

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
			return true;
		}

		// A typepath answers for `vars` too, with something that isn't a list
		Self::primitive_fields(value).is_some() || value.get(byond_string!("vars")).is_ok_and(|vars| List::is_list(&vars))
	}

	fn stringify(value: &Value) -> String {
		if List::is_list(value) {
			match List::from_value(value) {
				Ok(list) => format!("/{} {{len = {}}}", if list.is_alist() { "alist" } else { "list" }, list.len()),
				Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message)
			}
		} else {
			match value.to_string() {
				// What BYOND has no text for
				Ok(v) if v.is_empty() => match value.raw.tag {
					ValueTag::World => "world".to_owned(),
					// Of a proc that has ended
					ValueTag::Pointer => "a pointer to a var that's gone".to_owned(),
					ValueTag::Image => "/image".to_owned(),
					ValueTag::Appearance => "appearance".to_owned(),
					_ => value.raw.to_string()
				},
				Ok(value) => value,
				Err(Runtime { message }) => {
					format!("{} -- stringify error: {:?}", value.raw, message)
				}
			}
		}
	}

	/// Return an object typepath or a primitive kind label.
	fn kind(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		if List::is_list(value) {
			return Some(
				if List::from_value(value).is_ok_and(|list| list.is_alist()) {
					"/alist"
				} else {
					"/list"
				}
				.to_owned()
			);
		}
		let word = match value.raw.tag {
			Tag::Null => "null",
			Tag::Number => "num",
			Tag::String => "text",
			Tag::World => "/world",
			Tag::Resource | Tag::File => "file",
			Tag::ProcRef => "proc",
			Tag::Pointer => "pointer",
			Tag::Appearance => "appearance",
			Tag::Vector => "/vector",
			Tag::PixLoc => "/pixloc",
			Tag::Callee => "/callee",
			Tag::MobTypepath
			| Tag::ObjTypepath
			| Tag::TurfTypepath
			| Tag::AreaTypepath
			| Tag::DatumTypepath
			| Tag::SaveFileTypepath
			| Tag::ListTypepath
			| Tag::ClientTypepath
			| Tag::ImageTypepath
			| Tag::BuiltinTypepath => "path",
			_ => return value.get(byond_string!("type")).ok().map(|path| Self::stringify(&path))
		};
		Some(word.to_owned())
	}

	/// A value written as DM code, if it's one that code can write: a key
	/// to read a list's entry by
	fn literal(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		match value.raw.tag {
			Tag::String => Self::text_literal(StringRef::from_value(value.clone())?.data()),
			Tag::Number | Tag::MobTypepath | Tag::ObjTypepath | Tag::TurfTypepath | Tag::AreaTypepath | Tag::DatumTypepath => {
				Some(Self::stringify(value))
			}
			_ => None
		}
	}

	/// Encode text as an exact DM string literal. Macros and control characters
	/// cannot be represented this way.
	fn text_literal(bytes: &[u8]) -> Option<String> {
		let mut literal = String::with_capacity(bytes.len() + 2);
		literal.push('"');
		for c in std::str::from_utf8(bytes).ok()?.chars() {
			match c {
				'\\' | '"' | '[' | ']' => {
					literal.push('\\');
					literal.push(c);
				}
				'\n' => literal.push_str("\\n"),
				'\t' => literal.push_str("\\t"),
				_ if c.is_control() => return None,
				_ => literal.push(c)
			}
		}
		literal.push('"');
		Some(literal)
	}

	/// Build a list-entry expression using a literal key, object ref, or
	/// one-based position. Positional lookup is unavailable for alists.
	fn entry_expr(list: &str, key: &Value, position: Option<u32>) -> Option<String> {
		use ValueTag as Tag;
		if let Some(key) = Self::literal(key) {
			return Some(format!("{}[{}]", list, key));
		}
		let locatable = matches!(
			key.raw.tag,
			Tag::Turf | Tag::Obj | Tag::Mob | Tag::Area | Tag::Client | Tag::Image | Tag::Datum | Tag::List | Tag::Alist
		);
		if locatable {
			return Some(format!("{}[locate(\"\\{}\")]", list, Self::reference(key)));
		}
		position.map(|i| format!("{}[{}[{}]]", list, list, i))
	}

	/// Build a variable descriptor with an optional access expression.
	fn value_to_variable(&self, name: String, value: &Value, expr: Option<String>) -> Variable {
		Variable {
			name,
			value: Self::stringify(value),
			variables: self.value_to_variables_ref(value, expr.clone()),
			kind: Self::kind(value),
			entries: Self::entries(value),
			expr
		}
	}

	/// How many entries a list has
	fn entries(value: &Value) -> Option<u32> {
		if !List::is_list(value) {
			return None;
		}
		List::from_value(value).ok().map(|list| list.len())
	}

	fn value_to_variables_ref(&self, value: &Value, expr: Option<String>) -> Option<VariablesRef> {
		match self.state.as_ref() {
			Some(state) if List::is_list(value) => Some(state.get_ref(Variables::ListContents(value.clone(), expr))),

			Some(state) if Self::is_object(value) => Some(state.get_ref(Variables::ObjectVars(value.clone(), expr))),

			// A pointer opens to what it names
			Some(state) if value.raw.tag == ValueTag::Pointer && auxtools::pointer::read_raw(value).is_some() => {
				Some(state.get_ref(Variables::Pointee(value.clone(), expr)))
			}

			_ => None
		}
	}

	/// List entries starting at zero-based `start`, optionally limited to
	/// `count`.
	fn list_to_variables(&mut self, value: &Value, expr: Option<&str>, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();
		let first = start.unwrap_or(0).min(len);
		let last = count.map_or(len, |count| first.saturating_add(count).min(len));

		// Expose `key = value` as an expandable pair accessed through
		// `list[key]`. Alist numbers are keys, so entries cannot be accessed by
		// position.
		let alist = list.is_alist();
		let pair = |i: u32, key: Value, value: Value| {
			let entry = expr.and_then(|list| Self::entry_expr(list, &key, (!alist).then_some(i)));
			Variable {
				name: format!("[{}]", i),
				value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
				kind: Self::kind(&value),
				entries: None,
				expr: entry.clone(),
				variables: Some(state.get_ref(Variables::ListPair {
					key,
					value,
					of: Value::from(&list),
					entry
				}))
			}
		};

		// numbers are keys in an alist, so the position walk below would read
		// the wrong entries
		if alist {
			return Ok(list
				.alist_pairs()?
				.into_iter()
				.enumerate()
				.skip(first as usize)
				.take((last - first) as usize)
				.map(|(i, (key, value))| pair(i as u32 + 1, key, value))
				.collect());
		}

		let mut variables = vec![];

		for i in first + 1..=last {
			let key = list.get(i)?;

			// assoc entry
			if key.raw.tag != raw_types::values::ValueTag::Number {
				if let Ok(value) = list.get(&key) {
					if value.raw.tag != raw_types::values::ValueTag::Null {
						variables.push(pair(i, key, value));
						continue;
					}
				}
			}

			// non-assoc entry
			variables.push(self.value_to_variable(format!("[{}]", i), &key, expr.map(|list| format!("{}[{}]", list, i))));
		}

		Ok(variables)
	}

	fn object_to_variables(&mut self, value: &Value, expr: Option<&str>) -> Result<Vec<Variable>, Runtime> {
		let member = |name: &str| expr.map(|object| format!("{}.{}", object, name));

		if let Some(fields) = Self::primitive_fields(value) {
			return fields
				.iter()
				.map(|&name| Ok(self.value_to_variable(name.to_owned(), &value.get(StringRef::new(name)?)?, member(name))))
				.collect();
		}

		let vars = List::from_value(&value.get(byond_string!("vars"))?)?;

		let mut variables = vec![];
		let mut top_variables = vec![]; // These fields get displayed on top of
										// all others

		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(StringRef::new(name.as_str())?)?;
			let expr = member(&name);
			let variable = self.value_to_variable(name, &value, expr);
			if variable.name == "type" {
				top_variables.push(variable);
			} else {
				variables.push(variable);
			}
		}

		variables.sort_by_key(|a| a.name.to_lowercase());
		top_variables.append(&mut variables);

		Ok(top_variables)
	}

	/// The names of the constants BYOND declares itself, from its stddef.dm:
	/// the lines under each `var/const`
	fn builtin_constants() -> std::collections::HashSet<String> {
		let mut names = std::collections::HashSet::new();
		let mut in_consts = false;
		for line in crate::stddef::get_stddef().unwrap_or_default().lines() {
			if line.trim_end() == "var/const" {
				in_consts = true;
			} else if in_consts && line.starts_with([' ', '\t']) {
				if let Some(name) = line.split(['=', '/']).next().map(str::trim).filter(|name| !name.is_empty()) {
					names.insert(name.to_owned());
				}
			} else if !line.trim().is_empty() {
				in_consts = false;
			}
		}
		names
	}

	/// List world globals separately from built-in constants.
	fn globals_to_variables(&mut self, builtin: bool) -> Result<Vec<Variable>, Runtime> {
		let names = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) })?;
		let builtins = Self::builtin_constants();

		let mut variables = vec![];
		let mut constants = 0;
		for i in 1..=names.len() {
			let name = names.get(i)?.as_string()?;
			if builtins.contains(&name) != builtin {
				constants += 1;
				continue;
			}
			let value = Value::GLOBAL.get(StringRef::new(name.as_str())?)?;
			let expr = format!("global.{}", name);
			variables.push(self.value_to_variable(name, &value, Some(expr)));
		}
		variables.sort_by_key(|a| a.name.to_lowercase());

		if !builtin && constants > 0 {
			variables.push(Variable {
				name: "(BYOND's)".to_owned(),
				value: format!("{} constants", constants),
				variables: self.state.as_ref().map(|state| state.get_ref(Variables::Builtins)),
				kind: None,
				expr: None,
				entries: None
			});
		}
		Ok(variables)
	}

	/// Hide outer debugger helper frames, preserving frame positions and IDs.
	/// Keep stacks containing only helpers visible, so a pause in the keepalive
	/// loop still has a location.
	fn shown(stack: &[debug::StackFrame]) -> &[debug::StackFrame] {
		match stack.iter().rposition(|frame| !crate::launched::runs_spawns(&frame.proc)) {
			Some(last) => &stack[..=last],
			None => stack
		}
	}

	/// Whether a sleeping stack contains only debugger helpers.
	fn only_runs_spawns(instance: *mut raw_types::procs::ProcInstance) -> bool {
		// From the outermost frame to the innermost
		let mut ctx = unsafe { (*instance).context };
		while !ctx.is_null() {
			let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
			if !proc.is_some_and(|proc| crate::launched::runs_spawns(&proc)) {
				return false;
			}
			ctx = unsafe { *(*ctx).parent_context() };
		}
		true
	}

	/// Use the outermost proc instance's BYOND ID as a stable stack ID.
	/// The instance address changes on first sleep; spawned copies receive
	/// distinct IDs. IDs may be reused after a proc ends. For spawned code,
	/// use the outermost visible frame so its ID survives sleeping.
	fn thread_of(&self, stack: &[debug::StackFrame]) -> u32 {
		// 0 is "the running proc's" in a request
		Self::shown(stack)
			.last()
			.map_or(0, |frame| unsafe { (*frame.instance).argslist_idx.id }.wrapping_add(1))
	}

	/// Where the stack a client calls `thread` is: 0 for the running proc's
	/// (which a client can call 0 too), then the sleeping ones as BYOND has
	/// them
	fn stack_position(&self, thread: u32) -> Option<usize> {
		let state = self.state.as_ref()?;
		if thread == 0 {
			return Some(0);
		}
		let is = |stack: &[debug::StackFrame]| self.thread_of(stack) == thread;
		if is(state.active()) {
			return Some(0);
		}
		state.suspended().iter().position(|stack| is(stack)).map(|at| at + 1)
	}

	fn get_stack(&self, position: usize) -> Option<&Vec<debug::StackFrame>> {
		let state = self.state.as_ref()?;

		if position == 0 {
			return Some(state.active());
		}

		state.suspended().get(position - 1)
	}

	/// The frame a client knows by `frame_index`, if it's still there
	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let state = self.state.as_ref()?;
		if frame_index < SLEEPING_FRAMES {
			return state.active().get(frame_index as usize);
		}

		let frame = state.sleeping_frame(frame_index)?;
		let position = self.stack_position(frame.thread).filter(|&position| position != 0)?;
		self.get_stack(position)?
			.get(frame.depth)
			.filter(|found| found.instance as usize == frame.instance)
	}

	fn get_args(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		let mut vars = vec![
			self.value_to_variable("src".to_owned(), &frame.src, Some("src".to_owned())),
			self.value_to_variable("usr".to_owned(), &frame.usr, Some("usr".to_owned())),
		];

		let mut unnamed_count = 0;
		for (i, (name, value)) in frame.args.iter().enumerate() {
			// One more than the proc declares is only in its args
			let (name, expr) = match name {
				Some(name) => (String::from(name), String::from(name)),
				None => {
					unnamed_count += 1;
					(format!("undefined argument #{}", unnamed_count), format!("args[{}]", i + 1))
				}
			};
			vars.push(self.value_to_variable(name, value, Some(expr)));
		}

		vars
	}

	fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		let mut vars = vec![self.value_to_variable(".".to_owned(), &frame.dot, Some(".".to_owned()))];

		// Only expose the local selected by the bytecode-use heuristic for each
		// name.
		for (name, (_, local)) in Self::visible_locals(&frame).into_iter().zip(&frame.locals) {
			if let Some(name) = name {
				vars.push(self.value_to_variable(name.clone(), local, Some(name)));
			}
		}

		vars
	}

	/// Resolve function breakpoints after initial file/line records so the
	/// first step advances to the next line. Use offset 0 if records are
	/// absent.
	fn entry_offset(proc: &Proc) -> u32 {
		let bytecode = original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

		let mut has_line = false;
		for node in nodes {
			match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgFile(_), _) => {}
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(_), _) => has_line = true,
				dmasm::Node::Instruction(_, debug) if has_line => return debug.offset,
				dmasm::Node::Instruction(..) => return 0,
				_ => {}
			}
		}
		0
	}

	/// The offset a client means: 0 is the proc itself (see `entry_offset`)
	fn breakpoint_offset(proc: &Proc, offset: u32) -> u32 {
		if offset == 0 {
			Self::entry_offset(proc)
		} else {
			offset
		}
	}

	/// Sets a breakpoint: `exactly` at the instruction asked for, or else
	/// where a client means (`breakpoint_offset`)
	fn handle_breakpoint_set(
		&mut self,
		mut instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>,
		exactly: bool
	) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.current) else {
			self.send_or_disconnect(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let placed = Placed::of(instruction.offset, exactly);
		if !exactly {
			instruction.offset = Self::breakpoint_offset(&proc, instruction.offset);
		}
		let (file, line) = Self::breakpoint_place(&proc, instruction.offset);

		// Compile conditions and interpolated messages when setting the
		// breakpoint, so syntax errors are reported once before any hit.
		let log_error = || {
			log.iter().flatten().find_map(|part| match part {
				LogPart::Expr(expression) => {
					Self::expression_error(&proc, expression).map(|e| format!("the message's {{{}}} doesn't compile: {}", expression, e))
				}
				LogPart::Text(_) => None
			})
		};
		let condition_error = condition
			.as_deref()
			.and_then(|condition| Self::expression_error(&proc, condition))
			.map(|e| format!("the condition doesn't compile: {}", e));
		if let Some(reason) = condition_error.or_else(log_error) {
			self.send_or_disconnect(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				// Re-setting replaces the condition and resets hit counts.
				// Preserve error-report suppression only when the condition is
				// unchanged.
				let key = (proc.id, instruction.offset as u16);
				let claims = self.breakpoints.entry(key).or_default();
				let told = claims.get(&(client, placed)).is_some_and(|old| old.told && old.condition == condition);
				claims.insert((client, placed), Claim {
					condition,
					hit,
					hits: 0,
					log,
					skipped: 0,
					told
				});

				// The client now knows the remapped offset; discard its old
				// alias.
				let offset = instruction.offset;
				self.moved_breakpoints
					.retain(|&(id, from, who), &mut at| id != proc.id || who != (client, placed) || (at != offset && from != offset));

				// As the proc itself names it, whatever the client called it
				instruction.proc = unnamed::proc_ref(&proc);
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at: instruction, file }
				});
			}

			Err(_) => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	/// Compile breakpoint conditions and message expressions to detect syntax
	/// errors. Resolve names later in the frame that hits the breakpoint.
	fn expression_error(proc: &Proc, condition: &str) -> Option<String> {
		let mut names = vec![".".to_owned(), "usr".to_owned(), "src".to_owned()];
		names.extend(proc.parameter_names().iter().map(String::from));
		names.extend(proc.local_names().iter().map(String::from));
		let names: Vec<&str> = names.iter().map(String::as_str).collect();
		let condition = match crate::macros::expand(condition) {
			Ok(condition) => condition,
			Err(e) => return Some(e)
		};
		dmasm::compiler::compile_expr_for(&condition, &names, crate::world_byond())
			.err()
			.map(|e| e.to_string())
	}

	/// Interpolate a logpoint message in the hit frame. Text values are
	/// unquoted.
	fn log_message(&mut self, parts: &[LogPart]) -> String {
		let mut message = String::new();
		for part in parts {
			match part {
				LogPart::Text(text) => message.push_str(text),
				LogPart::Expr(expression) => {
					match self.eval_expr_plain(Some(0), expression) {
						Ok(value) => message.push_str(&value.as_string().unwrap_or_else(|_| Self::stringify(&value))),
						Err(error) => message.push_str(&format!("{{{}: {}}}", expression, error))
					}
					self.state.as_mut().unwrap().invalidate_stacks();
				}
			}
		}
		message
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef, exactly: bool) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.current) else {
			self.send_or_disconnect(Response::BreakpointUnset { success: false, skipped: 0 });
			return;
		};

		// It might be one a reload moved, known to the client by where it was
		let placed = Placed::of(instruction.offset, exactly);
		let asked = if exactly {
			instruction.offset
		} else {
			Self::breakpoint_offset(&proc, instruction.offset)
		};
		let who = (client, placed);
		let offset = self.moved_breakpoints.get(&(proc.id, asked, who)).copied().unwrap_or(asked);

		// The client doesn't know it by any other place now
		self.moved_breakpoints
			.retain(|(id, _, other), at| *id != proc.id || *at != offset || *other != who);

		let key = (proc.id, offset as u16);
		let claims = self.breakpoints.entry(key).or_default();
		let had = claims.remove(&who);
		let skipped = had.as_ref().map_or(0, |claim| claim.skipped);
		let success = if claims.is_empty() {
			// Nobody else wants it
			self.breakpoints.remove(&key);
			unhook_instruction(&proc, offset).is_ok()
		} else {
			had.is_some()
		};
		self.send_or_disconnect(Response::BreakpointUnset { success, skipped });
	}

	fn handle_stacks(&mut self, sleeping: Option<u32>) {
		let sleeping = sleeping.unwrap_or(DEFAULT_SLEEPING_STACKS) as usize;
		if let Some(state) = &mut self.state {
			// Allow for the hidden keepalive stack.
			state.read_sleeping(sleeping.saturating_add(1));
		}
		let mut asleep = 0;
		let stacks = match &self.state {
			Some(state) => {
				// Exclude sleeping helper stacks from the count and results.
				let instances = debug::CallStacks::suspended_instances();
				asleep = instances.len() - instances.into_iter().filter(|&instance| Self::only_runs_spawns(instance)).count();
				let suspended = state
					.suspended()
					.iter()
					.filter(|stack| !stack.iter().all(|frame| crate::launched::runs_spawns(&frame.proc)))
					.take(sleeping);
				let listed = std::iter::once(state.active()).chain(suspended);
				listed
					.enumerate()
					.filter(|(_, stack)| !stack.is_empty())
					.map(|(position, stack)| {
						// A sleeping stack's innermost frame is the one asleep
						let (file, line) = self.frame_place(&stack[0], position != 0);
						Stack {
							id: self.thread_of(stack),
							name: unnamed::proc_name(&stack[0].proc),
							file,
							line,
							sleeping: position != 0
						}
					})
					.collect()
			}

			None => vec![]
		};

		self.send_or_disconnect(Response::Stacks {
			stacks,
			sleeping: asleep as u32
		});
	}

	fn handle_stack_frames(&mut self, thread: u32, start_frame: Option<u32>, count: Option<u32>) {
		let position = self.stack_position(thread);
		let response = match position.and_then(|position| self.get_stack(position)) {
			Some(stack) => {
				let stack_id = position.unwrap_or(0);
				let thread = self.thread_of(stack);
				let stack = Self::shown(stack);
				let start_frame = start_frame.unwrap_or(0);
				let end_frame = start_frame.saturating_add(count.unwrap_or(stack.len() as u32));

				let start_frame = start_frame as usize;
				let end_frame = end_frame as usize;

				let mut frames = vec![];

				for i in start_frame..end_frame {
					if i >= stack.len() {
						break;
					}

					let id = match &self.state {
						Some(state) if stack_id != 0 => state.sleeping_frame_id(SleepingFrame {
							thread,
							depth: i,
							instance: stack[i].instance as usize
						}),
						_ => i as u32
					};
					// A sleeping stack's innermost frame is the one asleep
					frames.push(self.frame_info(&stack[i], id, stack_id != 0 && i == 0));
				}

				Response::StackFrames {
					frames,
					total_count: stack.len() as u32
				}
			}

			None => {
				if self.state.is_none() {
					self.notify("received StackFrames request when not paused");
				}
				Response::StackFrames {
					frames: vec![],
					total_count: 0
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_scopes(&mut self, frame_id: u32) {
		let response = match &self.state {
			Some(state) => Response::Scopes {
				arguments: Some(state.get_ref(Variables::Arguments { frame: frame_id })),
				locals: Some(state.get_ref(Variables::Locals { frame: frame_id })),
				globals: Some(state.get_ref(Variables::Globals))
			},
			None => Response::Scopes {
				arguments: None,
				locals: None,
				globals: None
			}
		};

		self.send_or_disconnect(response);
	}

	/// Expand a previously issued variable reference.
	fn list_variables(&mut self, container: Variables, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		match container {
			Variables::Arguments { frame } => Ok(self.get_args(frame)),
			Variables::Locals { frame } => Ok(self.get_locals(frame)),
			Variables::Globals => self.globals_to_variables(false),
			Variables::Builtins => self.globals_to_variables(true),
			Variables::ObjectVars(value, expr) => self.object_to_variables(&value, expr.as_deref()),
			Variables::ListContents(value, expr) => self.list_to_variables(&value, expr.as_deref(), start, count),
			Variables::ListPair { key, value, entry, .. } => Ok(vec![
				self.value_to_variable("key".to_owned(), &key, None),
				self.value_to_variable("value".to_owned(), &value, entry),
			]),
			Variables::Pointee(pointer, expr) => Ok(match auxtools::pointer::read(&pointer) {
				Some(named) => vec![self.value_to_variable("*".to_owned(), &named, expr.map(|pointer| format!("*{}", pointer)))],
				None => vec![]
			})
		}
	}

	/// Stale or unknown variable references have no children.
	fn handle_variables(&mut self, vars: VariablesRef, start: Option<u32>, count: Option<u32>) {
		let listed = match self.state.as_ref().and_then(|state| state.get_variables(vars)) {
			Some(container) => self.list_variables(container, start, count),
			None => Ok(vec![])
		};

		let vars = listed.unwrap_or_else(|e| {
			self.notify(format!("Couldn't list variables: {}", e.message));
			vec![]
		});
		self.send_or_disconnect(Response::Variables { vars });
	}

	/// The types an object is of, its own first: `/mob/player`, `/mob`,
	/// `/atom/movable`, `/atom`, `/datum`
	fn types_of(value: &Value) -> Vec<String> {
		let mut types = vec![];
		let mut path = value.get(byond_string!("type")).ok();
		// (a type's parent isn't always the path without its last part)
		while let Some(this) = path.filter(|path| path.raw.tag != ValueTag::Null) {
			let name = Self::stringify(&this);
			if !name.starts_with('/') || types.contains(&name) || types.len() > 64 {
				break;
			}
			types.push(name);
			path = this.get(byond_string!("parent_type")).ok();
		}
		types
	}

	/// Collect value members as (name, is_proc) entries.
	fn members_of(value: &Value, found: &mut BTreeMap<String, bool>) {
		if let Some(fields) = Self::primitive_fields(value) {
			found.extend(fields.iter().map(|&name| (name.to_owned(), false)));
		} else if let Ok(vars) = value.get(byond_string!("vars")).and_then(|vars| List::from_value(&vars)) {
			for i in 1..=vars.len() {
				if let Ok(name) = vars.get(i).and_then(|name| name.as_string()) {
					found.insert(name, false);
				}
			}
		}

		let types = Self::types_of(value);
		if types.is_empty() {
			return;
		}
		Self::each_proc(|owner, name| {
			if types.iter().any(|path| path == owner) {
				found.entry(name.to_owned()).or_insert(true);
			}
		});
	}

	/// Calls `each` with every proc's type (`""` for a global proc) and name
	fn each_proc(mut each: impl FnMut(&str, &str)) {
		for id in 0..auxtools::runtime_tables::proc_count() {
			let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
				continue;
			};
			// The debugger's own, and a type's init code, aren't for calling
			let Some((owner, name)) = proc.path.rsplit_once('/') else {
				continue;
			};
			if name.starts_with("auxtools_") || name.ends_with(')') || name.is_empty() {
				continue;
			}
			each(owner, name);
		}
	}

	/// The names that could follow what's typed (see `Request::Complete`)
	fn complete(&mut self, frame_id: Option<u32>, base: Option<&str>, prefix: &str) -> Vec<Completion> {
		let mut found: BTreeMap<String, bool> = BTreeMap::new();
		match base {
			Some(base) => {
				// Its code ran, whether it gave a value or not
				let value = self.eval_expr(frame_id, base);
				if let Some(state) = self.state.as_mut() {
					state.invalidate_stacks();
				}
				let Ok(value) = value else {
					return vec![];
				};
				Self::members_of(&value, &mut found);
			}

			None => {
				// What the frame has, then its src's, then the world's
				let mut src = None;
				if let Some(frame) = frame_id.and_then(|frame_id| self.get_stack_frame(frame_id)) {
					for name in [".", "src", "usr", "args"] {
						found.insert(name.to_owned(), false);
					}
					found.extend(
						frame
							.args
							.iter()
							.filter_map(|(name, _)| name.as_ref())
							.map(|name| (String::from(name), false))
					);
					found.extend(Self::visible_locals(&frame).into_iter().flatten().map(|name| (name, false)));
					src = Some(frame.src.clone()).filter(|src| *src != Value::NULL);
				}
				if let Some(src) = &src {
					Self::members_of(src, &mut found);
				}

				for name in ["world", "global"] {
					found.entry(name.to_owned()).or_insert(false);
				}
				if let Ok(names) = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) }) {
					for i in 1..=names.len() {
						if let Ok(name) = names.get(i).and_then(|name| name.as_string()) {
							found.entry(name).or_insert(false);
						}
					}
				}
				for name in hot_reload::added_globals() {
					found.entry(name).or_insert(false);
				}
				Self::each_proc(|owner, name| {
					if owner.is_empty() {
						found.entry(name.to_owned()).or_insert(true);
					}
				});
			}
		}

		let prefix = prefix.to_lowercase();
		found
			.into_iter()
			.filter(|(name, _)| name.to_lowercase().starts_with(&prefix))
			.take(Self::MOST_COMPLETIONS)
			.map(|(name, proc)| Completion { name, proc })
			.collect()
	}

	/// The entry of a list that a client knows as `[n]`: its place, from 1
	fn entry_number(name: &str) -> Option<u32> {
		name.strip_prefix('[')?.strip_suffix(']')?.parse().ok().filter(|&n| n > 0)
	}

	/// Assign a value to a variable. Report BYOND's write failures as runtime
	/// errors.
	fn write_variable(&mut self, container: &Variables, name: &str, value: &Value) -> Result<(), String> {
		let runtime = |e: Runtime| format!("runtime error: {}", e.message);
		// A frame's slot takes a reference of its own, and lets go of what
		// it had
		unsafe fn put(slot: *mut raw_types::values::Value, value: &Value) {
			unsafe {
				let _ = Value::from_raw_owned(*slot);
				let value = value.clone();
				*slot = value.raw;
				std::mem::forget(value);
			}
		}

		match container {
			Variables::Locals { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				if name == "." {
					unsafe { put((*frame.context).dot_mut(), value) };
					return Ok(());
				}
				let visible = Self::visible_locals(&frame);
				let mut named = visible.iter().enumerate().filter(|(_, local)| local.as_deref() == Some(name));
				match (named.next(), named.next()) {
					(Some((index, _)), None) => {
						unsafe { put((*(*frame.context).locals()).add(index), value) };
						Ok(())
					}
					(Some(_), Some(_)) => Err(format!("the proc has more than one local called {}", name)),
					_ => Err(format!("the frame has no local called {}", name))
				}
			}

			Variables::Arguments { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				match name {
					"src" => unsafe { put(&raw mut (*frame.instance).src, value) },
					"usr" => unsafe { put(&raw mut (*frame.instance).usr, value) },
					_ => {
						// One the proc doesn't declare is known by its number
						// among those
						let unnamed: Option<usize> = name.strip_prefix("undefined argument #").and_then(|n| n.parse().ok());
						let mut without_names = 0;
						let index = frame.args.iter().position(|(arg, _)| match arg {
							Some(arg) => String::from(arg) == name,
							None => {
								without_names += 1;
								unnamed == Some(without_names)
							}
						});
						let index = index.ok_or_else(|| format!("the frame has no argument called {}", name))?;
						unsafe { put((*frame.instance).args().add(index), value) };
					}
				}
				Ok(())
			}

			Variables::Globals => Value::GLOBAL.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),
			Variables::Builtins => Err("BYOND's constants can't be changed".to_owned()),
			Variables::ObjectVars(object, _) => object.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),

			Variables::ListContents(list, _) => {
				let list = List::from_value(list).map_err(runtime)?;
				let number = Self::entry_number(name)
					.filter(|&n| n <= list.len())
					.ok_or_else(|| format!("the list has no entry {}", name))?;
				// An entry with a value for its key is given another value
				if list.is_alist() {
					let (key, _) = list
						.alist_pairs()
						.map_err(runtime)?
						.into_iter()
						.nth(number as usize - 1)
						.ok_or("the list has changed")?;
					return list.set(&key, value).map_err(runtime);
				}
				let key = list.get(number).map_err(runtime)?;
				let has_value = key.raw.tag != ValueTag::Number && list.get(&key).is_ok_and(|of_key| of_key.raw.tag != ValueTag::Null);
				if has_value {
					list.set(&key, value).map_err(runtime)
				} else {
					list.set(number, value).map_err(runtime)
				}
			}

			Variables::ListPair { key, of, .. } => match name {
				"value" => List::from_value(of).map_err(runtime)?.set(key, value).map_err(runtime),
				_ => Err("list keys cannot be edited; remove the entry and add a new one".to_owned())
			},

			Variables::Pointee(..) => Err("assign to the target variable to change the pointed-to value".to_owned())
		}
	}

	/// Evaluate an assignment and return the variable's resulting value.
	fn set_variable(&mut self, vars: VariablesRef, name: &str, value: &str) -> Result<Variable, String> {
		let container = self
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("variable reference expired after execution resumed")?;
		let frame = match &container {
			Variables::Arguments { frame } | Variables::Locals { frame } => *frame,
			_ => 0
		};
		// Its code ran, whether it gave a value or not
		let value = self.eval_expr(Some(frame), value);
		self.state.as_mut().unwrap().invalidate_stacks();
		let value = value?;

		// Report assignment runtime errors to the requester without pausing.
		self.in_eval = true;
		self.eval_error = None;
		let written = self.write_variable(&container, name, &value);
		self.in_eval = false;
		self.refresh_watches();
		self.state.as_mut().unwrap().invalidate_stacks();
		if let Some(error) = self.eval_error.take() {
			return Err(format!("runtime error: {}", error));
		}
		written?;

		// Refresh the cached entry value after assignment.
		if let Variables::ListPair { entry, .. } = &container {
			return Ok(self.value_to_variable(name.to_owned(), &value, entry.clone()));
		}

		// As the world has it now, which isn't always what it was given
		let (start, count) = match &container {
			Variables::ListContents(..) => (Self::entry_number(name).map(|n| n - 1), Some(1)),
			_ => (None, None)
		};
		let listed = self.list_variables(container, start, count).map_err(|e| e.message)?;
		listed
			.into_iter()
			.find(|variable| variable.name == name)
			.ok_or_else(|| format!("{} is gone", name))
	}

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		// How many matches variables can you spot? It could be better...
		let response = match self.app.try_get_matches_from_mut(command.split_ascii_whitespace()) {
			Ok(matches) => {
				match matches.subcommand() {
					Some(("disassemble", matches)) => {
						if let Some(proc) = matches.get_one::<String>("proc").map(String::as_str) {
							// Default id to 0 in the worst way possible
							let id = matches
								.get_one::<String>("id")
								.map(String::as_str)
								.and_then(|x| x.parse::<u32>().ok())
								.unwrap_or(0);

							self.handle_disassemble(proc, id)
						} else if let Some(frame_id) = frame_id {
							if let Some(frame) = self.get_stack_frame(frame_id) {
								let proc = unnamed::proc_ref(&frame.proc);
								self.handle_disassemble(&proc.path, proc.override_id)
							} else {
								"couldn't find stack frame (is execution not paused?)".to_owned()
							}
						} else {
							"no execution frame selected".to_owned()
						}
					}

					Some(("guest_override", matches)) => match matches.get_one::<String>("ckey").map(String::as_str) {
						Some(ckey) => match crate::ckey_override::override_guest_ckey(ckey) {
							Ok(()) => "Success".to_owned(),

							Err(e) => {
								format!("Failed: {:?}", e)
							}
						},

						None => "no ckey provided".to_owned()
					},

					Some(("mem_profiler", matches)) => match matches.subcommand() {
						Some(("begin", matches)) => match matches.get_one::<String>("path").map(String::as_str) {
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

	/// Evaluate in the selected frame or global scope, returning a value or
	/// error.
	fn eval_expr(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		self.eval_expr_plain(frame_id, command).map_err(Self::unknown_name)
	}

	/// Evaluate a logpoint expression without the missing-macros hint,
	/// which would otherwise be repeated within every log message.
	fn eval_expr_plain(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		// Assembling the expression runs DM code of its own to look up
		// typepaths and resources, so it counts as eval code too. Otherwise the
		// instruction hook would handle the client's next request in the middle
		// of this one, and a runtime would pause inside the pause we're in.
		self.in_eval = true;
		self.eval_error = None;
		self.eval_callee_errors.clear();
		self.eval_callee_more = 0;
		self.eval_warnings.clear();
		self.eval_ran = std::ptr::null_mut();

		let result = self.compile_and_run_expr(frame_id, command);
		self.eval_given.clear();
		self.eval_src = None;
		// Whatever it changed isn't the world's doing
		self.refresh_watches();

		self.in_eval = false;

		// Its code failed: whatever it returned isn't the expression's value
		match self.eval_error.take() {
			Some(error) => Err(format!("runtime error: {}", error)),
			None => result
		}
	}

	/// Suggest unavailable source macros for unresolved uppercase names.
	fn unknown_name(error: String) -> String {
		let name = match (error.strip_prefix("undefined var: "), error.strip_prefix("proc not found: ")) {
			(Some(name), _) => name,
			(_, Some(path)) => path.rsplit('/').next().unwrap_or(path),
			_ => return error
		};
		// Uppercase unresolved names are likely macros; avoid the hint
		// for ordinary misspellings.
		let capitals = name.chars().any(|c| c.is_ascii_uppercase()) && !name.chars().any(|c| c.is_ascii_lowercase());
		if !capitals || crate::macros::given() {
			return error;
		}
		format!(
			"{} (if that's a #define: source macros unavailable until the project is parsed for hot reload)",
			error
		)
	}

	/// Format an assembly error for the user.
	fn assemble_error(error: dmasm::assembler::AssembleError) -> String {
		use dmasm::assembler::AssembleError;
		let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
		match error {
			AssembleError::ProcNotFound(path) => format!("proc not found: {}", path),
			AssembleError::VariableNameNotFound(name) => format!("variable not found: {}", text(&name)),
			AssembleError::StringNotFound(string) => format!("string not found: {}", text(&string)),
			AssembleError::TypeNotFound(path) => format!("there's no type {}", path),
			AssembleError::ResourceNotFound(path) => format!("there's no file '{}'", path),
			other => format!("{:?}", other)
		}
	}

	fn compile_and_run_expr(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		enum ArgType {
			Dot,
			Usr,
			Src,
			Arg(u32),
			Local(u32),
			/// The evaluation owns these references; do not restore them to the
			/// frame.
			Given
		}

		// Expand source macros and expose globals through the synthetic
		// parameter.
		let command = &*crate::macros::expand(command)?;

		let globals = unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) };
		let mut src = None;
		let given_src = self.eval_src.take();

		let (ctx, instance, mut args) = match frame_id {
			// Global context, or an object's (`Request::EvalOn`)
			None => {
				let mut args = vec![];
				if let Some(object) = given_src {
					src = Some(object.clone());
					args.push(("src".to_owned(), object, ArgType::Given));
				}
				(std::ptr::null_mut(), std::ptr::null_mut(), args)
			}

			Some(frame_id) => {
				let Some(frame) = self.get_stack_frame(frame_id) else {
					return Err(format!("there's no frame {} (the world isn't paused, or has carried on since)", frame_id));
				};

				// Pass the frame's `args` so evaluation sees the paused proc's
				// arguments.
				let frame_args = List::new();
				for (_, value) in &frame.args {
					frame_args.append(value);
				}
				if frame.src != Value::NULL {
					src = Some(frame.src.clone());
				}

				let mut args = vec![
					("args".to_owned(), Value::from(frame_args), ArgType::Given),
					(".".to_owned(), frame.dot.clone(), ArgType::Dot),
					("usr".to_owned(), frame.usr.clone(), ArgType::Usr),
					("src".to_owned(), frame.src.clone(), ArgType::Src),
				];

				for (idx, arg) in frame.args.iter().enumerate() {
					if let Some(name) = &arg.0 {
						args.push((name.into(), arg.1.clone(), ArgType::Arg(idx as u32)));
					}
				}

				// Exclude hidden duplicate locals from evaluation names.
				for (idx, (local, name)) in frame.locals.iter().zip(Self::visible_locals(&frame)).enumerate() {
					if let Some(name) = name {
						args.push((name, local.1.clone(), ArgType::Local(idx as u32)));
					}
				}

				(frame.context, frame.instance, args)
			}
		};

		args.push((GLOBALS.to_owned(), globals, ArgType::Given));
		// Bind supplied names (value, old, __watched, __key) before frame
		// names.
		for (name, value) in std::mem::take(&mut self.eval_given).into_iter().rev() {
			args.insert(0, (name, value, ArgType::Given));
		}
		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		// Resolve names as in the proc: frame variables, then src members,
		// then globals. Bare calls use src procs when available.
		let has_var = |object: &Value, name: &str| StringRef::new(name).is_ok_and(|name| object.get(name).is_ok());
		let scope = dmasm::compiler::ExprScope {
			has_src_var: &|name| src.as_ref().is_some_and(|src| has_var(src, name)),
			has_src_proc: &|name| src.as_ref().is_some_and(|src| crate::assemble_env::AssembleEnv::has_proc(src, name)),
			// One a hot reload added isn't on the world's list of them
			global: &|name| {
				if has_var(&Value::GLOBAL, name) {
					Some(dmasm::compiler::ExprGlobal::Listed)
				} else {
					hot_reload::added_global(name).map(dmasm::compiler::ExprGlobal::Id)
				}
			},
			globals: GLOBALS,
			proc_ref: &crate::assemble_env::type_proc,
			proc_path: &crate::assemble_env::proc_path
		};
		// Include src's type in the cache key because it affects bare-name
		// resolution.
		let key = EvalCodeKey {
			generation: hot_reload::code_generation(),
			command: command.to_owned(),
			names: arg_names.iter().map(|name| (*name).to_owned()).collect(),
			src_type: src.as_ref().and_then(Self::kind)
		};
		let compiled = match self.eval_code.get(&key) {
			Some(compiled) => compiled.clone(),
			None => {
				let compiled = Self::compile_eval(command, &arg_names, &scope).and_then(|(code, locals, warnings)| {
					let assembly = dmasm::assembler::assemble(&code, &mut crate::assemble_env::AssembleEnv).map_err(Self::assemble_error)?;
					Ok((assembly, locals, warnings))
				});
				if self.eval_code.len() >= EVAL_CODE_KEPT {
					self.eval_code.clear();
				}
				self.eval_code.insert(key, compiled.clone());
				compiled
			}
		};
		let (assembly, locals, warnings) = compiled?;
		self.eval_warnings = warnings;
		// To be run by the world's loop, as its own code (see `launched`)
		if let Some(number) = self.spawning.take() {
			self.spawns.push_back(Spawn {
				number,
				bytecode: assembly,
				locals,
				args: arg_values.into_iter().cloned().collect()
			});
			if let Some(inbox) = self.spawn_inbox.as_ref().and_then(|inbox| List::from_value(inbox).ok()) {
				inbox.append(Value::from(number as f32));
			}
			return Ok(Value::NULL);
		}
		let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no /proc/auxtools_expr_stub to evaluate expressions in")?;

		proc.set_bytecode(assembly);

		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let result = Self::call_with_locals(&proc, &locals, &arg_values)?;
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) != ends {
			// Evaluation rebooted or ended the world. Do not access its old
			// server, frames or values.
			std::mem::forget(result);
			std::mem::forget(args);
			std::panic::resume_unwind(Box::new(crate::WorldEnded));
		}

		match result {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					// Evaluation may end a sleeping frame by deleting its src.
					// Do not restore variables into a frame that no longer
					// exists.
					if frame_id.is_some() && !Self::frame_is_live(ctx) {
						return Err("evaluation ended the selected frame".to_owned());
					}

					// The rest are the potentially mutated parameters. We need
					// to commit them to the function that
					// called us. TODO: This sucks, obviously.
					let len = list.len();
					for i in 2..=len {
						let value = list.get(i).unwrap();
						let slot = &args[i as usize - 2].2;

						unsafe {
							match slot {
								ArgType::Dot => {
									let dot = (*ctx).dot_mut();
									let _ = Value::from_raw_owned(*dot);
									*dot = value.raw;
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
									let locals = *(*ctx).locals();
									let local = locals.add(*idx as usize);
									let _ = Value::from_raw_owned(*local);
									(*local) = value.raw;
								}
								ArgType::Given => continue
							}
						}

						std::mem::forget(value);
					}

					Ok(list.get(1).unwrap())
				} else if !self.eval_ran.is_null() && background_known() && unsafe { *(*self.eval_ran).background() } != 0 {
					// Background procs sleep periodically, including those
					// moved there by world.loop_checks after about a million
					// iterations.
					Err(
						"evaluation aborted: BYOND moved the code to the background, so its result is unavailable. This can happen after about a \
						 million loop iterations (world.loop_checks), or with `set background = 1`. The code is still running. Use `eval --spawn \
						 --wait` to wait for a result"
							.to_owned()
					)
				} else if !background_known() {
					Err(
						"evaluation aborted because the code slept or BYOND moved it to the background, so its result is unavailable. BYOND does \
						 that after about a million loop iterations (world.loop_checks), or with `set background = 1`. Use `eval --spawn --wait` to \
						 wait for a result"
							.to_owned()
					)
				} else {
					// The stub returns a list unless it's cut short, and a
					// runtime error is told apart above
					Err("evaluation aborted because the code slept".to_owned())
				}
			}

			// A runtime error, which the world's runtime hook has the text of
			Err(Runtime { message }) => Err(format!("runtime error: {}", message))
		}
	}

	/// Compile an expression or proc body and return its local names and
	/// compatibility warnings.
	fn compile_eval(
		command: &str,
		arg_names: &[&str],
		scope: &dmasm::compiler::ExprScope
	) -> Result<(Vec<dmasm::Node>, Vec<String>, Vec<String>), String> {
		let byond = crate::world_byond();
		if dmasm::compiler::is_body(command) {
			let body = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).map_err(|e| e.to_string())?;
			return Ok((body.nodes, body.locals, body.warnings));
		}
		match dmasm::compiler::compile_expr_warned(command, arg_names, Some(scope), byond) {
			Ok((expr, warnings)) => Ok((expr, vec![], warnings)),
			// For body-shaped input, retain the body parser's line diagnostics.
			Err(_) if command.trim().contains(['\n', ';']) => {
				let error = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).err();
				Err(error.map_or_else(|| "invalid expression or proc body".to_owned(), |e| e.to_string()))
			}
			Err(e) => Err(e.to_string())
		}
	}

	/// Temporarily size the stub's locals table for this call. Sleeping
	/// frames retain the locals allocated when they started.
	fn call_with_locals(proc: &Proc, locals: &[String], args: &[&Value]) -> Result<Result<Value, Runtime>, String> {
		if locals.is_empty() {
			return Ok(proc.call(args));
		}
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let none = runtime_tables::proc_entry(proc.id).ok_or("couldn't read the stub's proc entry")?.0[ProcEntryWords::LOCALS];
		let table = hot_reload::locals_table(locals, none)?;
		runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::LOCALS, table);
		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let result = proc.call(args);
		// (a world the code ended has procs of its own by now)
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) == ends {
			runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::LOCALS, none);
		}
		Ok(result)
	}

	/// Whether a frame is still on one of the world's stacks, running or
	/// asleep
	fn frame_is_live(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		let on = |mut at: *mut raw_types::procs::ExecutionContext| {
			while !at.is_null() {
				if at == ctx {
					return true;
				}
				at = unsafe { *(*at).parent_context() };
			}
			false
		};
		on(unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT })
			|| debug::CallStacks::suspended_instances()
				.into_iter()
				.any(|instance| on(unsafe { (*instance).context }))
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str) {
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.send_or_disconnect(Response::Eval(Ok(EvalResponse {
				value: response,
				variables: None,
				kind: None,
				entries: None
			})));
			return;
		}

		// Retain access expressions only for single expressions. Expanding
		// a body's result must not run the body again.
		let reached = (!dmasm::compiler::is_body(command)).then(|| format!("({})", command));
		let response = self.eval_expr(frame_id, command).map(|result| EvalResponse {
			value: Self::stringify(&result),
			variables: self.value_to_variables_ref(&result, reached),
			kind: Self::kind(&result),
			entries: Self::entries(&result)
		});
		self.send_callee_errors();
		self.send_or_disconnect(Response::Eval(response));
	}

	/// What `Request::EvalOn` asks for: an expression with `src` an object
	fn handle_eval_on(&mut self, object: &str, expression: &str, depth: Option<u32>) {
		let value = self.object_of(object).and_then(|object| {
			self.eval_src = Some(object);
			self.eval_expr(None, expression)
		});
		let response = match depth {
			Some(depth) => Response::Render(value.map(|value| Self::render(&value, depth))),
			// What's inside is opened by itself: the expression means
			// nothing without its object
			None => Response::Eval(value.map(|result| EvalResponse {
				value: Self::stringify(&result),
				variables: self.value_to_variables_ref(&result, None),
				kind: Self::kind(&result),
				entries: Self::entries(&result)
			}))
		};
		self.send_callee_errors();
		self.send_or_disconnect(response);
	}

	/// Compile now and queue execution for the next world tick.
	fn handle_spawn(&mut self, expression: &str, object: Option<&str>) {
		let result = self.spawn(expression, object);
		self.send_or_disconnect(Response::Spawned(result));
	}

	fn spawn(&mut self, expression: &str, object: Option<&str>) -> Result<u64, String> {
		crate::launched::can_spawn()?;
		let object = object.map(|object| self.object_of(object)).transpose()?;
		let number = NEXT_SPAWN.fetch_add(1, Ordering::Relaxed);
		self.eval_src = object;
		self.spawning = Some(number);
		let compiled = self.eval_expr(None, expression);
		self.spawning = None;
		compiled?;
		self.spawned.insert(number, (expression.to_owned(), None));
		Ok(number)
	}

	/// The list the world's loop looks at, which has an entry when there's
	/// spawned code to run
	pub fn spawn_inbox(&mut self) -> Value {
		let inbox = Value::from(List::new());
		self.spawn_inbox = Some(inbox.clone());
		inbox
	}

	/// Install the next spawned expression in the stub and return
	/// `list(number, list(arguments))`, or null if none is queued.
	/// Active calls retain their own bytecode.
	pub fn next_spawn(&mut self) -> DMResult {
		let Some(spawn) = self.spawns.pop_front() else {
			return Ok(Value::NULL);
		};
		let Some(stub) = Proc::find("/proc/auxtools_spawn_stub") else {
			self.spawn_failed(spawn.number, "the world has no /proc/auxtools_spawn_stub to run it in".to_owned());
			return Ok(Value::NULL);
		};
		// A body's own vars: the stub has room for them from this call on.
		// Code that's asleep has its own already.
		if let Err(error) = self.set_stub_locals(&stub, &spawn.locals) {
			self.spawn_failed(spawn.number, error);
			return self.next_spawn();
		}
		stub.set_bytecode(spawn.bytecode);
		let args = List::new();
		for arg in &spawn.args {
			args.append(arg);
		}
		let next = List::new();
		next.append(Value::from(spawn.number as f32));
		next.append(Value::from(args));
		Ok(Value::from(next))
	}

	/// Gives the spawn stub the locals its next code declares, or none
	fn set_stub_locals(&mut self, stub: &Proc, locals: &[String]) -> Result<(), String> {
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let now = runtime_tables::proc_entry(stub.id).ok_or("couldn't read the spawn stub's proc entry")?.0[ProcEntryWords::LOCALS];
		// What this world's stub was compiled with, which is no locals at all
		let none = *self.spawn_stub_locals.get_or_insert(now);
		let table = hot_reload::locals_table(locals, none)?;
		if table != now {
			runtime_tables::set_proc_entry_word(stub.id, ProcEntryWords::LOCALS, table);
		}
		Ok(())
	}

	/// Complete spawned code with its result list (value first), or null on
	/// failure.
	pub fn spawn_done(&mut self, number: u64, returned: &Value) {
		let Some((expression, error)) = self.spawned.remove(&number) else {
			return;
		};
		let result = match (error, returned.as_list()) {
			(Some(error), _) => Err(error),
			(None, Ok(list)) => {
				let value = list.get(1).unwrap_or(Value::NULL);
				Ok(SpawnedValue {
					text: Self::stringify(&value),
					rendered: Self::render(&value, 1)
				})
			}
			(None, Err(_)) => Err("evaluation ended without a value".to_owned())
		};
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result
			},
			None
		);
	}

	/// Journal a spawned evaluation that could not start.
	fn spawn_failed(&mut self, number: u64, why: String) {
		let expression = self.spawned.remove(&number).map(|(expression, _)| expression).unwrap_or_default();
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result: Err(why)
			},
			None
		);
	}

	/// Resolve a live object reference such as `[0x2000001]`, with optional
	/// brackets.
	fn object_of(&mut self, reference: &str) -> Result<Value, String> {
		let text = reference.trim();
		let inner = text.strip_prefix('[').and_then(|text| text.strip_suffix(']')).unwrap_or(text);
		let id = inner
			.strip_prefix("0x")
			.or_else(|| inner.strip_prefix("0X"))
			.and_then(|hex| u32::from_str_radix(hex, 16).ok())
			.ok_or_else(|| format!("invalid object reference {}; expected a ref() value such as [0x2000001]", reference))?;
		let reference = format!("[0x{:x}]", id);
		// The world's own lookup knows what's still there. (A `[` in a text
		// starts an expression in it.)
		let found = self.eval_expr(None, &format!("locate(\"\\{}\")", reference))?;
		if found == Value::NULL {
			return Err(format!("object not found: {}", reference));
		}
		Ok(found)
	}

	/// Claim hot reload ownership unless another connected client holds it.
	fn claim_reloads(&mut self, who: String) -> Result<(), String> {
		let id = self.current.ok_or("no client")?;
		match &self.reloader {
			Some((holder, theirs)) if *holder != id && self.clients.contains_key(holder) => Err(format!(
				"{} already hot reloads this world; two watchers would apply each change twice",
				theirs
			)),
			_ => {
				self.reloader = Some((id, who));
				Ok(())
			}
		}
	}

	fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		// Reject reloads from clients other than the connected owner.
		if let Some((holder, who)) = &self.reloader {
			if Some(*holder) != self.current && self.clients.contains_key(holder) {
				let refused = HotReloadResult {
					applied: false,
					errors: vec![format!("{} already hot reloads this world", who)],
					warnings: vec![],
					skipped: vec![]
				};
				self.send_or_disconnect(Response::HotReload(refused));
				return;
			}
		}
		let result = self.hot_reload(request);
		self.send_or_disconnect(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		// Assembly may evaluate DM to resolve paths and resources; treat
		// it as debugger evaluation.
		self.in_eval = true;
		self.eval_error = None;

		let mut warnings = vec![];
		let mut skipped = vec![];
		// New types come first, so the rest can be prepared against them
		let prepared = hot_reload::add_types(&request.new_types).and_then(|added| {
			hot_reload::prepare(request, &mut warnings, &mut skipped).map_err(|mut errors| {
				if added > 0 {
					errors.push(format!("{} new types remain allocated but unused", added));
				}
				errors
			})
		});
		let result = match prepared {
			Ok(prepared) => {
				hot_reload::apply(prepared, &mut self.breakpoints, &mut self.moved_breakpoints, &mut warnings);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings,
					skipped
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings,
				skipped
			}
		};

		self.in_eval = false;
		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime error during hot reload: {}", err));
		}

		// Every client hears what changed
		if result.applied && !request.is_empty() {
			let skipped_count = request.removed_procs.iter().filter(|r| result.skipped.contains(&r.proc.path)).count();
			self.broadcast(Response::CodeChanged, self.current);
			self.notify(format!(
				"Hot reload: new code for {} procs, new settings for {}, {} new procs, {} unlinked, {} new vars, {} new var defaults, {} new init \
				 procs, {} moved",
				request.procs.len(),
				request.settings.len(),
				request.new_procs.len(),
				request.removed_procs.len() - skipped_count,
				request.new_vars.len(),
				request.var_defaults.len(),
				request.init_procs.len(),
				request.line_moves.len()
			));
		}

		result
	}

	fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match unnamed::find(path, id) {
			Some(proc) => {
				// The live bytecode has our breakpoints patched into it, which
				// nobody wants to read
				let bytecode = original_bytecode(&proc);

				let mut env = crate::DisassembleEnv;
				let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
				let dism = dmasm::format_disassembly(&nodes, None);

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
			Request::CatchRuntimes { uncaught, caught } => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.uncaught = uncaught;
					client.caught = caught;
				}
			}
			Request::BreakpointSet {
				instruction,
				condition,
				hit,
				log
			} => self.handle_breakpoint_set(instruction, condition, hit, log, false),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction, false),
			Request::Stacks { sleeping } => self.handle_stacks(sleeping),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars, start, count } => self.handle_variables(vars, start, count),
			Request::Complete { frame_id, base, prefix } => {
				let names = self.complete(frame_id, base.as_deref(), &prefix);
				self.send_or_disconnect(Response::Complete(names));
			}
			Request::SetVariable { vars, name, value } => {
				let result = self.set_variable(vars, &name, &value);
				self.send_or_disconnect(Response::SetVariable(result));
			}
			Request::Eval { frame_id, command, .. } => self.handle_eval(frame_id, &command),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::Offset { proc, line } => {
				self.send_or_disconnect(Response::Offset {
					offset: self.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.send_or_disconnect(Response::Ack);
				return true;
			}

			Request::ByondVersion => {
				let (version, build) = auxtools::version::get();
				self.send_or_disconnect(Response::ByondVersion { version, build });
			}

			Request::HotReload(request) => self.handle_hot_reload(&request),
			Request::Reloader { who } => {
				let claimed = self.claim_reloads(who);
				self.send_or_disconnect(Response::Reloader(claimed));
			}

			Request::Status => {
				let paused = self.paused.clone();
				self.send_or_disconnect(Response::Status { paused });
			}

			Request::HoldPause => {
				self.hold = self.current;
				self.hold_max = self.current.and_then(|id| self.clients.get(&id)).and_then(|client| client.hold_max);
				self.send_or_disconnect(Response::Ack);
			}

			Request::Locate { file, line } => {
				let instructions = self.locate(&file, line);
				self.send_or_disconnect(Response::Locate { instructions });
			}

			Request::LinesWithCode { file, start, end } => {
				let lines = self.lines_with_code(&file, start, end);
				self.send_or_disconnect(Response::LinesWithCode { lines });
			}

			Request::WatchInfo { place } => {
				let info = self.watch_info(&place);
				self.send_or_disconnect(Response::WatchInfo(info));
			}

			Request::WatchSet { place, condition, hit } => {
				let set = self.watch_set(&place, condition, hit);
				self.send_or_disconnect(Response::WatchSet(set));
			}

			Request::WatchUnset { id } => {
				self.watch_unset(id);
				self.send_or_disconnect(Response::Ack);
			}

			Request::Files => {
				let mut files: Vec<String> = self.file_index().keys().cloned().collect();
				files.sort();
				self.send_or_disconnect(Response::Files { files });
			}

			Request::WorldInfo => {
				let (args, cwd) = crate::launched::command_line();
				self.send_or_disconnect(Response::WorldInfo { args, cwd });
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.send_or_disconnect(Response::StdDef(stddef));
			}

			Request::Watcher => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.debugs = false;
				}
				self.send_or_disconnect(Response::Ack);
			}

			Request::Disassemble { code } => {
				let disassembly = self.disassemble(code);
				self.send_or_disconnect(Response::Disassemble(disassembly));
			}

			Request::InstructionBreakpointSet {
				instruction,
				condition,
				hit,
				log
			} => self.handle_breakpoint_set(instruction, condition, hit, log, true),
			Request::InstructionBreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction, true),

			Request::Journal { after, kinds, limit, follow } => self.handle_journal(after, kinds, limit, follow),
			Request::JournalAdd(event) => self.journal(event, self.current),
			Request::HoldMax { seconds } => {
				let hold_max = seconds
					.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
					.map(Duration::from_secs_f64);
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.hold_max = hold_max;
				}
				self.send_or_disconnect(Response::Ack);
			}
			Request::Uptime => {
				let (loaded, up, loads, last_reload) = journal::uptime();
				self.send_or_disconnect(Response::Uptime {
					loaded,
					up,
					world_time: journal::world_time(),
					loads,
					last_reload,
					started: !self.waiting_to_start
				});
			}
			Request::ProcInfo { proc } => {
				let info = Self::proc_info(&proc);
				self.send_or_disconnect(Response::ProcInfo(info));
			}

			Request::Render { frame_id, expression, depth } => self.handle_render(frame_id, &expression, depth),
			Request::RenderVariables { vars, depth } => self.handle_render_variables(vars, depth),
			Request::EvalOn { object, expression, depth } => self.handle_eval_on(&object, &expression, depth),
			Request::Defines(macros) => {
				crate::macros::set(macros);
				hot_reload::code_changed();
			}
			Request::Spawn { expression, object } => self.handle_spawn(&expression, object.as_deref()),

			// The following requests are special cases and handled outside of
			// this function
			Request::Configured | Request::Continue { .. } => {
				self.send_or_disconnect(Response::Ack);
			}
		}

		false
	}

	/// Process network events and return the next request, setting `current`
	/// to its sender. `None` means the wait expired or, with `WhileClients`,
	/// the last debugging client disconnected.
	fn recv_request(&mut self, wait: Wait) -> Option<Request> {
		loop {
			let event = match wait {
				Wait::No => self.events.try_recv().ok()?,
				Wait::Until(Some(until)) => self.events.recv_timeout(until.saturating_duration_since(Instant::now())).ok()?,
				_ => self.events.recv().ok()?
			};

			match event {
				Event::Connected(id, connection) => {
					let (outbox, writer) = Self::writer(connection.clone());
					self.clients.insert(id, Client {
						uncaught: None,
						caught: None,
						debugs: true,
						follow: None,
						outbox,
						writer: Some(writer),
						hold_max: None
					});
					if let Wait::Until(_) = wait {
						return None;
					}
				}

				Event::Disconnected(id) => {
					self.remove_client(id);

					if (wait == Wait::WhileClients && !self.has_debuggers()) || matches!(wait, Wait::Until(_)) {
						return None;
					}
				}

				// Sent by a client we've since given up on
				Event::Request(id, _) if !self.clients.contains_key(&id) => waker::request_taken(),

				Event::Request(id, request) => {
					waker::request_taken();
					self.current = Some(id);
					return Some(request);
				}
			}
		}
	}

	/// Whether a client that debugs is connected
	fn has_debuggers(&self) -> bool {
		self.clients.values().any(|client| client.debugs)
	}

	/// Send to connected clients, or to world output if none are connected.
	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		if self.clients.is_empty() {
			eprintln!("Debug Server: {}", message);
		}

		self.broadcast(Response::Notification { message }, None);
	}

	/// Something in here panicked. We don't know what state that left the
	/// session in, so the debug-client gets dropped and can connect again.
	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.in_eval = false;
		// A panic mid-pause skips the cleanup at the end of handle_breakpoint.
		// The frames in here point at procs that are about to carry on running.
		self.state = None;
		self.disconnect_all();
	}

	/// Evaluate every client's claim at this breakpoint. Count hits only
	/// when its condition holds, apply its hit filter, and emit log messages.
	/// Return whether any claim requests a pause.
	fn breakpoint_hit(&mut self, key: (raw_types::procs::ProcId, u16)) -> bool {
		let (proc, offset) = key;
		let clients: Vec<(ClientId, Placed)> = self
			.breakpoints
			.get(&key)
			.map(|claims| claims.keys().copied().collect())
			.unwrap_or_default();

		// One nobody is known to have set stops, as it always has
		let mut pause = clients.is_empty();
		for client in clients {
			let Some(condition) = self
				.breakpoints
				.get(&key)
				.and_then(|claims| claims.get(&client))
				.map(|claim| claim.condition.clone())
			else {
				continue;
			};
			if let Some(condition) = condition {
				// Condition errors skip the hit and notify the owner once.
				let passed = self.eval_expr(Some(0), &condition).map(|result| result.is_truthy());

				// We might have just executed some code so invalidate the
				// stacks we already fetched
				self.state.as_mut().unwrap().invalidate_stacks();
				match passed {
					Ok(true) => {}
					Ok(false) => continue,
					Err(error) => {
						self.condition_failed(key, client, &condition, &error);
						continue;
					}
				}
			}

			// The condition's code could have taken the breakpoint away
			let Some(claim) = self.breakpoints.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
				continue;
			};
			claim.hits += 1;
			if claim.hit.is_some_and(|hit| !hit.takes(claim.hits)) {
				continue;
			}

			// A breakpoint with a message says it to its client, and the
			// world carries on
			let Some(log) = claim.log.clone() else {
				pause = true;
				continue;
			};
			let message = self.log_message(&log);
			let caller = self
				.state
				.as_ref()
				.and_then(|state| state.active().get(1))
				.map(|frame| self.frame_info(frame, 1, false));
			let logged = Proc::from_id(proc).map(|proc| {
				let place = unnamed::proc_ref(&proc);
				let (file, line) = Self::place_of(&proc, offset as u32);
				Response::Log {
					message,
					file,
					line,
					proc: place,
					caller
				}
			});
			if let (Some(logged), Some(to)) = (logged, self.clients.get_mut(&client.0)) {
				if !Self::deliver(to, &logged) {
					self.remove_client(client.0);
				}
			}
		}
		pause
	}

	/// Report the first condition error and skip this hit.
	fn condition_failed(&mut self, key: (raw_types::procs::ProcId, u16), client: (ClientId, Placed), condition: &str, error: &str) {
		let Some(claim) = self.breakpoints.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
			return;
		};
		claim.skipped += 1;
		if std::mem::replace(&mut claim.told, true) {
			return;
		}
		let place = match Proc::from_id(key.0) {
			Some(proc) => match Self::place_of(&proc, key.1 as u32) {
				(Some(file), Some(line)) => format!(" at {}:{}", file, line),
				_ => format!(" in {}", unnamed::proc_name(&proc))
			},
			None => String::new()
		};
		let message = format!(
			"The condition ({}) of the breakpoint{} failed; skipping affected hits (reported once): {}",
			condition, place, error
		);
		self.tell(client.0, Response::ConditionFailed { message });
	}

	/// Sends one client something, at any moment
	fn tell(&mut self, client: ClientId, response: Response) {
		if let Some(to) = self.clients.get_mut(&client) {
			if !Self::deliver(to, &response) {
				self.remove_client(client);
			}
		}
	}

	/// Pause if requested by a client and return its resume action.
	/// `None` leaves current execution, including a pending step, unchanged.
	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<ContinueKind> {
		// Every runtime error goes in the record, whoever pauses for it
		if let BreakpointReason::Runtime { message, caught } = &reason {
			let stack = self.journal_stack();
			self.journal(
				JournalEvent::Runtime {
					message: message.clone(),
					caught: *caught,
					stack
				},
				None
			);
		}

		// Ignore all breakpoints unless someone's connected, and runtime errors
		// unless someone wants them
		let wanted = match &reason {
			BreakpointReason::Runtime { message, caught } => self.clients.values().any(|client| {
				let filter = if *caught { &client.caught } else { &client.uncaught };
				filter.as_ref().is_some_and(|filter| filter.takes(message))
			}),
			_ => self.has_debuggers() || self.hold.is_some()
		};
		if !wanted {
			return None;
		}

		self.state = Some(State::new());

		let proc = unsafe { (*(*(*_ctx).proc_instance())).proc };
		let offset = unsafe { *(*_ctx).bytecode_offset() };
		match reason {
			BreakpointReason::Breakpoint => {
				if !self.breakpoint_hit((proc, offset)) {
					self.state = None;
					return None;
				}
			}

			// A coincident step or pause still counts the breakpoint hit
			// and emits its log message. Report one stop for the step.
			BreakpointReason::Step | BreakpointReason::Pause if crate::instruction_hooking::is_breakpoint(_ctx) => {
				self.breakpoint_hit((proc, offset));
			}

			_ => {}
		}

		let at = unsafe {
			let proc = Proc::from_id((*(*(*_ctx).proc_instance())).proc);
			InstructionRef {
				proc: proc.as_ref().map(unnamed::proc_ref).unwrap_or(ProcRef {
					path: String::new(),
					override_id: 0
				}),
				offset: u32::from(*(*_ctx).bytecode_offset())
			}
		};
		let stop = Stop {
			reason,
			thread: self.thread_of(self.state.as_ref().unwrap().active()),
			at
		};
		self.paused = Some(stop.clone());
		self.broadcast(Response::BreakpointHit(stop), None);

		// Start the hold timer when the last debugging client leaves.
		// Transient status requests do not reset it. An attached client
		// can retain the pause past the deadline until it disconnects.
		let mut unattended: Option<Instant> = None;
		loop {
			// Held, it waits for clients to come back
			let mut wait = if self.hold.is_some() { Wait::Forever } else { Wait::WhileClients };
			if wait == Wait::WhileClients && !self.has_debuggers() {
				break;
			}
			// For so long, if the client that held it said
			if let (Wait::Forever, Some(max)) = (wait, self.hold_max) {
				if self.has_debuggers() {
					wait = Wait::Until(None);
				} else {
					wait = Wait::Until(Some(*unattended.get_or_insert_with(Instant::now) + max));
				}
			}
			let request = match self.recv_request(wait) {
				Some(request) => request,
				None => match wait {
					Wait::Until(Some(until)) if Instant::now() >= until => {
						self.let_hold_go();
						return Some(ContinueKind::Continue);
					}
					// A client came or went
					Wait::Until(_) => continue,
					_ => break
				}
			};

			// Hijack and handle any Continue requests. The other clients are
			// told the world carries on.
			if let Request::Continue { kind } = request {
				// A step is in a stack, which the client names by its id
				let position = |thread: u32| self.stack_position(thread).unwrap_or(0) as u32;
				let kind = match kind {
					ContinueKind::Continue => ContinueKind::Continue,
					ContinueKind::StepOver { stack_id } => ContinueKind::StepOver {
						stack_id: position(stack_id)
					},
					ContinueKind::StepInto { stack_id } => ContinueKind::StepInto {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOut { stack_id } => ContinueKind::StepOut {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOverInstruction { stack_id } => ContinueKind::StepOverInstruction {
						stack_id: position(stack_id)
					},
					ContinueKind::StepIntoInstruction { stack_id } => ContinueKind::StepIntoInstruction {
						stack_id: position(stack_id)
					}
				};
				self.send_or_disconnect(Response::Ack);
				self.broadcast(Response::Resumed, self.current);
				self.state = None;
				self.paused = None;
				self.hold = None;
				self.hold_max = None;
				// Whoever steps hears if the step ends with nothing to pause in
				self.stepper = if kind == ContinueKind::Continue { None } else { self.current };
				return Some(kind);
			}

			// Hijack eval too so that we can refresh our state after it
			if let Request::Eval { frame_id, command, .. } = request {
				self.handle_eval(frame_id, &command);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::Render { frame_id, expression, depth } = request {
				self.handle_render(frame_id, &expression, depth);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::EvalOn { object, expression, depth } = request {
				self.handle_eval_on(&object, &expression, depth);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}

			// if we get a pause request here we can ignore it
			self.handle_request(request);
		}

		// Every client that debugs left. The others hear the world carries on.
		self.broadcast(Response::Resumed, None);
		self.state = None;
		self.paused = None;
		Some(ContinueKind::Continue)
	}

	/// Release an expired held pause and journal the reason.
	fn let_hold_go(&mut self) {
		let seconds = self.hold_max.map_or(0.0, |max| max.as_secs_f64());
		eprintln!("Debug Server: resumed after {} seconds without a debugger", seconds);
		self.hold = None;
		self.hold_max = None;
		self.state = None;
		self.paused = None;
		self.journal(JournalEvent::HoldEnded { seconds }, None);
		self.broadcast(Response::Resumed, None);
	}

	/// Notify the stepping client when its stack ends without pausing.
	pub fn step_ended(&mut self) {
		let Some(id) = self.stepper.take() else {
			return;
		};
		if let Some(client) = self.clients.get_mut(&id) {
			if !Self::deliver(client, &Response::StepEnded) {
				self.remove_client(id);
			}
		}
	}

	// returns true if we need to pause
	pub fn process(&mut self) -> bool {
		let mut should_pause = false;

		waker::alive();
		if waker::woke() {
			self.notify("Woke world suspended by world.sleep_offline to process debugger requests");
		}

		// Avoid polling an empty channel on every DM instruction.
		if waker::events_pending() {
			while let Some(request) = self.recv_request(Wait::No) {
				// Not `||`, that would skip every request after a Pause
				should_pause |= self.handle_request(request);
			}
		}

		should_pause
	}

	/// Process requests until a debugging client finishes configuration.
	pub fn process_until_configured(&mut self) {
		self.waiting_to_start = true;
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.send_or_disconnect(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
		self.waiting_to_start = false;
	}

	/// Answers the client whose request this is
	fn send_or_disconnect(&mut self, response: Response) {
		// Nobody to tell: the client left while we were busy
		let Some(id) = self.current else {
			return;
		};
		let Some(client) = self.clients.get_mut(&id) else {
			return;
		};

		if !Self::deliver(client, &response) {
			eprintln!("Debug server failed to send a message to a client");
			self.remove_client(id);
		}
	}

	/// Tells every client but `except`
	fn broadcast(&mut self, response: Response, except: Option<ClientId>) {
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			if Some(id) != except && !Self::deliver(client, &response) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	/// Disconnect a client and remove its breakpoint claims.
	fn remove_client(&mut self, id: ClientId) {
		let Some(client) = self.clients.remove(&id) else {
			return;
		};
		// The writer drains queued messages before sending Disconnect.
		drop(client);
		if self.current == Some(id) {
			self.current = None;
		}
		// Discard a pending pause hold if its owner leaves before the pause.
		if self.hold == Some(id) && self.paused.is_none() {
			self.hold = None;
			self.hold_max = None;
		}

		self.forget_watches_of(id);

		let mut unclaimed = vec![];
		for (key, claims) in &mut self.breakpoints {
			claims.retain(|(client, _), _| *client != id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		self.moved_breakpoints.retain(|(.., (client, _)), _| *client != id);
		for (proc, offset) in unclaimed {
			self.breakpoints.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}

		// Whoever connects next sets up their own breakpoints. Until then the
		// old ones would only slow down the procs they sit in.
		if self.clients.is_empty() {
			self.breakpoints.clear();
			self.moved_breakpoints.clear();
			unhook_all();
		}
	}

	fn disconnect_all(&mut self) {
		let ids: Vec<ClientId> = self.clients.keys().copied().collect();
		for id in ids {
			self.remove_client(id);
		}
	}

	pub(crate) fn send(stream: &mut impl Write, response: &Response) -> Result<(), Box<dyn Error>> {
		stream.write_all(&Self::encode(response)?)?;
		stream.flush()?;
		Ok(())
	}

	/// Frame a bincode message with its length prefix.
	fn encode(response: &Response) -> Result<Vec<u8>, Box<dyn Error>> {
		let data = bincode::serialize(response)?;
		let mut bytes = Vec::with_capacity(data.len() + 4);
		bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
		bytes.extend_from_slice(&data);
		Ok(bytes)
	}

	/// Deliver a message; return false if the client must be disconnected.
	/// Queue writes off the world thread: Windows flushes block until the
	/// client reads the data. Disconnect clients whose queues fill.
	fn deliver(client: &mut Client, response: &Response) -> bool {
		Self::encode(response).is_ok_and(|bytes| client.outbox.try_send(bytes).is_ok())
	}

	/// Start a per-client writer. Drain queued messages before disconnecting.
	fn writer(mut connection: Connection) -> (mpsc::SyncSender<Vec<u8>>, thread::JoinHandle<()>) {
		let (outbox, sent) = mpsc::sync_channel::<Vec<u8>>(OUTBOX);
		let writer = thread::spawn(move || {
			while let Ok(bytes) = sent.recv() {
				if connection.write_all(&bytes).and_then(|()| connection.flush()).is_err() {
					return;
				}
			}
			let _ = Self::send(&mut connection, &Response::Disconnect);
		});
		(outbox, writer)
	}

	/// Notify clients of shutdown, disconnect them, and briefly wait for
	/// writers to drain before the process exits. This does not run for reboot.
	pub fn world_shutting_down(&mut self) {
		self.broadcast(
			Response::Notification {
				message: WORLD_SHUTTING_DOWN.to_owned()
			},
			None
		);
		let writers: Vec<_> = self.clients.values_mut().filter_map(|client| client.writer.take()).collect();
		self.disconnect_all();
		let deadline = Instant::now() + Duration::from_secs(1);
		while writers.iter().any(|writer| !writer.is_finished()) && Instant::now() < deadline {
			thread::sleep(Duration::from_millis(5));
		}
	}

	/// Append a journal entry and notify matching subscribers except `from`.
	fn journal(&mut self, event: JournalEvent, from: Option<ClientId>) {
		let entry = journal::add(event);
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			let follows = client.follow.as_ref().is_some_and(|kinds| journal::wanted(&entry, kinds));
			if follows && Some(id) != from && !Self::deliver(client, &Response::Journaled(entry.clone())) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	/// The procs the running code is in, as the record keeps them
	fn journal_stack(&self) -> Vec<JournalFrame> {
		Self::shown(&debug::CallStacks::active())
			.iter()
			.take(JOURNAL_FRAMES)
			.map(|frame| {
				let (file, line) = self.frame_place(frame, false);
				JournalFrame {
					proc: unnamed::proc_name(&frame.proc),
					file,
					line
				}
			})
			.collect()
	}

	/// What `Request::Journal` asks for
	fn handle_journal(&mut self, after: u64, kinds: Vec<JournalKind>, limit: Option<u32>, follow: bool) {
		let (entries, first, last) = journal::read(after, &kinds, limit);
		if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
			client.follow = follow.then_some(kinds);
		}
		self.send_or_disconnect(Response::Journal { entries, first, last });
	}
}

/// Whether a frame's background flag can be read. Its place in the
/// execution context was found on BYOND 516; older builds lay it out
/// differently.
fn background_known() -> bool {
	auxtools::version::get().0 >= 516
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect_all();

		// Clients from now on wait for the next Server
		if let Some(acceptor) = ACCEPTOR.lock().unwrap().as_mut() {
			let (events, waiting) = mpsc::channel();
			acceptor.events = events;
			acceptor.waiting = Some(waiting);
		}

		// Clients that connected before that, which we never got told about,
		// are told to go
		while let Ok(event) = self.events.try_recv() {
			if let Event::Connected(_, mut connection) = event {
				thread::spawn(move || Self::send(&mut connection, &Response::Disconnect).is_ok());
			}
		}
	}
}

/// Maximum callee runtime errors reported in full per evaluation.
const MAX_CALLEE_ERRORS: usize = 5;

/// Whether the frame runs debugger evaluation or spawned code.
pub(crate) fn is_expression_frame(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	["/proc/auxtools_expr_stub", "/proc/auxtools_spawn_stub"]
		.into_iter()
		.any(|path| Proc::find(path).is_some_and(|stub| stub.id == id))
}

/// Reads one request off a connection: a u32 length, then the request in
/// bincode.
fn read_request(stream: &mut impl Read, buf: &mut Vec<u8>) -> Result<Request, Box<dyn Error>> {
	let mut len_bytes = [0u8; 4];
	stream.read_exact(&mut len_bytes)?;
	let len = u32::from_le_bytes(len_bytes);

	if len > MAX_REQUEST_LEN {
		return Err(format!("request length {} exceeds the protocol limit", len).into());
	}

	buf.resize(len as usize, 0);
	stream.read_exact(buf)?;
	Ok(bincode::deserialize::<Request>(buf)?)
}

impl ServerThread {
	fn run(&self, id: ClientId, mut stream: Connection) {
		let mut buf = vec![];
		waker::client_connected();

		// Normal disconnects and EOF do not need world log entries.
		while let Ok(request) = read_request(&mut stream, &mut buf) {
			if let Request::Disconnect = request {
				break;
			}

			waker::request_waiting();
			if let Err(e) = self.events.send(Event::Request(id, request)) {
				eprintln!("Debug server thread failed to handle request: {}", e);
				break;
			}
			waker::event_sent();
		}

		waker::client_left();
		let _ = self.events.send(Event::Disconnected(id));
		waker::event_sent();
	}
}

/// Cache each local slot's first and last bytecode uses by code content.
/// Breakpoint conditions need this lookup on every hit.
fn local_uses(code: &[u32]) -> std::sync::Arc<HashMap<u32, (u32, u32)>> {
	use std::hash::{Hash, Hasher};
	type Uses = std::sync::Arc<HashMap<u32, (u32, u32)>>;
	static USES: std::sync::LazyLock<Mutex<HashMap<(u64, usize), Uses>>> = std::sync::LazyLock::new(Default::default);
	const KEEP: usize = 1024;

	fn locals_in(var: &dmasm::Variable, found: &mut Vec<u32>) {
		match var {
			dmasm::Variable::Local(slot) => found.push(*slot),
			dmasm::Variable::SetCache(a, b) => {
				locals_in(a, found);
				locals_in(b, found);
			}
			dmasm::Variable::Initial(inner) | dmasm::Variable::IsSaved(inner) => locals_in(inner, found),
			_ => {}
		}
	}
	let key = {
		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		code.hash(&mut hasher);
		(hasher.finish(), code.len())
	};
	if let Some(uses) = USES.lock().unwrap().get(&key) {
		return uses.clone();
	}
	let (nodes, _) = dmasm::disassembler::disassemble(code, &mut disassemble_env::DisassembleEnv);
	let mut uses: HashMap<u32, (u32, u32)> = HashMap::new();
	for node in &nodes {
		let dmasm::Node::Instruction(ins, debug) = node else { continue };
		let mut found = vec![];
		for var in ins.variables() {
			locals_in(var, &mut found);
		}
		for slot in found {
			let at = debug.offset as u32;
			uses.entry(slot).and_modify(|(_, last)| *last = at).or_insert((at, at));
		}
	}
	let uses = std::sync::Arc::new(uses);
	let mut kept = USES.lock().unwrap();
	if kept.len() >= KEEP {
		kept.clear();
	}
	kept.insert(key, uses.clone());
	uses
}

#[cfg(test)]
mod tests {
	use std::{io::Read, sync::mpsc, thread, time::Duration};

	use super::{Connection, Server};
	use crate::connection;

	#[test]
	fn drop_hangs_up_on_clients_and_the_next_server_takes_new_ones() {
		let (done, dropped) = mpsc::channel();

		thread::spawn(move || {
			let mut server = Server::listen().unwrap();
			let first = Connection::connect(&connection::world_name()).unwrap();
			let second = Connection::connect(&connection::world_name()).unwrap();
			thread::sleep(Duration::from_millis(200));
			// Both are taken in
			assert!(server.recv_request(super::Wait::No).is_none());
			assert_eq!(server.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("server drop blocked on client connections");
		// Each was told to go: a Disconnect, then the end of the connection
		for client in [&mut first, &mut second] {
			let mut len = [0u8; 4];
			client.read_exact(&mut len).expect("no Disconnect");
		}

		// A client that connects while the world reboots waits for the next
		// server
		let _third = Connection::connect(&connection::world_name()).expect("listener unavailable between server instances");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.clients.len(), 1);
		assert!(Server::listen().is_err(), "multiple active servers accepted");
	}
}
