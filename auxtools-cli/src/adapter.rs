//! Translate DAP requests and debug server events.
//!
//! The adapter launches or attaches to a world and reconnects after reboots.
//! Launches without debugging still connect for hot reload.
//!
//! A single event loop handles client messages, connections, source changes
//! and process exits. Breakpoints are retained for reconnection.

use std::{
	collections::{BTreeMap, HashMap, HashSet},
	io::BufReader,
	path::{Path, PathBuf},
	sync::{mpsc, Arc, Mutex},
	time::{Duration, Instant}
};

use auxtools_cli::{
	byond::{self, Server},
	game::{Game, Launch},
	local::{self, Stream},
	server_types::{self as aux, ContinueKind, InstructionRef, ProcRef, Request as AuxRequest, Response as AuxResponse}
};
use dap_types::*;
use serde::Deserialize;
use serde_json::Value;

use crate::{
	code,
	dap::{self, Output},
	debuggee::{Debuggee, LastError, SetBreakpoint, SetBreakpoints, INSTRUCTION_BREAKPOINTS},
	stddef::Stddef
};

mod data;
mod files;
mod journal;

pub use journal::reload_by_another;

/// Exception filter for uncaught runtime errors.
const RUNTIMES: &str = "runtimes";
/// Exception filter for runtime errors inside a `try` block.
const CAUGHT: &str = "caught";
/// Source reference for stddef.dm.
const STDDEF_REFERENCE: i64 = 1;
/// Timeout for the client to disconnect after the world exits.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// Grace period before forcibly terminating the world.
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
/// Source file used for debugger helper procs.
const HELPER_FILE: &str = "auxtools_launcher.dm";

/// Diagnostic for line breakpoints in a world built without debug information.
pub const NO_LINES: &str =
	"the world was compiled without debug information, so it has no line numbers: add #define DEBUG to the .dme and rebuild, or break on a proc";
/// Storage key for function breakpoints.
const FUNCTION_BREAKPOINTS: &str = "<functions>";
/// Progress operations indexed by their client-visible IDs.
const STARTING: &str = "starting";
const READING: &str = "reading";
const RESTARTING: &str = "restarting";
/// Retry interval while waiting for the debug server to listen.
const CONNECT_INTERVAL: Duration = Duration::from_millis(100);

/// Events handled by the adapter.
pub enum Input {
	/// A message from the client
	Message(String),
	StdinClosed,
	/// Connected to the world's debug server.
	Connected {
		generation: u64,
		stream: Stream
	},
	ConnectionClosed {
		generation: u64,
		reason: String
	},
	/// The world announced shutdown rather than reboot.
	ShuttingDown {
		generation: u64
	},
	Code(code::Event),
	/// Another client hot reloaded the world
	CodeChanged {
		generation: u64
	},
	/// A data breakpoint's target no longer exists.
	WatchEnded {
		generation: u64,
		id: u32,
		reason: String
	},
	/// The world paused
	Stopped {
		generation: u64,
		stop: aux::Stop
	},
	/// Another client resumed execution.
	Resumed {
		generation: u64
	},
	/// A breakpoint with a message was hit
	Logged {
		message: String,
		file: Option<String>,
		line: Option<u32>
	},
	/// Output from the launched world, for the journal.
	WorldOutput(Vec<String>),
	/// The world the adapter launched exited
	GameExited(Option<i32>),
	/// The process attached to is gone
	ProcessGone,
	/// Shutdown grace period expired for `process`.
	TerminateOverdue {
		process: u32
	},
	/// Disconnect timeout expired.
	Quit
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArguments {
	#[serde(default)]
	no_debug: bool,
	/// Project environment used for breakpoints, stack traces and hot reload.
	dme: Option<PathBuf>,
	/// World to launch, if its path differs from the `.dme` path with a `.dmb`
	/// extension.
	dmb: Option<PathBuf>,
	server: Option<Server>,
	/// Optional BYOND installation path.
	byond_path: Option<PathBuf>,
	/// Build defines (`NAME` or `NAME=VALUE`) used to parse the source.
	#[serde(default)]
	defines: Vec<String>,
	/// Swap changed code into the world as files are saved
	#[serde(default = "yes")]
	hot_reload: bool,
	/// Command-line arguments after the `.dmb`, replacing the defaults.
	args: Option<Vec<String>>,
	#[serde(default)]
	env: HashMap<String, String>,
	/// Debug library path; defaults to the library beside the launcher.
	debug_library: Option<PathBuf>,
	/// Maximum sleeping procs to include in the thread list.
	sleeping_threads: Option<u32>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachArguments {
	/// World process ID, as a number or numeric string.
	process_id: Value,
	/// Project environment, if it is not beside the running `.dmb`.
	dme: Option<PathBuf>,
	#[serde(default)]
	defines: Vec<String>,
	/// Disabled by default to avoid competing with the launching session's
	/// reloads.
	#[serde(default)]
	hot_reload: bool,
	/// Maximum sleeping procs to include in the thread list.
	sleeping_threads: Option<u32>
}

/// Client capabilities used by this adapter.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClientAbilities {
	/// Supports progress events.
	#[serde(default)]
	supports_progress_reporting: bool,
	/// Refreshes data named by `invalidated` events.
	#[serde(default)]
	supports_invalidated_event: bool
}

/// Arguments for `completions`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionsArguments {
	frame_id: Option<i64>,
	text: String,
	column: i64
}

/// Arguments for `breakpointLocations`, which dap-types does not define.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BreakpointLocationsArguments {
	source: Source,
	line: i64,
	end_line: Option<i64>
}

/// Arguments for `setVariable`, which dap-types does not define.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetVariableArguments {
	variables_reference: i64,
	name: String,
	value: String,
	format: Option<ValueFormat>
}

/// Arguments for `setExpression`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetExpressionArguments {
	expression: String,
	value: String,
	frame_id: Option<i64>,
	format: Option<ValueFormat>
}

/// Arguments for the `auxtoolsUnsaved` extension request.
/// DAP does not report unsaved files, so the VS Code extension sends updates.
#[derive(Deserialize)]
struct UnsavedArguments {
	path: String,
	unsaved: bool
}

/// Exception filters and optional conditions.
/// The dap-types definition supports only filters.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionBreakpointsArguments {
	#[serde(default)]
	filters: Vec<String>,
	#[serde(default)]
	filter_options: Vec<ExceptionFilterOption>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionFilterOption {
	filter_id: String,
	condition: Option<String>
}

/// Arguments for `next` and `stepIn`, including granularity missing from
/// dap-types.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StepArguments {
	thread_id: i64,
	/// `instruction`, or `statement`/`line` for line stepping.
	granularity: Option<String>
}

impl StepArguments {
	fn by_instruction(&self) -> bool {
		self.granularity.as_deref() == Some("instruction")
	}
}

/// Arguments for `setInstructionBreakpoints`.
#[derive(Deserialize)]
struct InstructionBreakpointsArguments {
	breakpoints: Vec<InstructionBreakpoint>
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InstructionBreakpoint {
	/// Instruction reference from disassembly or a stack frame.
	instruction_reference: String,
	/// Offset from the reference, in words.
	offset: Option<i64>,
	condition: Option<String>,
	hit_condition: Option<String>
}

const fn yes() -> bool {
	true
}

type Handled = Result<Value, String>;

/// Project source path, once resolved.
struct CodeSettings {
	dme: Option<PathBuf>,
	defines: Vec<String>,
	hot_reload: bool
}

pub struct Adapter {
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,

	/// Whether the client debugs, rather than only running the world
	debugging: bool,
	/// The world's process, once launched or attached to
	pid: Option<u32>,
	/// The world the adapter launched
	game: Option<Game>,

	/// Parse options retained until the first connection.
	code_settings: Option<CodeSettings>,
	/// Source worker thread, while active.
	code: Option<mpsc::Sender<code::Command>>,
	hot_reload: bool,
	/// Whether the initial source parse has finished, successfully or
	/// otherwise.
	code_read: bool,
	/// Whether the source worker is watching files.
	watching: bool,
	/// Whether initial configuration is complete and `Output::ready` has been
	/// sent.
	configured: bool,
	said_ready: bool,
	stddef: Option<Stddef>,
	/// Last macros sent to the world. Resend on connection in case a reboot
	/// interrupted delivery.
	macros: Option<Vec<aux::Macro>>,

	debuggee: Option<Debuggee>,
	generation: u64,
	/// Termination requested; do not reconnect.
	ending: bool,
	/// Suppress reconnect messages after shutdown until a world responds again.
	shutting_down: bool,
	/// Delay world configuration until the source worker finishes.
	waiting_for_code: bool,
	/// Whether the client has received the initialized event.
	initialized: bool,
	last_error: Arc<Mutex<LastError>>,
	/// Start time of the latest reload request.
	reload_started: Option<Instant>,

	/// Client breakpoints indexed by file, with stable IDs.
	breakpoints: BTreeMap<String, Vec<(i64, SourceBreakpoint)>>,
	/// Client capabilities.
	client: ClientAbilities,
	/// Active client progress indicators.
	progress: Vec<&'static str>,
	/// Debug source names mapped to local files, shared with the output reader.
	files: Arc<Mutex<files::WorldFiles>>,
	/// World output buffered while disconnected.
	waiting_output: Vec<String>,
	/// Source text used to interpret each file's breakpoint lines.
	/// `None` if the file could not be read.
	breakpoint_texts: HashMap<String, Option<Vec<String>>>,
	/// Files the client reports as unsaved.
	unsaved: HashSet<String>,
	/// Last source text sent to the world for each file with breakpoints.
	/// Initially, the text read when the first breakpoint was set.
	world_texts: HashMap<String, Option<Vec<String>>>,
	/// Time of the last source synchronization, used to detect pending edits.
	world_as_of: Option<std::time::SystemTime>,
	/// Files with breakpoints in unsaved editor text; defer until saved.
	deferred: HashSet<String>,
	/// Files with breakpoints in saved text; defer until reloaded.
	waiting: HashSet<String>,
	/// Time the pending reload's source changes were read.
	reload_seen: Option<std::time::SystemTime>,
	/// Files already reported as changed since the world was built.
	said_changed: HashSet<String>,
	next_breakpoint_id: i64,
	function_breakpoints: Vec<FunctionBreakpoint>,
	/// The client's instruction breakpoints, with the ids they were given
	instruction_breakpoints: Vec<(i64, InstructionBreakpoint)>,
	/// The ids of `function_breakpoints`, in their order
	function_breakpoint_ids: Vec<i64>,
	/// Last breakpoint status reported to the client, indexed by ID.
	told: HashMap<i64, (bool, Option<i64>, Option<String>)>,
	/// Runtime error filters for uncaught and caught errors.
	uncaught: Option<aux::RuntimeFilter>,
	caught: Option<aux::RuntimeFilter>,
	/// What's set in the world now, by file (or `FUNCTION_BREAKPOINTS`)
	set: SetBreakpoints,
	/// Maximum sleeping procs to include in the thread list.
	sleeping_threads: u32,
	/// Whether the client has been warned that the thread list is truncated.
	said_more_sleep: bool,
	data: data::DataBreakpoints
}

/// Runs the adapter on stdin and stdout until the client leaves
pub fn run() {
	let (inputs, receiver) = mpsc::channel();

	let stdin_inputs = inputs.clone();
	std::thread::spawn(move || {
		let mut stdin = BufReader::new(std::io::stdin());
		while let Ok(Some(message)) = dap::read_message(&mut stdin) {
			if stdin_inputs.send(Input::Message(message)).is_err() {
				return;
			}
		}
		let _ = stdin_inputs.send(Input::StdinClosed);
	});

	serve(Output::new(), inputs, &receiver);
}

/// Run the event loop until the client disconnects or the world exits.
/// A world launched by this adapter is terminated when the loop ends.
/// In-process callers (`auxtools launch`) supply requests through `inputs`
/// and receive events through `out`.
///
/// Run on the main thread: Linux terminates the child when its launching
/// thread exits.
pub fn serve(out: Output, inputs: mpsc::Sender<Input>, receiver: &mpsc::Receiver<Input>) {
	let mut adapter = Adapter {
		out: Arc::new(out),
		inputs,
		debugging: true,
		pid: None,
		game: None,
		code_settings: None,
		code: None,
		hot_reload: false,
		code_read: false,
		watching: false,
		configured: false,
		said_ready: false,
		stddef: None,
		macros: None,
		debuggee: None,
		generation: 0,
		ending: false,
		shutting_down: false,
		waiting_for_code: false,
		initialized: false,
		last_error: Arc::new(Mutex::new(LastError::default())),
		reload_started: None,
		client: ClientAbilities::default(),
		progress: vec![],
		files: Arc::new(Mutex::new(files::WorldFiles::default())),
		waiting_output: vec![],
		breakpoints: BTreeMap::new(),
		breakpoint_texts: HashMap::new(),
		unsaved: HashSet::new(),
		deferred: HashSet::new(),
		waiting: HashSet::new(),
		reload_seen: None,
		world_texts: HashMap::new(),
		world_as_of: None,
		said_changed: HashSet::new(),
		next_breakpoint_id: 1,
		function_breakpoints: vec![],
		instruction_breakpoints: vec![],
		function_breakpoint_ids: vec![],
		told: HashMap::new(),
		uncaught: Some(aux::RuntimeFilter::default()),
		caught: None,
		set: SetBreakpoints::default(),
		sleeping_threads: aux::DEFAULT_SLEEPING_STACKS,
		said_more_sleep: false,
		data: data::DataBreakpoints::default()
	};

	while let Ok(input) = receiver.recv() {
		if !adapter.handle(input) {
			break;
		}
	}

	if let Some(debuggee) = &mut adapter.debuggee {
		let _ = debuggee.send(&AuxRequest::Disconnect);
	}
	if let Some(game) = adapter.game.take() {
		game.kill();
	}
}

fn arguments<T: for<'de> Deserialize<'de>>(arguments: Option<Value>) -> Result<T, String> {
	serde_json::from_value(arguments.unwrap_or(Value::Null)).map_err(|e| format!("bad arguments: {}", e))
}

fn json(value: impl serde::Serialize) -> Handled {
	serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Adapter {
	/// Returns whether to keep going
	fn handle(&mut self, input: Input) -> bool {
		match input {
			Input::Message(message) => return self.message(&message),
			Input::StdinClosed | Input::Quit => return false,
			Input::Connected { generation, stream } => {
				if generation == self.generation && self.debuggee.is_none() {
					self.connected(stream);
				}
			}
			Input::ConnectionClosed { generation, reason } => self.connection_closed(generation, &reason),
			Input::ShuttingDown { generation } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.shutting_down = true;
				}
			}
			Input::Code(event) => self.code_event(event),
			Input::WorldOutput(lines) => self.journal_output(lines),
			Input::Logged { message, file, line } => {
				// Include the source location so the client can create a link.
				let source = file.map(|file| self.file_source(&file));
				self.out.event(OutputEvent {
					output: message + "\n",
					category: Some("console".to_owned()),
					line: source.as_ref().and(line).map(i64::from),
					source,
					..Default::default()
				});
			}
			// Send after the response to the request that resumed execution.
			Input::Stopped { generation, stop } => {
				if let Some(debuggee) = self.debuggee.as_mut().filter(|d| d.generation == generation) {
					debuggee.stopped_at(stop);
				}
			}
			Input::Resumed { generation } => {
				if let Some(debuggee) = self.debuggee.as_mut().filter(|d| d.generation == generation) {
					debuggee.resumed();
				}
			}
			Input::CodeChanged { generation } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.reapply_breakpoints();
				}
			}
			Input::WatchEnded { generation, id, reason } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.watch_ended(id, &reason);
				}
			}
			Input::GameExited(code) => {
				match code {
					Some(code) => self.out.console(format!("The world exited with code {}", code)),
					None => self.out.console("The world was terminated")
				}
				self.game = None;
				self.ended(code.unwrap_or(-1));
			}
			Input::TerminateOverdue { process } => {
				if self.ending && self.world_process() == Some(process) {
					self.out.console(format!(
						"Shutdown timed out after {} seconds; terminating process {}",
						TERMINATE_GRACE.as_secs(),
						process
					));
					self.kill_world();
				}
			}
			Input::ProcessGone => {
				if self.game.is_none() {
					self.out.console("The world's process has ended");
					self.ended(0);
				}
			}
		}
		true
	}

	/// Report the world's exit before closing the launcher.
	fn ended(&mut self, exit_code: i32) {
		self.pid = None;
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.code = None;
		for id in std::mem::take(&mut self.progress) {
			self.out.event(dap::ProgressEnd { progressId: id.to_owned() });
		}
		self.out.event(TerminatedEvent::default());
		self.out.event(ExitedEvent { exitCode: exit_code.into() });

		let inputs = self.inputs.clone();
		std::thread::spawn(move || {
			std::thread::sleep(EXIT_GRACE);
			let _ = inputs.send(Input::Quit);
		});
	}

	fn message(&mut self, message: &str) -> bool {
		let request: RequestMessage = match serde_json::from_str(message) {
			Ok(request) => request,
			Err(e) => {
				self.out.console(format!("Invalid DAP request: {}", e));
				return true;
			}
		};

		let command = request.command.as_str();
		let args = request.arguments;
		let result = match command {
			"initialize" => {
				self.client = arguments(args).unwrap_or_default();
				self.initialize()
			}
			"launch" => arguments(args).and_then(|args| self.launch(args)),
			"attach" => arguments(args).and_then(|args| self.attach(args)),
			"configurationDone" => self.configuration_done(),
			"setBreakpoints" => arguments(args).and_then(|args| self.set_breakpoints(args)),
			"auxtoolsUnsaved" => arguments(args).and_then(|args| self.unsaved_changed(args)),
			"setFunctionBreakpoints" => arguments(args).and_then(|args| self.set_function_breakpoints(args)),
			"setInstructionBreakpoints" => arguments(args).and_then(|args| self.set_instruction_breakpoints(args)),
			"setExceptionBreakpoints" => arguments(args).and_then(|args| self.set_exception_breakpoints(args)),
			"dataBreakpointInfo" => arguments(args).and_then(|args| self.data_breakpoint_info(args)),
			"setDataBreakpoints" => arguments(args).and_then(|args| self.set_data_breakpoints(args)),
			"threads" => self.threads(),
			"stackTrace" => arguments(args).and_then(|args| self.stack_trace(args)),
			"scopes" => arguments(args).and_then(|args| self.scopes(args)),
			"variables" => arguments(args).and_then(|args| self.variables(args)),
			"continue" => self.resume(ContinueKind::Continue).and_then(|()| {
				json(ContinueResponse {
					allThreadsContinued: Some(true)
				})
			}),
			"next" => arguments::<StepArguments>(args).and_then(|args| {
				let by_instruction = args.by_instruction();
				self.step(
					|stack_id| {
						if by_instruction {
							ContinueKind::StepOverInstruction { stack_id }
						} else {
							ContinueKind::StepOver { stack_id }
						}
					},
					args.thread_id
				)
			}),
			"stepIn" => arguments::<StepArguments>(args).and_then(|args| {
				let by_instruction = args.by_instruction();
				self.step(
					|stack_id| {
						if by_instruction {
							ContinueKind::StepIntoInstruction { stack_id }
						} else {
							ContinueKind::StepInto { stack_id }
						}
					},
					args.thread_id
				)
			}),
			"stepOut" => arguments::<StepOutArguments>(args).and_then(|args| self.step(|stack_id| ContinueKind::StepOut { stack_id }, args.threadId)),
			"pause" => self.pause(),
			"evaluate" => arguments(args).and_then(|args| self.evaluate(args)),
			"completions" => arguments(args).and_then(|args| self.completions(args)),
			"setVariable" => arguments(args).and_then(|args| self.set_variable(args)),
			"setExpression" => arguments(args).and_then(|args| self.set_expression(args)),
			"exceptionInfo" => self.exception_info(),
			"source" => arguments(args).and_then(|args| self.source(args)),
			"breakpointLocations" => arguments(args).and_then(|args| self.breakpoint_locations(args)),
			"loadedSources" => self.loaded_sources(),
			"terminate" => self.terminate(),
			"disassemble" => arguments(args).and_then(|args| self.disassemble(args)),
			"disconnect" => {
				let args: Option<DisconnectArguments> = arguments(args).ok();
				self.disconnect(args.and_then(|args| args.terminateDebuggee));
				self.out.response(request.protocol_message.seq, command, Ok(Value::Null));
				return false;
			}
			_ => Err(format!("unsupported request: {}", command))
		};

		self.out.response(request.protocol_message.seq, command, result);
		true
	}

	fn initialize(&mut self) -> Handled {
		let mut capabilities = json(Capabilities {
			supportsConfigurationDoneRequest: Some(true),
			supportsFunctionBreakpoints: Some(true),
			supportsConditionalBreakpoints: Some(true),
			supportsHitConditionalBreakpoints: Some(true),
			supportsLogPoints: Some(true),
			supportsValueFormattingOptions: Some(true),
			supportsEvaluateForHovers: Some(true),
			supportsSetVariable: Some(true),
			supportsSetExpression: Some(true),
			supportsExceptionInfoRequest: Some(true),
			supportTerminateDebuggee: Some(true),
			supportsTerminateRequest: Some(true),
			supportsBreakpointLocationsRequest: Some(true),
			supportsDataBreakpoints: Some(true),
			supportsLoadedSourcesRequest: Some(true),
			supportsDisassembleRequest: Some(true),
			..Default::default()
		})?;
		capabilities["supportsInstructionBreakpoints"] = true.into();
		capabilities["supportsSteppingGranularity"] = true.into();
		// What dap-types has no fields for
		let condition = "Comma-separated message filters, e.g. \"bounds, null\". Prefix exclusions with !, e.g. \"!libdreamluau\".";
		capabilities["supportsExceptionFilterOptions"] = true.into();
		capabilities["supportsCompletionsRequest"] = true.into();
		capabilities["completionTriggerCharacters"] = serde_json::json!([".", ":"]);
		capabilities["exceptionBreakpointFilters"] = serde_json::json!([
			{
				"filter": RUNTIMES,
				"label": "Runtime errors",
				"description": "Pause on uncaught runtime errors and throws",
				"default": true,
				"supportsCondition": true,
				"conditionDescription": condition
			},
			{
				"filter": CAUGHT,
				"label": "Caught runtime errors",
				"description": "Also pause on runtime errors and throws inside try blocks",
				"default": false,
				"supportsCondition": true,
				"conditionDescription": condition
			}
		]);
		Ok(capabilities)
	}

	fn launch(&mut self, args: LaunchArguments) -> Handled {
		if self.pid.is_some() {
			return Err("a world is already running".to_owned());
		}
		let dme = args.dme.clone().or_else(|| args.dmb.as_ref().map(|dmb| dmb.with_extension("dme")));
		let dmb = args
			.dmb
			.clone()
			.or_else(|| args.dme.as_ref().map(|dme| dme.with_extension("dmb")))
			.ok_or("specify the world to launch with dme or dmb")?;

		let bin = byond::bin_dir(args.byond_path.as_deref())?;
		let server = args.server.unwrap_or_else(Server::default_for_platform);
		let program = bin.join(server.program());
		if !program.is_file() {
			return Err(format!("there's no {} in {}", server.program(), bin.display()));
		}

		if !dmb.is_file() {
			return Err(format!("there's no {}", dmb.display()));
		}

		let library = match args.debug_library {
			Some(library) => library,
			None => default_library()?
		};

		self.debugging = !args.no_debug;
		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
		self.code_settings = Some(CodeSettings {
			dme: dme.filter(|dme| dme.is_file()),
			defines: args.defines,
			hot_reload: args.hot_reload
		});

		let game_args = args.args.unwrap_or_else(|| default_args(server));
		// Debugged worlds wait for breakpoints. Hot reload also requires a wait
		// after reboot; release the initial launch as soon as we connect.
		let launch = Launch {
			program,
			dmb: &dmb,
			args: &game_args,
			env: &args.env,
			debug: Some((library, self.debugging || args.hot_reload))
		};
		let (out, inputs, files) = (self.out.clone(), self.inputs.clone(), self.files.clone());
		let batch = journal::batch_output(self.inputs.clone());
		let game = Game::start(
			launch,
			// Pass BYOND's runtime error location to the client and journal.
			move |category, line| {
				// Exclude the launcher's own diagnostics.
				if category != "console" {
					let _ = batch.send(line.trim_end_matches(['\r', '\n']).to_owned());
				}
				match runtime_place(&line) {
					Some((file, number)) => {
						let path = files.lock().unwrap().path(file);
						out.event(OutputEvent {
							category: Some(category.to_owned()),
							source: Some(source_at(&path)),
							line: Some(number),
							output: line,
							..Default::default()
						});
					}
					None => out.output(category, line)
				}
			},
			move |code| {
				let _ = inputs.send(Input::GameExited(code));
			}
		)?;
		self.pid = Some(game.pid());
		self.game = Some(game);
		self.out.event(dap::ProcessEvent {
			name: dmb.to_string_lossy().into_owned(),
			systemProcessId: self.pid.unwrap_or_default(),
			isLocalProcess: true,
			startMethod: "launch".to_owned()
		});
		self.progress_start(STARTING, "Starting the world".to_owned());
		self.started(None);
		Ok(Value::Null)
	}

	fn attach(&mut self, args: AttachArguments) -> Handled {
		if self.pid.is_some() {
			return Err("a world is already running".to_owned());
		}
		let pid = match &args.process_id {
			Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
			Value::String(s) => s.trim().parse().ok(),
			_ => None
		}
		.ok_or("specify processId to attach")?;
		if !local::alive(pid) {
			return Err(format!("process not found: {}", pid));
		}
		// The process exists, so try connecting immediately.
		let stream = local::connect(pid).map_err(|e| {
			format!(
				"process {} isn't a world with this version of auxtools' debugger in it (protocol {}): {}",
				pid,
				aux::PROTOCOL_VERSION,
				e
			)
		})?;

		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
		self.code_settings = Some(CodeSettings {
			dme: args.dme,
			defines: args.defines,
			hot_reload: args.hot_reload
		});
		self.pid = Some(pid);
		self.out.console(format!("Attaching to process {}", pid));
		self.out.event(dap::ProcessEvent {
			name: format!("process {}", pid),
			systemProcessId: pid,
			isLocalProcess: true,
			startMethod: "attach".to_owned()
		});
		self.started(Some(stream));
		Ok(Value::Null)
	}

	/// The world is running: connect to it, unless that's done
	fn started(&mut self, stream: Option<Stream>) {
		// Without debugging, there are no breakpoints to wait for
		if !self.debugging {
			self.initialized = true;
			self.out.event(InitializedEvent);
		}
		match stream {
			// Handled once the client has its answer
			Some(stream) => {
				self.generation += 1;
				let _ = self.inputs.send(Input::Connected {
					generation: self.generation,
					stream
				});
			}
			None => self.connect()
		}
	}

	/// Connects to the world when it listens, in the background
	fn connect(&mut self) {
		let Some(pid) = self.pid else {
			return;
		};
		self.generation += 1;
		let generation = self.generation;
		let inputs = self.inputs.clone();
		let launched = self.game.is_some();
		std::thread::Builder::new()
			.name("connect".to_owned())
			.spawn(move || loop {
				// The world isn't listening while it loads, or reboots
				if let Ok(stream) = local::connect(pid) {
					let _ = inputs.send(Input::Connected { generation, stream });
					return;
				}
				// The launched world's exit is heard about anyway
				if !launched && !local::alive(pid) {
					let _ = inputs.send(Input::ProcessGone);
					return;
				}
				std::thread::sleep(CONNECT_INTERVAL);
			})
			.expect("couldn't start a thread");
	}

	/// Start a progress indicator if the client supports it.
	fn progress_start(&mut self, id: &'static str, title: String) {
		if self.client.supports_progress_reporting && !self.progress.contains(&id) {
			self.progress.push(id);
			self.out.event(dap::ProgressStart {
				progressId: id.to_owned(),
				title
			});
		}
	}

	fn progress_end(&mut self, id: &'static str) {
		if let Some(at) = self.progress.iter().position(|shown| *shown == id) {
			self.progress.remove(at);
			self.out.event(dap::ProgressEnd { progressId: id.to_owned() });
		}
	}

	fn connected(&mut self, stream: Stream) {
		self.progress_end(STARTING);
		self.progress_end(RESTARTING);
		let debuggee = Debuggee::new(
			stream,
			self.generation,
			self.debugging,
			self.out.clone(),
			self.inputs.clone(),
			self.last_error.clone(),
			self.set.clone()
		);
		match debuggee {
			Ok(debuggee) => self.debuggee = Some(debuggee),
			Err(e) => {
				self.out.console(format!("Debug server connection failed: {}", e));
				return;
			}
		}
		self.set.lock().unwrap().clear();
		self.watches_lost();

		// It hangs up straight away if it's on its way to a reboot
		let byond = match self.request(&AuxRequest::ByondVersion) {
			Ok(AuxResponse::ByondVersion { version, build }) => (version, build),
			_ => return
		};
		// A responding world either recovered from failed shutdown or started
		// anew.
		self.shutting_down = false;
		self.send_output();
		self.give_macros();
		self.follow_reloads();
		// This session only launches and reloads; preserve pauses owned by
		// other clients.
		if !self.debugging {
			let _ = self.ack(&AuxRequest::Watcher);
		}
		if self.stddef.is_none() {
			if let Ok(AuxResponse::StdDef(Some(text))) = self.request(&AuxRequest::StdDef) {
				self.stddef = Some(Stddef::new(text));
			}
		}

		// Prevent multiple sessions from reloading the same change.
		let reloads = match &self.code_settings {
			Some(settings) => settings.hot_reload,
			None => self.hot_reload && self.code.is_some()
		};
		if reloads && !self.claim_reloads() {
			match &mut self.code_settings {
				Some(settings) => settings.hot_reload = false,
				// Stop watching if another session claimed reloads during
				// disconnection.
				None => {
					self.hot_reload = false;
					self.code = None;
				}
			}
		}

		match self.code_settings.take() {
			Some(settings) => self.start_code(settings),
			None => self.out.console("The world restarted")
		}
		self.waiting_for_code = match &self.code {
			// A world that restarted waits for its reloads again
			Some(code) => {
				let world = self.generation;
				code.send(code::Command::Connected { byond, world }).is_ok() && self.code_read
			}
			None => false
		};
		if !self.waiting_for_code {
			self.ready();
		}
	}

	/// Claim hot reload ownership. Report an existing owner and return false if
	/// denied.
	fn claim_reloads(&mut self) -> bool {
		let who = match self.game {
			Some(_) => format!("the launcher (pid {})", std::process::id()),
			None => format!("another debugger session (pid {})", std::process::id())
		};
		match self.request(&AuxRequest::Reloader { who }) {
			Ok(AuxResponse::Reloader(Err(why))) => {
				self.out.console(format!("Hot reload disabled: {}", why));
				false
			}
			_ => true
		}
	}

	/// Resolve and parse the project source for hot reload.
	/// The world supplies proc locations independently.
	fn start_code(&mut self, settings: CodeSettings) {
		let dme = settings.dme.or_else(|| {
			// The world's command line says which .dmb it runs
			let AuxResponse::WorldInfo { args, cwd } = self.request(&AuxRequest::WorldInfo).ok()? else {
				return None;
			};
			Some(local::world_dmb(&args, &cwd)?.with_extension("dme"))
		});
		*self.files.lock().unwrap() = files::WorldFiles::new(dme.as_ref().and_then(|dme| std::path::absolute(dme).ok()));
		self.hot_reload = settings.hot_reload;
		if !settings.hot_reload {
			self.code_read = true;
			return;
		}
		let Some(dme) = dme.filter(|dme| dme.is_file()) else {
			self.out.console("Hot reload disabled: project .dme not found");
			self.code_read = true;
			return;
		};

		let options = code::Options {
			dme,
			defines: settings.defines.iter().map(|define| code::parse_define(define)).collect(),
			hot_reload: settings.hot_reload
		};
		self.progress_start(READING, "Parsing project source".to_owned());
		self.code = Some(code::start(options, self.out.clone(), self.inputs.clone()));
	}

	fn code_event(&mut self, event: code::Event) {
		// The initial parse has finished, successfully or otherwise.
		if !matches!(event, code::Event::Reload { .. }) {
			self.progress_end(READING);
		}
		match event {
			code::Event::Changed => {
				// The world now runs the source text captured for this reload.
				self.world_as_of = self.reload_seen.take().or_else(|| Some(std::time::SystemTime::now()));
				// Reapply breakpoints using the client's current line
				// positions.
				if self.code_read && !self.waiting_for_code && self.debuggee.is_some() && self.debugging {
					self.reapply_breakpoints();
				}
				self.code_read = true;
			}
			code::Event::Failed(e) => {
				self.out.console(format!("Hot reload disabled: could not parse source: {}", e));
				self.code_read = true;
				self.code = None;
				self.say_ready();
				if self.waiting_for_code {
					self.waiting_for_code = false;
					self.ready();
				}
			}
			code::Event::Ready => {
				if self.waiting_for_code {
					self.waiting_for_code = false;
					self.ready();
				}
			}
			code::Event::Watching => {
				self.watching = true;
				self.say_ready();
			}
			code::Event::Reload {
				request,
				summary,
				notes,
				world
			} => {
				// A reboot invalidates pending reloads: a rebuilt .dmb may
				// already contain the changes. The source worker will use the
				// new baseline.
				let applied = if self.debuggee.as_ref().is_some_and(|d| d.generation != world) {
					self.out
						.console(format!("Hot reload skipped: prepared before the world restarted: {}", summary.text(&[])));
					false
				} else {
					self.reload_seen = Some(std::time::SystemTime::now());
					self.reload(*request, &summary, notes)
				};
				if let Some(code) = &self.code {
					let _ = code.send(code::Command::Reloaded(applied));
				}
			}
			code::Event::NotReloaded { summary, errors, warnings } => self.journal_reload(false, summary, errors, warnings),
			code::Event::NothingChanged { warnings } => self.journal_reload(true, "no runtime changes".to_owned(), vec![], warnings),
			code::Event::Macros(macros) => {
				self.macros = Some(macros);
				self.give_macros();
			}
		}
	}

	/// Gives the world the code's macros, if they've been read
	fn give_macros(&mut self) {
		if let (Some(macros), Some(debuggee)) = (&self.macros, &mut self.debuggee) {
			let _ = debuggee.send(&AuxRequest::Defines(macros.clone()));
		}
	}

	/// Apply a reload and journal the result, including `notes`.
	/// Returns whether it succeeded.
	fn reload(&mut self, request: aux::HotReloadRequest, summary: &code::Summary, notes: Vec<String>) -> bool {
		self.reload_started = Some(Instant::now());
		let result = match self.request(&AuxRequest::HotReload(request)) {
			Ok(AuxResponse::HotReload(result)) => result,
			Ok(other) => {
				let why = unexpected(other);
				self.notes(&notes);
				self.out.console(format!("Hot reload failed: {}", why));
				self.journal_reload(false, "reload failed".to_owned(), vec![why], notes);
				return false;
			}
			Err(e) => {
				self.notes(&notes);
				self.out.console(format!("Hot reload failed: {}", e));
				self.journal_reload(false, "reload failed".to_owned(), vec![e], notes);
				return false;
			}
		};
		// Exclude removals for procs absent from the running world.
		let summary = summary.text(&result.skipped);
		let notes: Vec<String> = notes
			.into_iter()
			.filter(|note| {
				note.strip_suffix(code::RETAINED)
					.and_then(|note| note.strip_suffix(": a definition"))
					.is_none_or(|path| !result.skipped.iter().any(|skipped| skipped == path))
			})
			.collect();
		self.notes(&notes);
		let summary_of_outcome = if result.applied {
			summary.clone()
		} else {
			format!("reload failed ({})", summary)
		};
		self.journal_reload(
			result.applied,
			summary_of_outcome,
			result.errors.clone(),
			notes.into_iter().chain(result.warnings.iter().cloned()).collect()
		);

		let took = self.reload_started.take().map_or(0, |started| started.elapsed().as_millis());
		if result.applied {
			self.out.console(format!("Hot reload: {} in {} ms", summary, took));
		} else {
			self.out.console("Hot reload failed:");
		}
		for e in &result.errors {
			self.out.console(format!("    {}", e));
		}
		for w in &result.warnings {
			self.out.console(format!("    warning: {}", w));
		}
		result.applied
	}

	/// Print source-worker reload notes.
	fn notes(&self, notes: &[String]) {
		for why in notes {
			self.out.console(format!("    note: {}", why));
		}
	}

	/// The world has the code it should: give it the breakpoints and let it run
	fn ready(&mut self) {
		if !self.initialized {
			// The client sets its breakpoints, then says it's done
			self.initialized = true;
			self.out.event(InitializedEvent);
			return;
		}
		self.configure();
	}

	/// Gives the world the client's breakpoints and lets it run
	fn configure(&mut self) {
		if self.debugging {
			let _ = self.catch_runtimes();
			// Those on lines of unsaved text wait for it to be saved
			let files: Vec<String> = self.breakpoints.keys().filter(|file| !self.deferred.contains(*file)).cloned().collect();
			// Report breakpoints whose status or position changed after
			// reconnection.
			for file in files {
				let breakpoints = self.apply_breakpoints(&file);
				self.tell(breakpoints, false);
			}
			let breakpoints = self.apply_function_breakpoints();
			self.tell(breakpoints, false);
			self.rewatch();
			let _ = self.apply_instruction_breakpoints();
		}
		let _ = self.ack(&AuxRequest::Configured);
		self.catch_up_on_pause();
		self.configured = true;
		self.say_ready();
	}

	/// Report readiness once the world is running and source watching has
	/// started.
	fn say_ready(&mut self) {
		if !self.said_ready && self.configured && (self.code.is_none() || self.watching) {
			self.said_ready = true;
			self.out.ready();
		}
	}

	/// Tells the client if the world was paused before this connected
	fn catch_up_on_pause(&mut self) {
		if self.debugging {
			if let Some(debuggee) = &mut self.debuggee {
				let _ = debuggee.catch_up_on_pause();
			}
		}
	}

	/// Remap breakpoints after source changes and notify the client.
	/// Breakpoint lines refer to the source text captured when they were set.
	fn reapply_breakpoints(&mut self) {
		let files: Vec<String> = self.breakpoints.keys().cloned().collect();
		for file in files {
			let now = crate::lines::read(&file);
			let had = self.world_texts.insert(file.clone(), now.clone());
			// Those on lines of unsaved text are set when it's saved: the
			// file's text now isn't what their lines are of
			if self.deferred.contains(&file) {
				continue;
			}
			let then = self.breakpoint_texts.get(&file).cloned().flatten();
			if let (Some(then), Some(now)) = (&then, &now) {
				if then != now {
					let map = crate::lines::LineMap::new(then, now);
					for (_, breakpoint) in self.breakpoints.get_mut(&file).into_iter().flatten() {
						breakpoint.line = map.line(breakpoint.line as u32).into();
					}
				}
			}
			// Preserve breakpoints and hit counts for unchanged files, unless
			// they were deferred until this version of the source was loaded.
			if then == now && had.as_ref() == Some(&now) && !self.waiting.contains(&file) {
				continue;
			}
			self.waiting.remove(&file);
			self.breakpoint_texts.insert(file.clone(), now);

			let breakpoints = self.apply_breakpoints(&file);
			self.tell(breakpoints, true);
		}

		// A paused world's frames may be on other lines now
		if self.client.supports_invalidated_event && self.debuggee.as_ref().is_some_and(Debuggee::stopped) {
			self.out.event(dap::InvalidatedEvent {
				areas: vec!["stacks".to_owned()]
			});
		}
	}

	fn connection_closed(&mut self, generation: u64, reason: &str) {
		if self.debuggee.as_ref().is_none_or(|d| d.generation != generation) {
			return;
		}
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();

		// A world that's ending shuts its debugger down on the way
		if self.pid.is_some() && !self.ending {
			// Shutdown ends the session when the process exits. Suppress
			// reconnect messages while its socket still accepts and immediately
			// closes connections.
			if !self.shutting_down {
				self.out.console(format!("Debug server disconnected ({}); waiting to reconnect", reason));
				self.progress_start(RESTARTING, "Waiting to reconnect".to_owned());
			}
			self.connect();
		}
	}

	fn disconnect(&mut self, terminate: Option<bool>) {
		// Preserve the pause if other debugging clients remain; otherwise
		// resume.
		if let Some(debuggee) = &mut self.debuggee {
			debuggee.leave();
		}
		self.debuggee = None;
		// A launched world depends on this adapter to read its output.
		if terminate == Some(false) && self.game.is_some() {
			self.out
				.console("Stopping the world launched by this session. Attach to an existing world to leave it running on disconnect.");
		}
		// A world that was attached to carries on unless asked otherwise
		if terminate.unwrap_or(false) || self.game.is_some() {
			if let Some(process) = self.world_process() {
				self.out.console(format!("Ending the world (process {})", process));
				self.kill_world();
			}
		}
		// Stop trying to connect
		self.pid = None;
		self.generation += 1;
	}

	/// Process ID of the launched or attached world.
	fn world_process(&self) -> Option<u32> {
		self.game.as_ref().map(Game::pid).or(self.pid)
	}

	/// Terminate the world immediately.
	fn kill_world(&mut self) {
		if let Some(game) = &self.game {
			game.kill();
		} else if let Some(pid) = self.pid {
			local::kill(pid);
		}
	}

	/// Request shutdown through `world/Del()`, then terminate the process if
	/// it exceeds the grace period.
	fn terminate(&mut self) -> Handled {
		let Some(process) = self.world_process() else {
			return Ok(Value::Null);
		};
		if self.ending {
			return Ok(Value::Null);
		}
		self.ending = true;
		// Do not wait for a response: successful shutdown closes the
		// connection.
		let asked = self
			.debuggee
			.as_mut()
			.is_some_and(|debuggee| debuggee.eval_unanswered("world.Del()".to_owned()).is_ok());
		if asked {
			self.out.console("Requested shutdown through world/Del()");
			let inputs = self.inputs.clone();
			let launched = self.game.is_some();
			std::thread::spawn(move || {
				let deadline = std::time::Instant::now() + TERMINATE_GRACE;
				let mut overdue = false;
				// The launched world's exit is heard about anyway
				while launched || local::alive(process) {
					if !overdue && std::time::Instant::now() >= deadline {
						overdue = true;
						if inputs.send(Input::TerminateOverdue { process }).is_err() || launched {
							return;
						}
					}
					std::thread::sleep(CONNECT_INTERVAL);
				}
				let _ = inputs.send(Input::ProcessGone);
			});
		} else {
			self.out.console(format!("Debug server disconnected; terminating process {}", process));
			self.kill_world();
		}
		Ok(Value::Null)
	}

	fn debuggee(&mut self) -> Result<&mut Debuggee, String> {
		self.debuggee.as_mut().ok_or_else(|| "debug server disconnected".to_owned())
	}

	fn request(&mut self, request: &AuxRequest) -> Result<AuxResponse, String> {
		self.debuggee()?.request(request)
	}

	/// A request the server answers with `Ack`
	fn ack(&mut self, request: &AuxRequest) -> Result<(), String> {
		match self.request(request)? {
			AuxResponse::Ack => Ok(()),
			other => Err(unexpected(other))
		}
	}

	fn configuration_done(&mut self) -> Handled {
		if self.debugging && self.debuggee.is_some() {
			// A client that said nothing about runtime errors has the
			// filter's default, here as in the worlds after a reboot
			self.catch_runtimes()?;
			self.ack(&AuxRequest::Configured)?;
		}
		self.catch_up_on_pause();
		Ok(Value::Null)
	}

	fn set_breakpoints(&mut self, args: SetBreakpointsArguments) -> Handled {
		let path = args.source.path.ok_or("breakpoint source is missing a path")?;
		// VS Code sends updated breakpoint lines after saving. They refer
		// to the current disk contents.
		let saved_since = args.sourceModified.unwrap_or(false);
		let reloading = self.code.is_some();
		for (id, _) in self.breakpoints.get(&path).into_iter().flatten() {
			self.told.remove(id);
		}
		let mut wanted = vec![];
		for breakpoint in args.breakpoints.unwrap_or_default() {
			wanted.push((self.next_breakpoint_id, breakpoint));
			self.next_breakpoint_id += 1;
		}

		// Defer breakpoints in unsaved editor text until the file is saved
		// and reloaded. Only clients using `auxtoolsUnsaved` report this state.
		// Other clients' line numbers refer to disk contents. Without hot
		// reload, set breakpoints against the running code immediately.
		if !saved_since && reloading && self.unsaved.contains(&path) {
			let breakpoints = wanted
				.iter()
				.map(|(number, breakpoint)| Breakpoint {
					id: Some(*number),
					verified: false,
					message: Some("pending save and hot reload".to_owned()),
					line: Some(breakpoint.line),
					..Default::default()
				})
				.collect::<Vec<_>>();
			self.told(&breakpoints);
			// Keep existing breakpoints in the running code until reload.
			// An empty replacement list removes them immediately.
			if wanted.is_empty() {
				self.unset(&path);
			}
			self.world_texts.entry(path.clone()).or_insert_with(|| crate::lines::read(&path));
			self.breakpoints.insert(path.clone(), wanted);
			self.breakpoint_texts.insert(path.clone(), None);
			self.deferred.insert(path);
			return json(SetBreakpointsResponse { breakpoints });
		}

		if saved_since && !reloading && self.said_changed.insert(path.clone()) {
			self.out.console(format!(
				"{} has changed since the world was built: without hot reloading, its breakpoints go by the code the world has, and may be on other \
				 lines",
				Path::new(&path).file_name().unwrap_or_default().to_string_lossy()
			));
		}
		self.deferred.remove(&path);
		let text = crate::lines::read(&path);
		// These lines refer to saved code that has not been reloaded yet.
		// Defer installation so breakpoints are not assigned to unrelated
		// statements in the old code.
		if self.world_is_behind(&path, &text) {
			let breakpoints = Self::waiting_for_the_world(&wanted);
			self.told(&breakpoints);
			if wanted.is_empty() {
				self.unset(&path);
			}
			self.breakpoints.insert(path.clone(), wanted);
			self.breakpoint_texts.insert(path.clone(), text);
			self.waiting.insert(path);
			return json(SetBreakpointsResponse { breakpoints });
		}
		self.waiting.remove(&path);
		// The first time, the world is taken to have what's on disk
		self.world_texts.entry(path.clone()).or_insert_with(|| text.clone());
		self.breakpoints.insert(path.clone(), wanted);
		self.breakpoint_texts.insert(path.clone(), text);
		let breakpoints = self.apply_breakpoints(&path);
		self.told(&breakpoints);
		json(SetBreakpointsResponse { breakpoints })
	}

	/// Whether the saved file contains changes not yet reloaded.
	/// For files without a stored baseline, compare the modification time
	/// with the last source synchronization. Always false without hot reload.
	fn world_is_behind(&self, path: &str, text: &Option<Vec<String>>) -> bool {
		if self.code.is_none() || !self.code_read {
			return false;
		}
		match self.world_texts.get(path) {
			Some(world) => world != text,
			None => {
				let modified = std::fs::metadata(path).and_then(|file| file.modified()).ok();
				matches!((modified, self.world_as_of), (Some(modified), Some(as_of)) if modified > as_of)
			}
		}
	}

	/// Report breakpoints deferred until reload.
	fn waiting_for_the_world(wanted: &[(i64, SourceBreakpoint)]) -> Vec<Breakpoint> {
		wanted
			.iter()
			.map(|(number, breakpoint)| Breakpoint {
				id: Some(*number),
				verified: false,
				message: Some("pending hot reload".to_owned()),
				line: Some(breakpoint.line),
				..Default::default()
			})
			.collect()
	}

	/// Record the breakpoint status sent in a response.
	fn told(&mut self, breakpoints: &[Breakpoint]) {
		for breakpoint in breakpoints {
			if let Some(id) = breakpoint.id {
				self.told.insert(id, (breakpoint.verified, breakpoint.line, breakpoint.message.clone()));
			}
		}
	}

	/// Report breakpoint status changes, or every status if `all` is set.
	fn tell(&mut self, breakpoints: Vec<Breakpoint>, all: bool) {
		for breakpoint in breakpoints {
			if let Some(id) = breakpoint.id {
				let now = (breakpoint.verified, breakpoint.line, breakpoint.message.clone());
				if !all && self.told.get(&id) == Some(&now) {
					continue;
				}
				self.told.insert(id, now);
			}
			self.out.event(dap::BreakpointEvent {
				reason: "changed".to_owned(),
				breakpoint
			});
		}
	}

	/// Update a file's unsaved state. On save, pending breakpoint lines refer
	/// to disk contents. Install them immediately unless the world is behind.
	fn unsaved_changed(&mut self, args: UnsavedArguments) -> Handled {
		let path = args.path;
		if args.unsaved {
			self.unsaved.insert(path);
			return Ok(Value::Null);
		}
		self.unsaved.remove(&path);
		if self.deferred.remove(&path) && self.breakpoints.contains_key(&path) {
			let text = crate::lines::read(&path);
			let behind = self.world_is_behind(&path, &text);
			self.breakpoint_texts.insert(path.clone(), text);
			// If the world doesn't have the change yet, they're set when
			// it does
			if behind {
				self.waiting.insert(path.clone());
				let breakpoints = Self::waiting_for_the_world(self.breakpoints.get(&path).map_or(&[], Vec::as_slice));
				self.tell(breakpoints, false);
				return Ok(Value::Null);
			}
			let breakpoints = self.apply_breakpoints(&path);
			self.tell(breakpoints, true);
		}
		Ok(Value::Null)
	}

	/// Resolve the file's debug name, usually relative to the project root.
	/// The server uses suffix matching as a fallback.
	fn world_file_name(&self, path: &str) -> String {
		self.files.lock().unwrap().name(path)
	}

	/// Sets a file's breakpoints in the world, in place of the ones it had
	fn apply_breakpoints(&mut self, path: &str) -> Vec<Breakpoint> {
		self.unset(path);
		let wanted: Vec<(i64, i64, Option<String>, Option<String>, Option<String>)> = self.breakpoints[path]
			.iter()
			.map(|(number, b)| (*number, b.line, b.condition.clone(), b.hitCondition.clone(), b.logMessage.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (number, line, condition, hit, log) in wanted {
			// Logpoints print their message without pausing.
			let log = log.filter(|message| !message.is_empty()).map(|message| aux::LogPart::parse(&message));
			let unverified = |message: &str| Breakpoint {
				id: Some(number),
				verified: false,
				message: Some(message.to_owned()),
				line: Some(line),
				..Default::default()
			};

			if !self.debugging {
				breakpoints.push(unverified("debugging is disabled"));
				continue;
			}
			if self.debuggee.is_none() {
				breakpoints.push(unverified("world is not running"));
				continue;
			}
			let hit = match hit_condition(hit.as_deref()) {
				Ok(hit) => hit,
				Err(e) => {
					breakpoints.push(unverified(&e));
					continue;
				}
			};

			// The world knows which of its procs have code on the line
			let instructions = match self.request(&AuxRequest::Locate {
				file: self.world_file_name(path),
				line: line as u32
			}) {
				Ok(AuxResponse::Locate { instructions }) if instructions.is_empty() => {
					breakpoints.push(unverified(if self.world_has_lines() {
						"no executable code at or after this line"
					} else {
						NO_LINES
					}));
					continue;
				}
				Ok(AuxResponse::Locate { instructions }) => instructions,
				Ok(other) => {
					breakpoints.push(unverified(&unexpected(other)));
					continue;
				}
				Err(e) => {
					breakpoints.push(unverified(&e));
					continue;
				}
			};

			let mut result = None;
			for instruction in instructions {
				let mut at = None;
				match self.set_breakpoint(&instruction, condition.clone(), hit, log.clone()) {
					Ok(Some((set_line, set_at))) => {
						result.get_or_insert(Ok(set_line));
						at = Some(set_at);
					}
					Ok(None) => {}
					Err(e) => {
						result.get_or_insert(Err(e));
					}
				}
				set.push(SetBreakpoint {
					asked: instruction,
					at,
					watch: None,
					id: number
				});
			}
			breakpoints.push(match result {
				Some(Ok(set_line)) => Breakpoint {
					id: Some(number),
					verified: true,
					line: Some(set_line.unwrap_or(line as u32).into()),
					..Default::default()
				},
				Some(Err(e)) => unverified(&e),
				None => unverified("could not set breakpoint")
			});
		}
		self.set.lock().unwrap().insert(path.to_owned(), set);
		breakpoints
	}

	/// Resolved line if the breakpoint was verified.
	fn set_breakpoint(
		&mut self,
		instruction: &InstructionRef,
		condition: Option<String>,
		hit: Option<aux::HitCondition>,
		log: Option<Vec<aux::LogPart>>
	) -> Result<Option<(Option<u32>, InstructionRef)>, String> {
		self.breakpoint_set(&AuxRequest::BreakpointSet {
			instruction: instruction.clone(),
			condition,
			hit,
			log
		})
	}

	/// Interpret the breakpoint response; returns the verified line, if any.
	fn breakpoint_set(&mut self, request: &AuxRequest) -> Result<Option<(Option<u32>, InstructionRef)>, String> {
		match self.request(request)? {
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Success { line, at, .. }
			} => Ok(Some((line, at))),
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Failed
			} => Ok(None),
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Refused { reason }
			} => Err(reason),
			other => Err(unexpected(other))
		}
	}

	/// Remove breakpoints stored under `key`.
	fn unset(&mut self, key: &str) {
		let set = self.set.lock().unwrap().remove(key).unwrap_or_default();
		for breakpoint in set {
			let instruction = breakpoint.asked;
			let _ = self.request(&if key == INSTRUCTION_BREAKPOINTS {
				AuxRequest::InstructionBreakpointUnset { instruction }
			} else {
				AuxRequest::BreakpointUnset { instruction }
			});
		}
	}

	fn set_function_breakpoints(&mut self, args: SetFunctionBreakpointsArguments) -> Handled {
		for id in &self.function_breakpoint_ids {
			self.told.remove(id);
		}
		// Preserve IDs so the client can identify subsequent updates.
		self.function_breakpoint_ids = (0..args.breakpoints.len() as i64).map(|i| self.next_breakpoint_id + i).collect();
		self.next_breakpoint_id += args.breakpoints.len() as i64;
		self.function_breakpoints = args.breakpoints;
		let breakpoints = self.apply_function_breakpoints();
		self.told(&breakpoints);
		json(SetFunctionBreakpointsResponse { breakpoints })
	}

	/// Set function breakpoints. `/mob/proc/Login#1` selects the second
	/// definition.
	fn apply_function_breakpoints(&mut self) -> Vec<Breakpoint> {
		self.unset(FUNCTION_BREAKPOINTS);
		let wanted: Vec<(i64, String, Option<String>, Option<String>)> = self
			.function_breakpoints
			.iter()
			.zip(&self.function_breakpoint_ids)
			.map(|(b, id)| (*id, b.name.clone(), b.condition.clone(), b.hitCondition.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (id, name, condition, hit) in wanted {
			if !self.debugging || self.debuggee.is_none() {
				breakpoints.push(Breakpoint {
					id: Some(id),
					verified: false,
					message: Some("no active debugging connection".to_owned()),
					..Default::default()
				});
				continue;
			}
			let instruction = InstructionRef {
				proc: named_proc(&name),
				offset: 0
			};
			let set_as_asked = hit_condition(hit.as_deref()).and_then(|hit| self.set_breakpoint(&instruction, condition, hit, None));
			let mut at = None;
			breakpoints.push(match set_as_asked {
				Ok(Some((line, set_at))) => {
					at = Some(set_at);
					Breakpoint {
						id: Some(id),
						verified: true,
						line: line.map(i64::from),
						..Default::default()
					}
				}
				Ok(None) => Breakpoint {
					id: Some(id),
					verified: false,
					message: Some(format!("there's no {}", name)),
					..Default::default()
				},
				Err(e) => Breakpoint {
					id: Some(id),
					verified: false,
					message: Some(e),
					..Default::default()
				}
			});
			set.push(SetBreakpoint {
				asked: instruction,
				at,
				watch: None,
				id
			});
		}
		self.set.lock().unwrap().insert(FUNCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn set_instruction_breakpoints(&mut self, args: InstructionBreakpointsArguments) -> Handled {
		let mut wanted = vec![];
		for breakpoint in args.breakpoints {
			wanted.push((self.next_breakpoint_id, breakpoint));
			self.next_breakpoint_id += 1;
		}
		self.instruction_breakpoints = wanted;
		let breakpoints = self.apply_instruction_breakpoints();
		Ok(serde_json::json!({ "breakpoints": breakpoints }))
	}

	/// Set instruction breakpoints using disassembly addresses.
	/// Addresses must point to instruction boundaries: inserting a breakpoint
	/// inside an instruction would corrupt its operands.
	fn apply_instruction_breakpoints(&mut self) -> Vec<Value> {
		self.unset(INSTRUCTION_BREAKPOINTS);
		let wanted = self.instruction_breakpoints.clone();

		let mut breakpoints = vec![];
		let mut set = vec![];
		let mut code: HashMap<aux::CodeRef, Option<aux::Disassembly>> = HashMap::new();
		for (id, breakpoint) in wanted {
			let reference = breakpoint.instruction_reference.clone();
			let unverified =
				|message: String| serde_json::json!({ "id": id, "verified": false, "message": message, "instructionReference": reference });
			if !self.debugging || self.debuggee.is_none() {
				breakpoints.push(unverified("no active debugging connection".to_owned()));
				continue;
			}
			let Some(address) = parse_address(&reference).map(|address| address.wrapping_add_signed(breakpoint.offset.unwrap_or(0))) else {
				breakpoints.push(unverified(format!("invalid instruction address: {:?}", reference)));
				continue;
			};
			let (place, offset) = aux::from_address(address);
			if place.old != 0 {
				breakpoints.push(unverified("cannot set a breakpoint in pre-reload bytecode".to_owned()));
				continue;
			}
			if !code.contains_key(&place) {
				let disassembly = match self.request(&AuxRequest::Disassemble { code: place }) {
					Ok(AuxResponse::Disassemble(Ok(disassembly))) => Some(disassembly),
					_ => None
				};
				code.insert(place, disassembly);
			}
			let Some(disassembly) = &code[&place] else {
				breakpoints.push(unverified(format!("no proc at address {}", address_text(address))));
				continue;
			};
			let proc = disassembly.proc.clone();
			let starts = disassembly
				.instructions
				.iter()
				.rev()
				.find(|instruction| instruction.offset <= offset)
				.map(|instruction| instruction.offset);
			let past_end = disassembly
				.instructions
				.last()
				.is_none_or(|last| usize::from(offset) >= usize::from(last.offset) + last.words.len());
			match starts {
				_ if past_end => {
					breakpoints.push(unverified(format!("{} is past the end of {}", address_text(address), proc.path)));
					continue;
				}
				Some(start) if start != offset => {
					breakpoints.push(unverified(format!(
						"{} is in the middle of an instruction of {}, which starts at {}",
						address_text(address),
						proc.path,
						address_text(aux::address(place, start))
					)));
					continue;
				}
				_ => {}
			}
			let hit = match hit_condition(breakpoint.hit_condition.as_deref()) {
				Ok(hit) => hit,
				Err(e) => {
					breakpoints.push(unverified(e));
					continue;
				}
			};

			let instruction = InstructionRef { proc, offset: offset.into() };
			let mut at = None;
			breakpoints.push(
				match self.breakpoint_set(&AuxRequest::InstructionBreakpointSet {
					instruction: instruction.clone(),
					condition: breakpoint.condition.clone(),
					hit,
					log: None
				}) {
					Ok(Some((line, set_at))) => {
						at = Some(set_at);
						serde_json::json!({ "id": id, "verified": true, "line": line, "instructionReference": address_text(address) })
					}
					Ok(None) => unverified("could not set breakpoint".to_owned()),
					Err(e) => unverified(e)
				}
			);
			set.push(SetBreakpoint {
				asked: instruction,
				at,
				watch: None,
				id
			});
		}
		self.set.lock().unwrap().insert(INSTRUCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn set_exception_breakpoints(&mut self, args: ExceptionBreakpointsArguments) -> Handled {
		// A filter that's ticked is in one list or the other: with its
		// condition, or without one
		let filter = |id: &str| -> Option<aux::RuntimeFilter> {
			if let Some(option) = args.filter_options.iter().find(|option| option.filter_id == id) {
				return Some(aux::RuntimeFilter::parse(option.condition.as_deref().unwrap_or_default()));
			}
			args.filters.iter().any(|filter| filter == id).then(aux::RuntimeFilter::default)
		};
		self.uncaught = filter(RUNTIMES);
		self.caught = filter(CAUGHT);
		if self.debugging && self.debuggee.is_some() {
			self.catch_runtimes()?;
		}
		Ok(Value::Null)
	}

	/// Tells the world which runtime errors to pause on (it doesn't answer)
	fn catch_runtimes(&mut self) -> Result<(), String> {
		let (uncaught, caught) = (self.uncaught.clone(), self.caught.clone());
		self.debuggee()?.send(&AuxRequest::CatchRuntimes { uncaught, caught })
	}

	/// Details of the current runtime error: message, caught status and stack.
	fn exception_info(&mut self) -> Handled {
		let error = self.last_error.lock().unwrap().clone();
		let frames = match self.request(&AuxRequest::StackFrames {
			stack_id: 0,
			start_frame: None,
			count: None
		}) {
			Ok(AuxResponse::StackFrames { frames, .. }) => frames,
			_ => vec![]
		};
		let trace: Vec<String> = frames
			.iter()
			.map(|frame| match (&frame.file, frame.line) {
				(Some(file), Some(line)) => format!("{} ({}:{})", frame.instruction.proc.path, file, line),
				_ => frame.instruction.proc.path.clone()
			})
			.collect();
		json(ExceptionInfoResponse {
			exceptionId: crate::debuggee::error_kind(error.caught).to_owned(),
			description: Some(error.message.clone()),
			breakMode: if error.caught {
				ExceptionBreakMode::Always
			} else {
				ExceptionBreakMode::Unhandled
			},
			details: Some(ExceptionDetails {
				message: Some(error.message),
				stackTrace: (!trace.is_empty()).then(|| trace.join("\n")),
				..Default::default()
			})
		})
	}

	fn threads(&mut self) -> Handled {
		let mut threads = vec![];
		if self.debuggee.is_some() {
			// The server already excludes the keepalive stack.
			let limit = self.sleeping_threads as usize;
			match self.request(&AuxRequest::Stacks {
				sleeping: Some(self.sleeping_threads)
			})? {
				AuxResponse::Stacks { stacks, sleeping } => {
					let asleep = sleeping as usize;
					let mut listed = 0;
					for stack in stacks {
						if stack.sleeping {
							if listed == limit {
								continue;
							}
							listed += 1;
						}
						threads.push(Thread {
							id: stack.id.into(),
							name: thread_name(&stack)
						});
					}
					if asleep > listed && !self.said_more_sleep {
						self.said_more_sleep = true;
						self.out.console(format!(
							"{} procs are asleep: the list of threads has the first {} (\"sleepingThreads\" in the launch configuration sets how \
							 many)",
							asleep, listed
						));
					}
				}
				other => return Err(unexpected(other))
			}
		}

		// The client only asks to pause a thread it knows of
		if threads.is_empty() {
			threads.push(Thread {
				id: 0,
				name: "Main".to_owned()
			});
		}
		json(ThreadsResponse { threads })
	}

	fn stack_trace(&mut self, args: StackTraceArguments) -> Handled {
		let (frames, total) = match self.request(&AuxRequest::StackFrames {
			stack_id: args.threadId as u32,
			start_frame: args.startFrame.map(|x| x as u32),
			count: args.levels.filter(|&levels| levels > 0).map(|x| x as u32)
		})? {
			AuxResponse::StackFrames { frames, total_count } => (frames, total_count),
			other => return Err(unexpected(other))
		};

		let mut stack_frames = vec![];
		for (i, frame) in frames.into_iter().enumerate() {
			let proc = &frame.instruction.proc;
			let mut dap_frame = StackFrame {
				id: frame.id.into(),
				// Frames running pre-reload bytecode refer to the old source
				// text.
				name: if frame.old_code {
					format!("{} (old code)", proc.path)
				} else {
					proc.path.clone()
				},
				instructionPointerReference: Some(address_text(aux::address(frame.code, frame.instruction.offset as u16))),
				// Has to be more than 0 for the client to show an exception
				column: if i == 0 { 1 } else { 0 },
				..Default::default()
			};

			// The world says which file the proc is in, but for stddef.dm's
			match &frame.file {
				// One of the debug library's own
				Some(file) if file == HELPER_FILE => dap_frame.presentationHint = Some(StackFramePresentationHint::Subtle),
				Some(file) => dap_frame.source = Some(self.file_source(file)),
				None => match self.stddef.as_ref().and_then(|stddef| stddef.line(&proc.path, proc.override_id)) {
					Some(line) => {
						dap_frame.source = Some(Source {
							name: Some("stddef.dm".to_owned()),
							sourceReference: Some(STDDEF_REFERENCE),
							..Default::default()
						});
						dap_frame.line = line.into();
					}
					None => dap_frame.presentationHint = Some(StackFramePresentationHint::Subtle)
				}
			}
			if let Some(line) = frame.line {
				dap_frame.line = line.into();
			}
			stack_frames.push(dap_frame);
		}

		json(StackTraceResponse {
			stackFrames: stack_frames,
			totalFrames: Some(total.into())
		})
	}

	fn scopes(&mut self, args: ScopesArguments) -> Handled {
		let (arguments, locals, globals) = match self.request(&AuxRequest::Scopes {
			frame_id: args.frameId as u32
		})? {
			AuxResponse::Scopes { arguments, locals, globals } => (arguments, locals, globals),
			other => return Err(unexpected(other))
		};
		// Load globals lazily; a world may have thousands.
		let scopes = [
			("Locals", locals, Some("locals")),
			("Arguments", arguments, Some("arguments")),
			("Globals", globals, None)
		]
		.into_iter()
		.filter_map(|(name, vars, hint)| {
			vars.map(|vars| Scope {
				name: name.to_owned(),
				presentationHint: hint.map(str::to_owned),
				variablesReference: vars.0.into(),
				expensive: hint.is_none(),
				..Default::default()
			})
		})
		.collect();
		json(ScopesResponse { scopes })
	}

	fn variables(&mut self, args: VariablesArguments) -> Handled {
		// A client takes a long list a piece at a time
		let piece = |n: Option<i64>| n.and_then(|n| u32::try_from(n).ok());
		let vars = match self.request(&AuxRequest::Variables {
			vars: aux::VariablesRef(args.variablesReference as i32),
			start: piece(args.start),
			count: piece(args.count).filter(|&count| count > 0)
		})? {
			AuxResponse::Variables { vars } => vars,
			other => return Err(unexpected(other))
		};
		let hex = args.format.is_some_and(|format| format.hex == Some(true));

		// Variable names must be unique within a container.
		let mut seen: HashMap<String, u32> = HashMap::new();
		let variables = vars
			.into_iter()
			.map(|var| {
				let count = seen.entry(var.name.clone()).or_default();
				let name = if *count == 0 { var.name } else { format!("{} #{}", var.name, count) };
				*count += 1;
				Variable {
					name,
					value: shown(var.value, hex),
					type_: var.kind,
					evaluateName: var.expr,
					variablesReference: var.variables.map_or(0, |vars| vars.0.into()),
					indexedVariables: var.entries.map(i64::from),
					..Default::default()
				}
			})
			.collect();
		json(VariablesResponse { variables })
	}

	fn pause(&mut self) -> Handled {
		// Without debugging, the client receives no stop events and cannot
		// resume.
		if !self.debugging {
			return Err("debugging is disabled".to_owned());
		}
		self.ack(&AuxRequest::Pause).map(|()| Value::Null)
	}

	fn resume(&mut self, kind: ContinueKind) -> Result<(), String> {
		self.ack(&AuxRequest::Continue { kind })
	}

	fn step(&mut self, kind: impl Fn(u32) -> ContinueKind, thread: i64) -> Handled {
		self.resume(kind(thread as u32)).map(|()| Value::Null)
	}

	fn evaluate(&mut self, args: EvaluateArguments) -> Handled {
		// Hover evaluation must not change the world.
		if args.context.as_deref() == Some("hover") {
			if let Some(why) = hover_refuses(&args.expression) {
				return Err(why.to_owned());
			}
		}
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::Eval {
			frame_id: args.frameId.map(|id| id as u32),
			command: args.expression,
			context: args.context
		})? {
			AuxResponse::Eval(Ok(result)) => json(EvaluateResponse {
				result: shown(result.value, hex),
				type_: result.kind,
				variablesReference: result.variables.map_or(0, |vars| vars.0.into()),
				indexedVariables: result.entries.map(|entries| entries as usize),
				..Default::default()
			}),
			// The client shows it where the value would be
			AuxResponse::Eval(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	/// Complete the expression at the cursor.
	fn completions(&mut self, args: CompletionsArguments) -> Handled {
		// The column counts UTF-16 units, from 1
		let before = args.column.max(1) as usize - 1;
		let cursor = args
			.text
			.char_indices()
			.scan(0, |units, (at, c)| {
				let here = *units;
				*units += c.len_utf16();
				Some((at, here))
			})
			.find(|(_, units)| *units >= before)
			.map_or(args.text.len(), |(at, _)| at);
		let Some(place) = completion_place(&args.text[..cursor]) else {
			return Ok(serde_json::json!({ "targets": [] }));
		};
		let prefix = &args.text[place.start..cursor];
		let (start, length) = (args.text[..place.start].encode_utf16().count() + 1, prefix.encode_utf16().count());
		let target = |label: &str, kind: &str| serde_json::json!({ "label": label, "type": kind, "start": start, "length": length });

		// The debug server's own commands
		if place.command {
			let targets: Vec<Value> = ["dis", "disassemble", "guest_override", "help", "mem_profiler"]
				.into_iter()
				.filter(|command| command.starts_with(prefix))
				.map(|command| target(command, "keyword"))
				.collect();
			return Ok(serde_json::json!({ "targets": targets }));
		}

		match self.request(&AuxRequest::Complete {
			frame_id: args.frame_id.map(|id| id as u32),
			base: place.base.map(str::to_owned),
			prefix: prefix.to_owned()
		})? {
			AuxResponse::Complete(names) => {
				let member = place.base.is_some();
				let targets: Vec<Value> = names
					.iter()
					.map(|name| {
						let kind = match (name.proc, member) {
							(true, true) => "method",
							(true, false) => "function",
							(false, true) => "property",
							(false, false) => "variable"
						};
						target(&name.name, kind)
					})
					.collect();
				Ok(serde_json::json!({ "targets": targets }))
			}
			other => Err(unexpected(other))
		}
	}

	/// Assign an evaluated expression to a variable.
	fn set_variable(&mut self, args: SetVariableArguments) -> Handled {
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::SetVariable {
			vars: aux::VariablesRef(args.variables_reference as i32),
			name: args.name,
			value: args.value
		})? {
			AuxResponse::SetVariable(Ok(variable)) => Ok(serde_json::json!({
				"value": shown(variable.value, hex),
				"type": variable.kind,
				"variablesReference": variable.variables.map_or(0, |vars| vars.0),
				"indexedVariables": variable.entries
			})),
			AuxResponse::SetVariable(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	/// Assign to an expression using DM's assignment operator.
	fn set_expression(&mut self, args: SetExpressionArguments) -> Handled {
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::Eval {
			frame_id: args.frame_id.map(|id| id as u32),
			command: format!("{} = ({})", args.expression, args.value),
			context: None
		})? {
			AuxResponse::Eval(Ok(result)) => Ok(serde_json::json!({
				"value": shown(result.value, hex),
				"type": result.kind,
				"variablesReference": result.variables.map_or(0, |vars| vars.0),
				"indexedVariables": result.entries
			})),
			AuxResponse::Eval(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	/// Return exactly the requested disassembly window.
	/// Mark entries outside the bytecode as invalid instructions.
	fn disassemble(&mut self, args: DisassembleArguments) -> Handled {
		let count = usize::try_from(args.instructionCount).map_err(|_| "the instruction count is negative".to_owned())?;
		let reference = parse_address(&args.memoryReference).ok_or_else(|| format!("invalid instruction address: {:?}", args.memoryReference))?;
		// An address counts words, as the code does
		let reference = reference.wrapping_add_signed(args.offset.unwrap_or(0));
		let (code, offset) = aux::from_address(reference);
		let disassembly = match self.request(&AuxRequest::Disassemble { code })? {
			AuxResponse::Disassemble(Ok(disassembly)) => Some(disassembly),
			// Past the start or the end of some code, there's none
			AuxResponse::Disassemble(Err(_)) => None,
			other => return Err(unexpected(other))
		};

		let empty = vec![];
		let listed = disassembly.as_ref().map_or(&empty, |disassembly| &disassembly.instructions);
		// Old bytecode refers to source text the client no longer has.
		let file = disassembly
			.as_ref()
			.filter(|_| code.old == 0)
			.and_then(|disassembly| disassembly.file.clone());
		let symbol = disassembly.as_ref().map(|disassembly| {
			let path = &disassembly.proc.path;
			if code.old != 0 {
				format!("{} (old code)", path)
			} else {
				path.clone()
			}
		});
		let error = disassembly.as_ref().and_then(|disassembly| disassembly.error.clone());

		let instructions: Vec<DisassembledInstruction> =
			instruction_window(reference, code, offset, listed, args.instructionOffset.unwrap_or(0), count)
				.into_iter()
				.map(|place| match place {
					Ok(index) => {
						let instruction = &listed[index];
						let text = match (&instruction.text, &error) {
							(text, _) if !text.is_empty() => text.clone(),
							(_, Some(error)) => format!("(not read: {})", error),
							(_, None) => "(not read)".to_owned()
						};
						DisassembledInstruction {
							address: address_text(aux::address(code, instruction.offset)),
							instructionBytes: Some(instruction.words.iter().map(|word| format!("{:08X}", word)).collect::<Vec<_>>().join(" ")),
							instruction: text,
							symbol: symbol.clone(),
							location: file.as_deref().filter(|_| instruction.line.is_some()).map(|file| self.file_source(file)),
							line: instruction.line.map(i64::from),
							..Default::default()
						}
					}
					Err(address) => DisassembledInstruction {
						address: address_text(address),
						instruction: String::new(),
						..Default::default()
					}
				})
				.collect();

		// Marked for clients that grey out what isn't code
		let mut instructions = json(instructions)?;
		for instruction in instructions.as_array_mut().into_iter().flatten() {
			if instruction["instructionBytes"].is_null() {
				instruction["presentationHint"] = "invalid".into();
			}
		}
		Ok(serde_json::json!({ "instructions": instructions }))
	}

	/// Build the client's source descriptor from a debug source name.
	fn file_source(&self, file: &str) -> Source {
		source_at(&self.files.lock().unwrap().path(file))
	}

	fn source(&mut self, args: SourceArguments) -> Handled {
		let reference = args.source.and_then(|source| source.sourceReference).unwrap_or(args.sourceReference);
		match (reference, &self.stddef) {
			(STDDEF_REFERENCE, Some(stddef)) => json(SourceResponse {
				content: stddef.text.clone(),
				mimeType: None
			}),
			_ => Err("source not found".to_owned())
		}
	}

	/// Return executable lines in the requested range.
	/// Unavailable without an active debugging connection or for stddef.dm.
	fn breakpoint_locations(&mut self, args: BreakpointLocationsArguments) -> Handled {
		let path = match args.source.path {
			Some(path) if self.debugging && self.debuggee.is_some() => path,
			_ => return Ok(serde_json::json!({ "breakpoints": [] }))
		};
		let start = args.line.clamp(1, u32::MAX.into()) as u32;
		let end = args.end_line.unwrap_or(args.line).clamp(start.into(), u32::MAX.into()) as u32;
		match self.request(&AuxRequest::LinesWithCode {
			file: self.world_file_name(&path),
			start,
			end
		})? {
			AuxResponse::LinesWithCode { lines } => {
				let breakpoints: Vec<Value> = lines.into_iter().map(|line| serde_json::json!({ "line": line })).collect();
				Ok(serde_json::json!({ "breakpoints": breakpoints }))
			}
			other => Err(unexpected(other))
		}
	}

	/// Whether the world has source files in its debug info, beyond the
	/// launcher helpers.
	fn world_has_lines(&mut self) -> bool {
		match self.request(&AuxRequest::Files) {
			Ok(AuxResponse::Files { files }) => files.iter().any(|file| file != HELPER_FILE),
			// Keep the usual diagnostic if the request fails; missing debug
			// info is unconfirmed.
			_ => true
		}
	}

	/// List sorted source paths from the world's debug info, appending
	/// stddef.dm if available. Files without procs are absent. Unavailable
	/// without an active debugging connection.
	fn loaded_sources(&mut self) -> Handled {
		if !self.debugging || self.debuggee.is_none() {
			return Ok(serde_json::json!({ "sources": [] }));
		}
		let files = match self.request(&AuxRequest::Files)? {
			AuxResponse::Files { files } => files,
			other => return Err(unexpected(other))
		};
		let mut paths: Vec<PathBuf> = files
			.into_iter()
			// The debug library's own procs aren't in a file of the world's
			.filter(|file| file != HELPER_FILE)
			.map(|file| self.files.lock().unwrap().path(&file))
			.collect();
		paths.sort();
		paths.dedup();
		let mut sources: Vec<Source> = paths.iter().map(|path| source_at(path)).collect();
		if self.stddef.is_some() {
			sources.push(Source {
				name: Some("stddef.dm".to_owned()),
				sourceReference: Some(STDDEF_REFERENCE),
				..Default::default()
			});
		}
		json(serde_json::json!({ "sources": sources }))
	}
}

/// The file and line in BYOND's report of a runtime error:
/// `  source file: code/game/mob.dm,42`
fn runtime_place(line: &str) -> Option<(&str, i64)> {
	let (file, number) = line.trim().strip_prefix("source file: ")?.rsplit_once(',')?;
	Some((file, number.trim().parse().ok()?))
}

/// Completion prefix at the end of the input.
#[derive(Debug, PartialEq, Eq)]
struct CompletionPlace<'a> {
	/// Where the name starts
	start: usize,
	/// Receiver expression, e.g. `a.b[2]` in `a.b[2].na`.
	base: Option<&'a str>,
	/// It's one of the debug server's commands, after a `#`
	command: bool
}

/// Find the completion prefix and receiver at the end of `text`.
/// Returns `None` inside strings or when resolving the receiver requires a
/// call.
fn completion_place(text: &str) -> Option<CompletionPlace<'_>> {
	let name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
	if text.chars().filter(|&c| c == '"').count() % 2 == 1 {
		return None;
	}
	let start = text.trim_end_matches(name_char).len();
	// A number isn't a name
	if text[start..].starts_with(|c: char| c.is_ascii_digit()) {
		return None;
	}
	if text[..start].trim_start() == "#" {
		return Some(CompletionPlace {
			start,
			base: None,
			command: true
		});
	}

	// What's before it, back through `.`, `?.`, `:` and `?:`, names and
	// indexes
	let mut at = start;
	let mut base_end = None;
	loop {
		let before = &text[..at];
		let Some(joined) = ["?.", "?:", ".", ":"].into_iter().find(|by| before.ends_with(by)) else {
			break;
		};
		let member_end = at - joined.len();
		base_end.get_or_insert(member_end);
		let mut member = member_end;
		// Indexes: `[...]`, with nothing called inside
		while text[..member].ends_with(']') {
			let mut depth = 0;
			let mut open = None;
			for (i, c) in text[..member].char_indices().rev() {
				match c {
					']' => depth += 1,
					'[' => {
						depth -= 1;
						if depth == 0 {
							open = Some(i);
							break;
						}
					}
					'(' | ')' => return None,
					_ => {}
				}
			}
			member = open?;
		}
		let name = text[..member].trim_end_matches(name_char).len();
		// Something else: a call's result, a parenthesis
		if name == member {
			return None;
		}
		at = name;
	}
	Some(CompletionPlace {
		start,
		base: base_end.map(|end| &text[at..end]),
		command: false
	})
}

/// Explain why hover evaluation is unsafe. Reject proc bodies, which can
/// loop indefinitely even without calls or assignments, and expressions
/// that may have side effects.
fn hover_refuses(code: &str) -> Option<&'static str> {
	if dmasm::compiler::is_body(code) {
		return Some("hover requires a single expression");
	}
	changes_things(code).then_some("hover expression may have side effects")
}

/// Whether the expression may call, assign, allocate, delete or perform I/O.
/// Ignore string contents except embedded expressions.
fn changes_things(expression: &str) -> bool {
	let mut quoted = false;
	let mut chars = expression.chars().peekable();
	let mut last = ' ';
	// The name being read, outside texts
	let mut word = String::new();
	while let Some(c) = chars.next() {
		if !quoted && (c.is_ascii_alphanumeric() || c == '_') {
			word.push(c);
		} else {
			// `new /datum/thing` with no parentheses, and `del x`
			if matches!(word.as_str(), "new" | "del") {
				return true;
			}
			word.clear();
		}
		if quoted {
			match c {
				'\\' => {
					chars.next();
				}
				'"' => quoted = false,
				// An embedded expression may have side effects.
				'[' => return true,
				_ => {}
			}
			continue;
		}
		let next = chars.peek().copied().unwrap_or(' ');
		match c {
			'"' => quoted = true,
			'(' => return true,
			'+' | '-' if next == c => return true,
			'<' | '>' if next == c => return true,
			// `=` on its own, or after an operator that makes an assignment
			// of it (`+=`, `||=`), but not `==`, `!=`, `<=` or `>=`
			'=' if next != '=' && !matches!(last, '=' | '!' | '<' | '>') => return true,
			'=' if next != '=' && matches!(last, '<' | '>') => {
				// `<<=` and `>>=` assign, `<=` and `>=` compare
				let shifts = expression.contains("<<=") || expression.contains(">>=");
				if shifts {
					return true;
				}
			}
			_ => {}
		}
		last = c;
	}
	matches!(word.as_str(), "new" | "del")
}

/// Format a value for the client, using hex for integers if requested.
fn shown(value: String, hex: bool) -> String {
	if !hex {
		return value;
	}
	match value.parse::<i64>() {
		Ok(n) if n < 0 => format!("-0x{:X}", -n),
		Ok(n) => format!("0x{:X}", n),
		Err(_) => value
	}
}

/// Parse the client's optional breakpoint hit condition.
fn hit_condition(text: Option<&str>) -> Result<Option<aux::HitCondition>, String> {
	match text.map(str::trim).filter(|text| !text.is_empty()) {
		Some(text) => aux::HitCondition::parse(text).map(Some),
		None => Ok(None)
	}
}

/// Format an instruction reference as hexadecimal.
fn address_text(address: u64) -> String {
	format!("0x{:016X}", address)
}

/// Parse an instruction reference as hexadecimal (`0x`) or decimal.
fn parse_address(text: &str) -> Option<u64> {
	let text = text.trim();
	match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
		Some(hex) => u64::from_str_radix(hex, 16).ok(),
		None => text.parse().ok()
	}
}

/// Map a disassembly window to instruction indexes or placeholder addresses.
/// The window begins `shift` instructions from `offset` within `reference`.
/// Outside the code, assign consecutive word addresses to preserve ordering.
fn instruction_window(
	reference: u64,
	code: aux::CodeRef,
	offset: u16,
	instructions: &[aux::BytecodeInstruction],
	shift: i64,
	count: usize
) -> Vec<Result<usize, u64>> {
	let Some(last) = instructions.last() else {
		// No code here: each place is a word on from the last
		return (0..count as i64).map(|k| Err(reference.wrapping_add_signed(shift + k))).collect();
	};
	let start = aux::address(code, 0);
	let length = i64::from(last.offset) + last.words.len() as i64;
	let listed = instructions.len() as i64;
	// The instruction the offset is in, or how far past the end it is
	let at = if i64::from(offset) >= length {
		listed + i64::from(offset) - length
	} else {
		instructions.partition_point(|instruction| instruction.offset <= offset) as i64 - 1
	};
	(0..count as i64)
		.map(|k| {
			let index = at + shift + k;
			if index < 0 {
				Err(start.wrapping_add_signed(index))
			} else if index >= listed {
				Err(start.wrapping_add_signed(length + index - listed))
			} else {
				Ok(index as usize)
			}
		})
		.collect()
}

/// Parse a function breakpoint path and optional zero-based `#N` override.
fn named_proc(name: &str) -> ProcRef {
	let (path, override_id) = match name.split_once('#') {
		Some((path, id)) => (path.trim().to_owned(), id.trim().parse().unwrap_or(0)),
		None => (name.trim().to_owned(), 0)
	};
	ProcRef { path, override_id }
}

/// A file as the client is shown it
fn source_at(path: &Path) -> Source {
	Source {
		name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
		path: Some(path.to_string_lossy().into_owned()),
		..Default::default()
	}
}

/// Format a thread label with its proc, location and sleeping status.
fn thread_name(stack: &aux::Stack) -> String {
	let mut name = stack.name.clone();
	if let (Some(file), Some(line)) = (&stack.file, stack.line) {
		let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
		name += &format!("  {}:{}", file, line);
	}
	if stack.sleeping {
		name += ", sleeping";
	}
	name
}

fn unexpected(response: AuxResponse) -> String {
	format!("unexpected debug server response: {:?}", response)
}

/// Default arguments passed after the `.dmb` unless overridden.
pub fn default_args(server: Server) -> Vec<String> {
	match server {
		Server::DreamDaemon if cfg!(windows) => vec!["-trusted".to_owned(), "-close".to_owned()],
		_ => vec!["-trusted".to_owned()]
	}
}

/// The debug library, next to the launcher
fn default_library() -> Result<PathBuf, String> {
	let name = if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" };
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let library = exe.parent().map(|dir| dir.join(name)).unwrap_or_else(|| Path::new(name).to_owned());
	if !library.is_file() {
		return Err(format!("debug library not found at {} (debugLibrary)", library.display()));
	}
	Ok(library)
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use auxtools_cli::server_types::{HitCondition, LogPart, RuntimeFilter};

	use super::{changes_things, completion_place, hover_refuses, instruction_window, named_proc, parse_address, runtime_place, shown};

	#[test]
	fn byond_says_where_a_runtime_error_was() {
		assert_eq!(runtime_place("  source file: code/game/mob.dm,42"), Some(("code/game/mob.dm", 42)));
		assert_eq!(runtime_place("  source file: a, b.dm,7\r\n"), Some(("a, b.dm", 7)));
		for other in [
			"runtime error: bad",
			"proc name: fail (/proc/fail)",
			"  source file: mob.dm",
			"source file: mob.dm,x",
			"  usr: null"
		] {
			assert_eq!(runtime_place(other), None, "{}", other);
		}
	}

	#[test]
	fn a_name_is_completed_where_it_starts() {
		fn place(text: &str) -> Option<(&str, Option<&str>, bool)> {
			completion_place(text).map(|place| (&text[place.start..], place.base, place.command))
		}
		assert_eq!(place("hea"), Some(("hea", None, false)));
		assert_eq!(place(""), Some(("", None, false)));
		assert_eq!(place("x + hea"), Some(("hea", None, false)));
		assert_eq!(place("leaf(by, n"), Some(("n", None, false)));
		assert_eq!(place("src."), Some(("", Some("src"), false)));
		assert_eq!(place("src.na"), Some(("na", Some("src"), false)));
		assert_eq!(place("x + src.loc.na"), Some(("na", Some("src.loc"), false)));
		assert_eq!(place("a?.b:c?:d"), Some(("d", Some("a?.b:c"), false)));
		assert_eq!(place("items[2].na"), Some(("na", Some("items[2]"), false)));
		assert_eq!(place("f(x) + items[\"k\"][i + 1].na"), Some(("na", Some("items[\"k\"][i + 1]"), false)));
		assert_eq!(place("#"), Some(("", None, true)));
		assert_eq!(place("#di"), Some(("di", None, true)));
		assert_eq!(place(" #mem"), Some(("mem", None, true)));
	}

	#[test]
	fn nothing_is_completed_where_it_cant_be_known() {
		for text in [
			"get_mob().na",
			"(a).b",
			"items[f()].na",
			"\"text",
			"\"a\" + \"b.",
			"12",
			"x + 3",
			".na",
			"a..b",
			"]."
		] {
			assert_eq!(completion_place(text), None, "{}", text);
		}
	}

	#[test]
	fn a_hover_reads_and_does_nothing_else() {
		for reads in [
			"x",
			"src.health",
			"a?.b:c",
			"x == 1",
			"x != y && x <= 2 || y >= 3",
			"L[2]",
			"L[\"a = b\"]",
			"\"text with (parens) and = signs++\"",
			"x - -y",
			"x + +y",
			"a < b",
			"/datum/thing",
			"newest",
			"src.renew",
			"x.del_count",
			"model",
			"\"new del << >>\"",
			"x < -y",
			"a > b"
		] {
			assert!(!changes_things(reads), "{}", reads);
		}
		for changes in [
			"f()",
			"src.bump(1)",
			"x = 1",
			"x += 1",
			"x ||= y",
			"x <<= 1",
			"x++",
			"--x",
			"L[f()]",
			"\"[f()]\"",
			"new /datum/thing()",
			"new /datum/thing",
			"new/datum/thing",
			"x = new",
			"del x",
			"del(x)",
			"world << \"x\"",
			"src << x",
			"x >> y",
			"L[new /datum/thing]"
		] {
			assert!(changes_things(changes), "{}", changes);
		}
	}

	#[test]
	fn a_hover_runs_no_body() {
		// Bodies without calls or assignments must still be rejected.
		for body in ["again:\ngoto again", "var/x\nx", "return n", "n; n", "x\ny"] {
			assert_eq!(hover_refuses(body), Some("hover requires a single expression"), "{:?}", body);
		}
		for (code, why) in [("x", None), ("src.n;", None), ("x = 1", Some("hover expression may have side effects"))] {
			assert_eq!(hover_refuses(code), why, "{:?}", code);
		}
	}

	#[test]
	fn whole_numbers_are_shown_in_hex_when_asked() {
		assert_eq!(shown("42".to_owned(), true), "0x2A");
		assert_eq!(shown("0".to_owned(), true), "0x0");
		assert_eq!(shown("-255".to_owned(), true), "-0xFF");
		assert_eq!(shown("16777216".to_owned(), true), "0x1000000");
		assert_eq!(shown("42".to_owned(), false), "42");
		for other in ["1.5", "\"42\"", "null", "/list {len = 3}", "inf"] {
			assert_eq!(shown(other.to_owned(), true), other);
		}
	}

	#[test]
	fn a_log_message_is_text_with_expressions_in_braces() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("plain"), [text("plain")]);
		assert_eq!(LogPart::parse(""), []);
		assert_eq!(LogPart::parse("{x}"), [expr("x")]);
		assert_eq!(LogPart::parse("x is {x}, y is { y + 1 }!"), [
			text("x is "),
			expr("x"),
			text(", y is "),
			expr("y + 1"),
			text("!")
		]);
		assert_eq!(LogPart::parse("{a}{b}"), [expr("a"), expr("b")]);
	}

	#[test]
	fn braces_in_an_expression_are_its_own() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{alist(\"a\" = 1)}"), [expr("alist(\"a\" = 1)")]);
		assert_eq!(LogPart::parse("{\"}\"} after"), [expr("\"}\""), text(" after")]);
		assert_eq!(LogPart::parse("{\"a \\\" } b\"}"), [expr("\"a \\\" } b\"")]);
		assert_eq!(LogPart::parse("{a {b} c}"), [expr("a {b} c")]);
	}

	#[test]
	fn doubled_braces_are_literal() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{{x}}"), [text("{x}")]);
		assert_eq!(LogPart::parse("{{\"a\": {a}}}"), [text("{\"a\": "), expr("a"), text("}")]);
		assert_eq!(LogPart::parse("{list(1) ~= list(1)}}}"), [expr("list(1) ~= list(1)"), text("}")]);
		// Braces inside expressions are not interpolation escapes.
		assert_eq!(LogPart::parse("{a {{b}} c}"), [expr("a {{b}} c")]);
	}

	#[test]
	fn a_brace_that_is_never_closed_is_text() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{x"), [text("{x")]);
		assert_eq!(LogPart::parse("{x} and {y"), [expr("x"), text(" and {y")]);
		assert_eq!(LogPart::parse("} {"), [text("} {")]);
	}

	#[test]
	fn a_hit_count_is_a_number_or_one_after_a_sign() {
		for (text, expected) in [
			("5", HitCondition::AtLeast(5)),
			(" 5 ", HitCondition::AtLeast(5)),
			(">= 5", HitCondition::AtLeast(5)),
			(">=5", HitCondition::AtLeast(5)),
			("> 5", HitCondition::More(5)),
			("== 5", HitCondition::Equal(5)),
			("=5", HitCondition::Equal(5)),
			("< 5", HitCondition::Less(5)),
			("<= 5", HitCondition::AtMost(5)),
			("% 5", HitCondition::Every(5)),
			("%5", HitCondition::Every(5))
		] {
			assert_eq!(HitCondition::parse(text), Ok(expected), "{:?}", text);
		}
		for text in ["", "five", "5 times", "-1", "% 0", "=> 5", "5 5", "> ", "1.5"] {
			assert!(HitCondition::parse(text).is_err(), "{:?}", text);
		}
	}

	#[test]
	fn a_hit_count_takes_the_hits_it_says() {
		let taken = |hit: HitCondition| -> Vec<u32> { (1..=8).filter(|&n| hit.takes(n)).collect() };
		assert_eq!(taken(HitCondition::AtLeast(6)), [6, 7, 8]);
		assert_eq!(taken(HitCondition::More(6)), [7, 8]);
		assert_eq!(taken(HitCondition::Equal(3)), [3]);
		assert_eq!(taken(HitCondition::Less(3)), [1, 2]);
		assert_eq!(taken(HitCondition::AtMost(3)), [1, 2, 3]);
		assert_eq!(taken(HitCondition::Every(3)), [3, 6]);
		assert_eq!(taken(HitCondition::Every(1)), [1, 2, 3, 4, 5, 6, 7, 8]);
	}

	#[test]
	fn a_runtime_filter_with_nothing_in_it_takes_every_error() {
		for condition in ["", "  ", ",", "!", " , ! , "] {
			let filter = RuntimeFilter::parse(condition);
			assert_eq!(filter, RuntimeFilter::default(), "{:?}", condition);
			assert!(filter.takes("Division by zero"));
		}
	}

	#[test]
	fn a_runtime_filter_takes_errors_with_one_of_its_texts() {
		let filter = RuntimeFilter::parse("bounds, Division");
		assert!(filter.takes("list index out of bounds"));
		assert!(filter.takes("division by zero"));
		assert!(!filter.takes("cannot read from list"));
	}

	#[test]
	fn a_runtime_filter_leaves_out_errors_with_text_after_a_bang() {
		let filter = RuntimeFilter::parse("!libdreamluau, ! rust_g ");
		assert!(filter.takes("list index out of bounds"));
		assert!(!filter.takes("Could not find LibDreamLuau.so"));
		assert!(!filter.takes("rust_g: bad call"));
		// Require a match in one set and no matches in the other.
		let filter = RuntimeFilter::parse("list, !bounds");
		assert!(filter.takes("cannot read from list"));
		assert!(!filter.takes("list index out of bounds"));
		assert!(!filter.takes("Division by zero"));
	}

	#[test]
	fn a_function_breakpoint_names_a_proc() {
		let proc = named_proc("/mob/proc/Login");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/proc/Login", 0));
		let proc = named_proc("  /proc/leaf ");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/proc/leaf", 0));
	}

	#[test]
	fn a_function_breakpoint_names_a_definition() {
		let proc = named_proc("/mob/Login#2");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 2));
		let proc = named_proc("/mob/Login # 1 ");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 1));
	}

	#[test]
	fn a_definition_that_isnt_a_number_is_the_first() {
		let proc = named_proc("/mob/Login#last");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 0));
	}

	/// Code of three instructions: two words, one, three
	fn three() -> Vec<auxtools_cli::server_types::BytecodeInstruction> {
		[(0, 2), (2, 1), (3, 3)]
			.into_iter()
			.map(|(offset, length)| auxtools_cli::server_types::BytecodeInstruction {
				offset,
				words: vec![0; length],
				text: "x".to_owned(),
				line: None
			})
			.collect()
	}

	#[test]
	fn an_address_is_the_proc_its_code_and_the_offset() {
		use auxtools_cli::server_types::{address, from_address, CodeRef};
		let code = CodeRef { proc: 0x2a, old: 3 };
		assert_eq!(address(code, 0x10), 0x0000_002a_0003_0010);
		assert_eq!(from_address(0x0000_002a_0003_0010), (code, 0x10));
		assert_eq!(parse_address("0x0000002A00030010"), Some(0x0000_002a_0003_0010));
		assert_eq!(parse_address("16"), Some(16));
		assert_eq!(parse_address("nowhere"), None);
	}

	#[test]
	fn a_window_has_as_many_places_as_asked_for_in_order() {
		use auxtools_cli::server_types::{address, CodeRef};
		let code = CodeRef { proc: 7, old: 0 };
		let start = address(code, 0);
		let listed = three();
		// From the second instruction, two back and five on
		let window = instruction_window(start + 2, code, 2, &listed, -3, 6);
		assert_eq!(window, [Err(start - 2), Err(start - 1), Ok(0), Ok(1), Ok(2), Err(start + 6)]);
		// From the middle of the first, which is the first
		assert_eq!(instruction_window(start + 1, code, 1, &listed, 0, 2), [Ok(0), Ok(1)]);
		// From past the end
		assert_eq!(instruction_window(start + 7, code, 7, &listed, -2, 3), [
			Ok(2),
			Err(start + 6),
			Err(start + 7)
		]);
		// Where there's no code
		assert_eq!(instruction_window(100, code, 100, &[], -1, 3), [Err(99), Err(100), Err(101)]);
	}
}
