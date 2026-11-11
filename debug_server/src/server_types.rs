// TODO: These will be shared properly

use serde::{Deserialize, Serialize};

/// Wire protocol version. Increment for changes incompatible with older
/// clients or servers. Including the version in endpoint names prevents
/// connections using incompatible message layouts.
pub const PROTOCOL_VERSION: u32 = 5;

/// The local socket the world in process `pid` listens on, when it has the
/// debug library: `auxtools-debug-v1-1234`
#[allow(dead_code)]
pub fn world_socket_name(pid: u32) -> String {
	world_socket_name_for(PROTOCOL_VERSION, pid)
}

/// Endpoint name for a specified protocol version, used for discovery.
#[allow(dead_code)]
pub fn world_socket_name_for(protocol: u32, pid: u32) -> String {
	format!("auxtools-debug-v{}-{}", protocol, pid)
}

// Message from client -> server
#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
	Disconnect,
	Configured,
	StdDef,
	Eval {
		frame_id: Option<u32>,
		command: String,
		context: Option<String>
	},
	BreakpointSet {
		instruction: InstructionRef,
		condition: Option<String>,
		/// Hit filter applied after the condition succeeds. `None` accepts
		/// every hit.
		hit: Option<HitCondition>,
		/// Logpoint message sent as `Response::Log` instead of pausing.
		log: Option<Vec<LogPart>>
	},
	BreakpointUnset {
		instruction: InstructionRef
	},
	/// Set uncaught and caught runtime error filters, including thrown
	/// exceptions. `None` disables that category. No response.
	CatchRuntimes {
		uncaught: Option<RuntimeFilter>,
		caught: Option<RuntimeFilter>
	},
	Offset {
		proc: ProcRef,
		line: u32
	},
	/// List the running stack and up to `sleeping` suspended stacks.
	/// `None` uses [`DEFAULT_SLEEPING_STACKS`].
	Stacks {
		sleeping: Option<u32>
	},
	StackFrames {
		stack_id: u32,
		start_frame: Option<u32>,
		count: Option<u32>
	},
	Scopes {
		frame_id: u32
	},
	Variables {
		vars: VariablesRef,
		/// List pagination: zero-based start and count. `None` returns all
		/// entries.
		start: Option<u32>,
		count: Option<u32>
	},
	Continue {
		kind: ContinueKind
	},
	Pause,

	// New requests go last, so older debug-clients' requests keep their numbers
	/// Which BYOND version the world runs on
	ByondVersion,
	/// Apply proc, variable, metadata and line-number changes.
	/// Apply the entire reload or reject it without changing runtime state.
	HotReload(HotReloadRequest),
	/// The program's command line and working directory, which say which world
	/// it runs
	WorldInfo,
	/// Whether the world is paused, and why
	Status,
	/// Retain the current or next pause after disconnecting, until resumed.
	/// Discard the request if the client disconnects before a pause occurs.
	HoldPause,
	/// Locate executable code using DbgFile/DbgLine. Prefer exact file names,
	/// then suffix matches. Non-executable lines resolve to the next code line.
	Locate {
		file: String,
		line: u32
	},
	/// Evaluate an assignment to `name` in container `vars`. Use the variable's
	/// frame, or the paused frame for non-frame containers.
	SetVariable {
		vars: VariablesRef,
		name: String,
		value: String
	},
	/// Complete names starting with `prefix`. With `base`, resolve members of
	/// that expression; otherwise use names visible in the frame.
	Complete {
		frame_id: Option<u32>,
		base: Option<String>,
		prefix: String
	},
	/// Mark this connection as a watcher rather than a debugger. Watchers
	/// do not retain pauses when the last debugging client disconnects.
	Watcher,
	/// The lines from `start` to `end` (both included) of a file that have
	/// code, in the code the world runs now. `file` is matched as in `Locate`.
	LinesWithCode {
		file: String,
		start: u32,
		end: u32
	},
	/// List debug source names from proc bytecode. Files without procs are
	/// absent.
	Files,
	/// Resolve a watch target without installing a watch.
	WatchInfo {
		place: WatchPlace
	},
	/// Pause when a watched value changes. Conditions evaluate in the running
	/// frame with `value` and `old` bound to the new and previous values.
	WatchSet {
		place: WatchPlace,
		condition: Option<String>,
		hit: Option<HitCondition>
	},
	WatchUnset {
		id: u32
	},
	/// The instructions of some code: a proc's, or the old code a frame runs
	Disassemble {
		code: CodeRef
	},
	/// Set an instruction breakpoint at an exact instruction boundary.
	/// Offset 0 means the first instruction. The claim is independent of this
	/// client's line and function breakpoints. Respond as for `BreakpointSet`.
	InstructionBreakpointSet {
		instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>
	},
	/// Takes away what `InstructionBreakpointSet` set. Answered as
	/// `BreakpointUnset` is.
	InstructionBreakpointUnset {
		instruction: InstructionRef
	},
	/// Read matching journal entries after `after`, oldest first. An empty kind
	/// list selects all kinds; `limit` selects the newest matching entries.
	/// With `follow`, stream new entries as `Response::Journaled` until
	/// replaced by another `Journal` request. Initial response:
	/// `Response::Journal`.
	Journal {
		after: u64,
		kinds: Vec<JournalKind>,
		limit: Option<u32>,
		follow: bool
	},
	/// Append launcher-observed reload results or world output to the journal.
	/// No response; do not echo the entry to its submitting client.
	JournalAdd(JournalEvent),
	/// How long the world has been up, and its last hot reload
	Uptime,
	/// Set the maximum held-pause duration without a debugging client.
	/// `None` removes the limit. At expiry, resume once remaining debugging
	/// clients disconnect. Respond with `Ack`.
	HoldMax {
		seconds: Option<f64>
	},
	/// Where a proc is, and what hot reloads did to it. `proc.path` can be
	/// written with `/proc/` or `/verb/` in it, or without.
	ProcInfo {
		proc: ProcRef
	},
	/// Evaluate and recursively serialize a value to `depth` (see
	/// [`Rendered`]). Available while running; evaluation uses the same scope
	/// as `Eval`.
	Render {
		frame_id: Option<u32>,
		expression: String,
		depth: u32
	},
	/// Serialize frame arguments/locals, globals or object variables to
	/// `depth`.
	RenderVariables {
		vars: VariablesRef,
		depth: u32
	},
	/// Evaluate with an object as `src`, outside any frame. Resolve bare names
	/// and calls as in that object's procs. Accept a ref() value with optional
	/// brackets. Respond as for `Eval`, or `Render` when `depth` is supplied.
	EvalOn {
		object: String,
		expression: String,
		depth: Option<u32>
	},
	/// Replace source macro definitions for all clients. Definitions survive
	/// reboots for the process lifetime. No response.
	Defines(Vec<Macro>),
	/// Queue evaluation on the next world tick, outside any frame. Optional
	/// `src` is resolved as in `EvalOn`. The code may sleep or hit breakpoints.
	/// Respond with `Response::Spawned` after compilation; journal the result
	/// as `JournalEvent::Spawned`.
	Spawn {
		expression: String,
		object: Option<String>
	}
}

/// A `#define` of the world's code, as its preprocessor has it
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Macro {
	pub name: String,
	/// Optional parameter list; variadic parameters retain their trailing
	/// `...`.
	pub params: Vec<String>,
	/// Replacement text on one line.
	pub text: String
}

// Message from server -> client
#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
	Ack,
	StdDef(Option<String>),
	/// Evaluation result or error.
	Eval(Result<EvalResponse, String>),
	BreakpointSet {
		result: BreakpointSetResult
	},
	BreakpointUnset {
		success: bool,
		/// Hits skipped because evaluating the condition failed.
		skipped: u32
	},
	Offset {
		offset: Option<u32>
	},
	Stacks {
		stacks: Vec<Stack>,
		/// How many procs are asleep, listed or not
		sleeping: u32
	},
	StackFrames {
		frames: Vec<StackFrame>,
		total_count: u32
	},
	Scopes {
		arguments: Option<VariablesRef>,
		locals: Option<VariablesRef>,
		globals: Option<VariablesRef>
	},
	Variables {
		vars: Vec<Variable>
	},

	// Asynchronous events may arrive between a request and its response.
	Disconnect,
	Notification {
		message: String
	},
	BreakpointHit(Stop),

	ByondVersion {
		version: u32,
		build: u32
	},
	HotReload(HotReloadResult),
	WorldInfo {
		args: Vec<String>,
		cwd: String
	},
	/// Another client resumed execution.
	Resumed,
	Status {
		paused: Option<Stop>
	},
	Locate {
		instructions: Vec<InstructionRef>
	},
	/// Another client reloaded code; source line mappings may have changed.
	CodeChanged,
	Complete(Vec<Completion>),
	/// The variable as it is now, or why it couldn't be set
	SetVariable(Result<Variable, String>),
	/// Logpoint message, location and caller frame. `None` means there is no
	/// caller, e.g. after sleep or spawn, or for a world-started proc.
	Log {
		message: String,
		file: Option<String>,
		line: Option<u32>,
		proc: ProcRef,
		caller: Option<StackFrame>
	},
	/// Sorted, each once
	LinesWithCode {
		lines: Vec<u32>
	},
	/// Sorted, spelled as the world has them
	Files {
		files: Vec<String>
	},
	/// What the place is called, or why it can't be watched
	WatchInfo(Result<String, String>),
	WatchSet(Result<Watch, String>),
	/// Watch expired because its target no longer exists. Sent asynchronously
	/// to its owner after a proc returns or object is deleted.
	WatchEnded {
		id: u32,
		reason: String
	},
	Disassemble(Result<Disassembly, String>),
	/// Requested entries and the oldest retained/newest entry numbers (0 if
	/// empty).
	Journal {
		entries: Vec<JournalEntry>,
		first: u64,
		last: u64
	},
	/// A new entry, for a client that follows the record. At any moment.
	Journaled(JournalEntry),
	Uptime {
		/// When this world loaded, in milliseconds since 1970 (UTC)
		loaded: u64,
		/// How long ago that was, in seconds
		up: f64,
		/// `world.time`, in deciseconds
		world_time: f64,
		/// How many worlds the process has loaded: 1, then one more for each
		/// reboot
		loads: u32,
		last_reload: Option<JournalEntry>,
		/// Whether world code has started executing after launcher
		/// configuration.
		started: bool
	},
	/// A breakpoint or watch condition failed; skip affected hits and notify
	/// the owner once. May arrive asynchronously.
	ConditionFailed {
		message: String
	},
	/// What the world knows of a proc, or why it knows nothing
	ProcInfo(Result<ProcInfo, String>),
	/// What an expression gave, or why it gave nothing
	Render(Result<Rendered, String>),
	/// Each var's name and value, or why there are none
	RenderVariables(Result<Vec<(String, Rendered)>, String>),
	/// Spawned evaluation ID for matching its journal result, or an error.
	Spawned(Result<u64, String>),
	/// The requested step ended without pausing because its stack has no caller
	/// to stop in. A step that pauses instead sends `BreakpointHit`.
	StepEnded
}

/// Journal entry retained across reboots for the process lifetime.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JournalEntry {
	/// Monotonic entry number, starting at 1 and never reused.
	pub number: u64,
	/// When it was made, in milliseconds since 1970 (UTC)
	pub time: u64,
	/// `world.time` then, in deciseconds
	pub world_time: f64,
	pub event: JournalEvent
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum JournalEvent {
	/// A runtime error, or a `throw`, whether or not anything paused for it:
	/// the procs it happened in, innermost first
	Runtime {
		message: String,
		/// It's inside a `try` that catches it
		caught: bool,
		stack: Vec<JournalFrame>
	},
	/// Launcher-reported reload result. Errors explain rejection, including
	/// compile failures detected before sending a reload request.
	Reload {
		applied: bool,
		summary: String,
		errors: Vec<String>,
		warnings: Vec<String>
	},
	/// World load event; load 1 is startup and later loads are reboots.
	Loaded { load: u32 },
	/// Lines the world printed, as a launcher that reads them passes them on
	Output { lines: Vec<String> },
	/// Held pause released after this many seconds without a debugging client.
	HoldEnded { seconds: f64 },
	/// Spawned evaluation result or runtime error with its evaluation call
	/// stack.
	Spawned {
		/// The number `Response::Spawned` gave it
		spawn: u64,
		expression: String,
		result: Result<SpawnedValue, String>
	}
}

/// What a spawned expression gave
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SpawnedValue {
	/// As `Eval` writes it
	pub text: String,
	/// Serialized to depth 1: list entries, or object metadata without
	/// variables.
	pub rendered: Rendered
}

impl JournalEvent {
	pub const fn kind(&self) -> JournalKind {
		match self {
			Self::Runtime { .. } => JournalKind::Runtime,
			Self::Reload { .. } => JournalKind::Reload,
			Self::Loaded { .. } | Self::HoldEnded { .. } => JournalKind::World,
			Self::Output { .. } => JournalKind::Output,
			Self::Spawned { .. } => JournalKind::Spawned
		}
	}
}

/// What a `Request::Journal` can ask for
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind {
	Runtime,
	Reload,
	/// Worlds loading, and what the debug server did by itself
	World,
	Output,
	/// What came of expressions run with `Request::Spawn`
	Spawned
}

/// A proc a runtime error happened in
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JournalFrame {
	/// Its path, with `#N` after it for the Nth override
	pub proc: String,
	pub file: Option<String>,
	pub line: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone)]
pub struct ProcRef {
	pub path: String,
	pub override_id: u32
}

#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone)]
pub struct InstructionRef {
	pub proc: ProcRef,
	pub offset: u32
}

/// Reference to current proc bytecode or retained pre-reload code.
/// Combine with an instruction offset using [`address`].
#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct CodeRef {
	/// The engine's id of the proc, which is the same in each world started
	/// from the same .dmb
	pub proc: u32,
	/// 0 for current code; otherwise a retained buffer ID valid until reboot.
	pub old: u16
}

/// Pack proc ID, code ID and word offset as `0xPPPPPPPPCCCCOOOO`.
/// Addresses increase by one per bytecode word.
#[allow(dead_code)]
pub const fn address(code: CodeRef, offset: u16) -> u64 {
	(code.proc as u64) << 32 | (code.old as u64) << 16 | offset as u64
}

/// The code and offset an [`address`] is of
#[allow(dead_code)]
pub const fn from_address(address: u64) -> (CodeRef, u16) {
	let code = CodeRef {
		proc: (address >> 32) as u32,
		old: (address >> 16) as u16
	};
	(code, address as u16)
}

/// The instructions of some code (`Request::Disassemble`)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
	pub proc: ProcRef,
	/// The file its DbgFile names
	pub file: Option<String>,
	/// Instructions in order. Unreadable trailing words have no text;
	/// `error` describes the disassembly failure.
	pub instructions: Vec<BytecodeInstruction>,
	pub error: Option<String>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BytecodeInstruction {
	/// In words, from the code's start
	pub offset: u16,
	/// Its opcode and operands, as the code has them (without the debugger's
	/// breakpoints)
	pub words: Vec<u32>,
	/// As dmasm writes it
	pub text: String,
	/// The line it's on, by the DbgLine before it (a line of the file as it
	/// was, for old code)
	pub line: Option<u32>
}

/// Recursively serialized value. At depth 1 include list entries; at depth 2
/// include object variables. Descend one level for each child. At the depth
/// limit return list length or object metadata.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Rendered {
	Null,
	Number(f32),
	Text(String),
	List {
		len: u32,
		/// Each entry, and the value it has as a key if it has one
		entries: Option<Vec<(Rendered, Option<Rendered>)>>
	},
	/// A datum, an atom, a client, the world...
	Object {
		/// As `ref()` gives it: `[0x2000001]`
		reference: String,
		/// Its type's path
		kind: String,
		/// Its `name`, if it has one that's text
		name: Option<String>,
		vars: Option<Vec<(String, Rendered)>>
	},
	/// Other DM value with kind label and display text, e.g. a path or pointer.
	Other {
		kind: String,
		text: String
	}
}

/// A proc, as `Request::ProcInfo` finds it
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
	/// Its path as the world spells it, and which definition of it it is
	pub proc: ProcRef,
	/// Its code now, for `Request::Disassemble`
	pub code: CodeRef,
	/// The file its DbgFile names
	pub file: Option<String>,
	/// The first line it has code on
	pub line: Option<u32>,
	/// Definition count; valid zero-based override IDs are below this value.
	pub definitions: u32,
	/// What hot reloads have done to it since the world started, if anything
	pub reloaded: Option<Reloaded>
}

/// Most significant reload change affecting a proc.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reloaded {
	/// Only debug line numbers changed.
	Lines,
	/// Bytecode replaced.
	Code,
	/// Proc added after world load.
	Added
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointReason {
	Breakpoint,
	Step,
	Pause,
	/// A runtime error, or a `throw`: its message, and whether it's inside
	/// a `try` that will catch it
	Runtime {
		message: String,
		caught: bool
	},
	/// Watched value changes observed after the preceding instruction or an
	/// engine update, before the current stop.
	Data {
		changes: Vec<DataChange>
	}
}

/// A place to watch for a change
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum WatchPlace {
	/// A variable as `Response::Variables` listed it: `name` in `vars`
	Variable { vars: VariablesRef, name: String },
	/// Expression watch target: a bare frame variable name, or an expression
	/// that can produce a pointer through `&(expression)`.
	Expression { frame_id: Option<u32>, expression: String }
}

/// A watch that was set
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Watch {
	/// Watch ID unique across all clients in this world.
	pub id: u32,
	/// What the place is called
	pub name: String,
	/// What's there now
	pub value: String
}

/// What a watch saw
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DataChange {
	/// The watch's (`Watch::id`)
	pub id: u32,
	pub name: String,
	pub old: String,
	pub new: String
}

/// A name that could be typed next
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Completion {
	pub name: String,
	/// It's a proc to call, not a var to read
	pub proc: bool
}

/// A piece of a breakpoint's log message
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum LogPart {
	Text(String),
	/// An expression, evaluated in the frame of the hit: its value goes in
	/// its place
	Expr(String)
}

#[allow(dead_code)]
impl LogPart {
	/// Parse text with `{expression}` interpolation. Respect nested braces
	/// and strings; an unmatched opening brace remains literal text.
	pub fn parse(message: &str) -> Vec<Self> {
		let mut parts = vec![];
		let mut text = String::new();
		let mut rest = message;
		while let Some(open) = rest.find('{') {
			let Some(length) = Self::expression_length(&rest[open + 1..]) else {
				break;
			};
			text.push_str(&rest[..open]);
			if !text.is_empty() {
				parts.push(Self::Text(std::mem::take(&mut text)));
			}
			parts.push(Self::Expr(rest[open + 1..open + 1 + length].trim().to_owned()));
			rest = &rest[open + 1 + length + 1..];
		}
		text.push_str(rest);
		if !text.is_empty() {
			parts.push(Self::Text(text));
		}
		parts
	}

	/// How far it is to the `}` that closes an expression starting here
	fn expression_length(text: &str) -> Option<usize> {
		let mut depth = 0;
		let mut quoted = false;
		let mut escaped = false;
		for (at, c) in text.char_indices() {
			if quoted {
				match c {
					_ if escaped => escaped = false,
					'\\' => escaped = true,
					'"' => quoted = false,
					_ => {}
				}
				continue;
			}
			match c {
				'"' => quoted = true,
				'{' => depth += 1,
				'}' if depth == 0 => return Some(at),
				'}' => depth -= 1,
				_ => {}
			}
		}
		None
	}
}

/// Breakpoint hit filter; hit numbers start at 1.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitCondition {
	AtLeast(u32),
	Equal(u32),
	More(u32),
	Less(u32),
	AtMost(u32),
	/// Every Nth hit.
	Every(u32)
}

#[allow(dead_code)]
impl HitCondition {
	/// Parse `N` (Nth hit onward), or `==`, `>`, `>=`, `<`, `<=`, `%` followed
	/// by N.
	pub fn parse(text: &str) -> Result<Self, String> {
		let text = text.trim();
		let (make, number): (fn(u32) -> Self, &str) = if let Some(rest) = text.strip_prefix(">=") {
			(Self::AtLeast, rest)
		} else if let Some(rest) = text.strip_prefix("<=") {
			(Self::AtMost, rest)
		} else if let Some(rest) = text.strip_prefix("==").or_else(|| text.strip_prefix('=')) {
			(Self::Equal, rest)
		} else if let Some(rest) = text.strip_prefix('>') {
			(Self::More, rest)
		} else if let Some(rest) = text.strip_prefix('<') {
			(Self::Less, rest)
		} else if let Some(rest) = text.strip_prefix('%') {
			(Self::Every, rest)
		} else {
			(Self::AtLeast, text)
		};
		match number.trim().parse::<u32>() {
			Ok(0) if make(0) == Self::Every(0) => Err("hit interval must be greater than zero".to_owned()),
			Ok(number) => Ok(make(number)),
			Err(_) => Err(format!(
				"invalid hit count {:?}; expected N or an operator (==, >, >=, <, <=, %) followed by N",
				text
			))
		}
	}

	/// Whether the hit of this number is one to stop at
	pub const fn takes(self, hits: u32) -> bool {
		match self {
			Self::AtLeast(n) => hits >= n,
			Self::Equal(n) => hits == n,
			Self::More(n) => hits > n,
			Self::Less(n) => hits < n,
			Self::AtMost(n) => hits <= n,
			Self::Every(n) => n != 0 && hits % n == 0
		}
	}
}

/// Which runtime errors to pause on, by their messages
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeFilter {
	/// Accept messages containing any of these strings, if nonempty.
	pub with: Vec<String>,
	/// Reject messages containing any of these strings.
	pub without: Vec<String>
}

#[allow(dead_code)]
impl RuntimeFilter {
	/// Parse comma-separated, case-insensitive message filters. Prefix `!`
	/// to exclude a substring. Empty input matches all errors. For example,
	/// `divi, bounds` accepts either substring; `!libdreamluau` excludes it.
	pub fn parse(condition: &str) -> Self {
		let mut filter = Self::default();
		for text in condition.split(',').map(str::trim).filter(|text| !text.is_empty()) {
			match text.strip_prefix('!') {
				Some(text) if !text.trim().is_empty() => filter.without.push(text.trim().to_lowercase()),
				Some(_) => {}
				None => filter.with.push(text.to_lowercase())
			}
		}
		filter
	}

	/// Whether an error with this message is one to pause on
	pub fn takes(&self, message: &str) -> bool {
		let message = message.to_lowercase();
		(self.with.is_empty() || self.with.iter().any(|text| message.contains(text))) && !self.without.iter().any(|text| message.contains(text))
	}
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum ContinueKind {
	Continue,
	StepOver {
		stack_id: u32
	},
	StepInto {
		stack_id: u32
	},
	StepOut {
		stack_id: u32
	},
	/// Step to the next instruction in the target frame, running calls to
	/// completion. If the frame returns, stop in its caller.
	StepOverInstruction {
		stack_id: u32
	},
	/// Step to the next instruction, entering callees.
	StepIntoInstruction {
		stack_id: u32
	}
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Stack {
	/// Stable stack ID for the outermost proc instance's lifetime. May be
	/// reused after return. Request ID 0 selects the currently running stack.
	pub id: u32,
	/// The proc it's in: its innermost frame's
	pub name: String,
	/// Where that frame is
	pub file: Option<String>,
	pub line: Option<u32>,
	/// Asleep, not the proc that's running
	pub sleeping: bool
}

/// How many sleeping procs' stacks `Request::Stacks` lists unless told
pub const DEFAULT_SLEEPING_STACKS: u32 = 100;

/// The world, paused
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Stop {
	pub reason: BreakpointReason,
	/// The stack of the proc that was running (a [`Stack`]'s id)
	pub thread: u32,
	/// The instruction it's about to run: a breakpoint's
	/// (`BreakpointSetResult::Success::at`), if it stopped for one
	pub at: InstructionRef
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct StackFrame {
	/// Running frame IDs are depths from 0, innermost first. Sleeping frames
	/// receive stable IDs for this pause, invalidated if the frame ends.
	pub id: u32,
	pub instruction: InstructionRef,
	pub line: Option<u32>,
	/// The file its proc is in, as its DbgFile names it (a world built with
	/// DEBUG has them)
	pub file: Option<String>,
	/// Frame still executing pre-reload bytecode; its source line refers
	/// to the previous file contents.
	pub old_code: bool,
	/// Bytecode reference for disassembly. Combine with `instruction.offset`
	/// using [`address`].
	pub code: CodeRef
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointSetResult {
	Success {
		line: Option<u32>,
		/// Where it is, as a stop there has it (`Stop::at`)
		at: InstructionRef,
		/// The file its proc is in, as its DbgFile names it
		file: Option<String>
	},
	/// There's no such proc, or no instruction there
	Failed,
	/// It can't be set as it was asked for: its condition doesn't compile
	Refused { reason: String }
}

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct VariablesRef(pub i32);

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Variable {
	pub name: String,
	pub value: String,
	pub variables: Option<VariablesRef>,
	/// What kind of value it is: an object's type, or `num`, `text`, `/list`
	/// and the like
	pub kind: Option<String>,
	/// An expression that gives it, in the frame it was listed for
	pub expr: Option<String>,
	/// For a list: how many entries it has
	pub entries: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EvalResponse {
	pub value: String,
	pub variables: Option<VariablesRef>,
	/// As a `Variable`'s
	pub kind: Option<String>,
	pub entries: Option<u32>
}

/// What a hot reload changes (see `Request::HotReload`)
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HotReloadRequest {
	/// Add types before other changes, parents first. Added types remain
	/// unused if preparing the rest of the reload fails.
	pub new_types: Vec<dmasm::reload::NewType>,
	pub procs: Vec<HotReloadProc>,
	/// Settings changes for procs with unchanged bytecode.
	pub settings: Vec<dmasm::reload::SettingsChange>,
	pub new_procs: Vec<HotReloadNewProc>,
	/// Definitions on types that are gone, unlinked before new procs are added
	pub removed_procs: Vec<dmasm::reload::RemovedProc>,
	/// Added before anything else, so the code in the reload can use them
	pub new_vars: Vec<dmasm::reload::NewVar>,
	/// Apply declaration changes after adding variables and before defaults.
	#[serde(default)]
	pub redeclared: Vec<dmasm::reload::Redeclared>,
	/// Applied after the new vars, so they can set the defaults of those too
	pub var_defaults: Vec<HotReloadVarDefault>,
	/// New defaults of built-in vars, which types keep in their records
	pub builtin_defaults: Vec<dmasm::reload::VarDefault>,
	/// Types whose new instances run other code to set their vars
	pub init_procs: Vec<HotReloadInitProc>,
	pub line_moves: Vec<HotReloadLines>
}

impl HotReloadRequest {
	/// How many things it changes
	pub const fn len(&self) -> usize {
		self.new_types.len()
			+ self.procs.len()
			+ self.settings.len()
			+ self.new_procs.len()
			+ self.removed_procs.len()
			+ self.new_vars.len()
			+ self.redeclared.len()
			+ self.var_defaults.len()
			+ self.builtin_defaults.len()
			+ self.init_procs.len()
			+ self.line_moves.len()
	}

	pub const fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

/// New code for a proc, as dmasm's proc compiler produced it
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadProc {
	pub proc: ProcRef,
	/// The body, with DbgFile and DbgLine. They're dropped for a world built
	/// without them.
	pub nodes: Vec<dmasm::Node>,
	/// Local variable names by slot
	pub locals: Vec<String>,
	pub params: Vec<dmasm::reload::Param>,
	/// None if the proc settings are unchanged.
	pub settings: Option<dmasm::reload::ProcSettings>,
	/// Static locals to allocate if missing from the running proc.
	pub statics: Vec<dmasm::reload::StaticVar>
}

/// A proc the world doesn't have yet. `code.proc` is how it will be known.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadNewProc {
	/// The type it's on, `""` for a global proc
	pub type_path: String,
	pub name: String,
	/// Declared on the type (`proc/name`), not overriding a parent's proc
	pub declared: bool,
	/// On the type's list of verbs, not of procs
	pub verb: bool,
	pub code: HotReloadProc
}

/// A new default for a var, on the type and each subtype that inherits it.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadVarDefault {
	pub var: String,
	pub value: dmasm::reload::ConstValue,
	pub types: Vec<String>
}

/// Initializer for computed defaults on new instances, including
/// inherited initializers.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadInitProc {
	pub type_path: String,
	/// `None` when there's nothing left for it to set
	pub code: Option<HotReloadProc>
}

/// A proc whose code is unchanged but now sits on other lines
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadLines {
	pub proc: ProcRef,
	/// (old line, new line) pairs
	pub lines: Vec<(u32, u32)>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HotReloadResult {
	/// Whether the reload changed runtime state. Errors prevent application.
	pub applied: bool,
	pub errors: Vec<String>,
	pub warnings: Vec<String>
}
