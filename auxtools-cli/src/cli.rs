//! Run a command against a world and print the result.
//!
//! Commands that leave the world paused hold the pause after disconnecting.
//! Results go to stdout as text or JSON; diagnostics go to stderr.
//! Exit codes: 0 success, 1 failure, 2 timeout, 3 world exited or rebooted.

/// Print to stdout, exiting quietly on write failure (e.g. `| head`).
/// This also handles Windows, where `print!` would panic on a closed pipe.
macro_rules! say_part {
	($($arg:tt)*) => {
		crate::cli::say(format_args!($($arg)*))
	};
}

/// `say_part!` with a trailing newline.
macro_rules! say {
	($($arg:tt)*) => {
		crate::cli::say(format_args!("{}\n", format_args!($($arg)*)))
	};
}

mod args;
mod help;
mod hold;
mod json;
mod launch;
mod log;
mod stop;
mod trace;

use std::{
	io,
	sync::mpsc::{self, Receiver, RecvTimeoutError},
	time::{Duration, Instant}
};

use auxtools_cli::{
	local::{self, Shared},
	server_types::*
};
use serde_json::{json, Map, Value};

use self::args::{Command, WaitFor};

/// Command error and exit code.
pub struct Failure(pub i32, pub String);

impl From<io::Error> for Failure {
	fn from(e: io::Error) -> Self {
		Self(1, e.to_string())
	}
}

impl From<String> for Failure {
	fn from(message: String) -> Self {
		Self(1, message)
	}
}

impl From<&str> for Failure {
	fn from(message: &str) -> Self {
		Self(1, message.to_owned())
	}
}

pub type Result<T> = std::result::Result<T, Failure>;

/// Command timed out.
const TIMED_OUT: i32 = 2;
/// World exited or rebooted before the command completed.
const HUNG_UP: i32 = 3;

/// Runs `args` (everything after the program's name), and exits with its code
pub fn run(args: &[String]) -> ! {
	// Exit quietly when a downstream reader closes the pipe (`| head`).
	#[cfg(unix)]
	unsafe {
		libc::signal(libc::SIGPIPE, libc::SIG_DFL);
	}
	let code = match execute(args) {
		Ok(()) => 0,
		Err(Failure(code, message)) => {
			eprintln!("error: {}", message);
			code
		}
	};
	std::process::exit(code)
}

/// Write to stdout and exit with code 0 on write or flush failure.
pub fn say(text: std::fmt::Arguments) {
	use std::io::Write;
	let mut stdout = io::stdout().lock();
	if stdout.write_fmt(text).and_then(|()| stdout.flush()).is_err() {
		std::process::exit(0);
	}
}

fn execute(raw: &[String]) -> Result<()> {
	let cli = match args::parse(raw) {
		Ok(cli) => cli,
		Err(args::Unparsed::Help(page)) => {
			say_part!("{}", page);
			return Ok(());
		}
		Err(args::Unparsed::Usage(why)) => return Err(why.into())
	};
	let Some(command) = cli.command else {
		// Asked which it is, or nothing at all
		if cli.version {
			return version(false);
		}
		return help::show(&[]);
	};
	if cli.pid.is_some() && !command.selects_world() {
		return Err(format!("--pid cannot be used with '{}': it does not select an existing world.", command.name()).into());
	}
	if cli.response_timeout.is_some() && !command.awaits_responses() {
		return Err(match command {
			Command::Stop(_) => "--response-timeout cannot be used with 'stop': it waits for the process to exit; use --timeout.".to_owned(),
			_ => format!(
				"--response-timeout cannot be used with '{}': it does not select an existing world.",
				command.name()
			)
		}
		.into());
	}
	// Validate arguments before connecting to a world.
	args::rules(&command)?;
	match command {
		Command::Dap => {
			crate::adapter::run();
			Ok(())
		}
		Command::Help(help) => help::show(&help.path),
		Command::Version(version) => self::version(version.json),
		Command::List(list) => self::list(list.json),
		Command::Launch(launch) => launch::launch(&launch, raw.iter().any(|arg| arg == "--")),
		Command::Wait(args::Wait { event: None }) => {
			help::show(&["wait".to_owned()])?;
			Err("wait requires a command: pause, reload, or reboot.".into())
		}
		command => {
			let hold_max = hold::max(&command)?;
			let pid = match cli.pid {
				Some(pid) => pid,
				None => only_world()?
			};
			let mut world = World::connect(pid)?;
			world.patience = cli.response_timeout.unwrap_or(args::DEFAULT_RESPONSE_TIMEOUT);
			if let Some(limit) = hold_max {
				world.hold_max(limit)?;
			}
			let result = world.command(command);
			world.leave();
			result
		}
	}
}

fn version(as_json: bool) -> Result<()> {
	if as_json {
		say!("{}", json!({ "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL_VERSION }));
	} else {
		say!("auxtools {} (protocol {})", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION);
	}
	Ok(())
}

/// `list`: the worlds running the debug library, as a table or as JSON
fn list(as_json: bool) -> Result<()> {
	let worlds = local::worlds();
	if as_json {
		say!("{}", serde_json::to_string(&worlds).map_err(|e| e.to_string())?);
		return Ok(());
	}
	if worlds.is_empty() {
		say!("No worlds found with the debug library loaded.");
		return Ok(());
	}
	let mut rows = vec![["PID", "PROGRAM", "PROTOCOL", "COMPATIBLE", "RESPONSIVE", "DMB", "CWD", "ARGS"].map(str::to_owned)];
	for world in &worlds {
		// World details are unavailable for incompatible or unresponsive
		// worlds.
		let unknown = |text: String| if text.is_empty() { "-".to_owned() } else { text };
		rows.push([
			world.pid.to_string(),
			world.program.clone(),
			world.protocol.to_string(),
			if world.compatible { "yes" } else { "no" }.to_owned(),
			match world.responsive {
				Some(true) => "yes",
				Some(false) => "no",
				None => "-"
			}
			.to_owned(),
			unknown(world.dmb.as_ref().map(|dmb| dmb.display().to_string()).unwrap_or_default()),
			unknown(world.cwd.clone()),
			unknown(world.args.join(" "))
		]);
	}
	for row in &rows {
		let mut line = String::new();
		for (column, cell) in row.iter().enumerate() {
			let width = rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0);
			line += &format!("{:width$}  ", cell);
		}
		say!("{}", line.trim_end());
	}
	Ok(())
}

/// A time as an error says it: `30 seconds`, `1 second`, `0.5 seconds`
fn seconds_text(time: Duration) -> String {
	let seconds = time.as_secs_f64();
	let number = if seconds.fract() == 0.0 {
		format!("{}", seconds as u64)
	} else {
		format!("{}", seconds)
	};
	format!("{} {}", number, if seconds == 1.0 { "second" } else { "seconds" })
}

/// Read an expression or message argument: `@FILE` reads a file and
/// `@@...` escapes a literal `@`. File input avoids shell quoting.
fn given_text(value: &str) -> Result<String> {
	match value.strip_prefix('@') {
		Some(rest) if rest.starts_with('@') => Ok(rest.to_owned()),
		Some(file) => read_text(file),
		None => Ok(value.to_owned())
	}
}

/// Read a file and remove its final line ending.
fn read_text(file: &str) -> Result<String> {
	let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {}.", file, e))?;
	Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn read_stdin() -> Result<String> {
	use std::io::{IsTerminal, Read};
	// Require piped input so an unattended caller cannot block on a terminal.
	if io::stdin().is_terminal() {
		return Err("'-' reads the expression from stdin, but stdin is a terminal.".into());
	}
	let mut text = String::new();
	io::stdin().read_to_string(&mut text).map_err(|e| format!("cannot read stdin: {}.", e))?;
	Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

/// `eval`'s expression: its words, what's in `--file FILE`, or stdin for `-`
/// (`args::rules` has seen that it's one of them)
fn expression(words: &[String], file: Option<&std::path::Path>, stdin: impl FnOnce() -> Result<String>) -> Result<String> {
	let (expression, from) = match (words, file) {
		(_, Some(file)) => (read_text(&file.to_string_lossy())?, file.display().to_string()),
		([dash], None) if dash == "-" => (stdin()?, "stdin".to_owned()),
		(_, None) => return Ok(words.join(" "))
	};
	if expression.trim().is_empty() {
		return Err(format!("{} contains no expression.", from).into());
	}
	Ok(expression)
}

/// Parse `/path` or `/path#N`, where N is the zero-based override index.
fn proc_ref(text: &str) -> Result<ProcRef> {
	if !text.starts_with('/') {
		return Err(format!("invalid proc path '{}': expected /PATH[#N].", text).into());
	}
	let (path, override_id) = match text.split_once('#') {
		Some((path, id)) => (
			path,
			id.parse()
				.map_err(|_| format!("invalid definition index '{}' in '{}': expected a non-negative integer.", id, text))?
		),
		None => (text, 0)
	};
	Ok(ProcRef {
		path: path.to_owned(),
		override_id
	})
}

/// A proc's name as the stack shows it: its path, and `#N` for a definition
/// after the first
fn proc_name(proc: &ProcRef) -> String {
	match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	}
}

/// Choose the only running world when no process ID was supplied.
fn only_world() -> Result<u32> {
	let worlds = local::worlds();
	let compatible: Vec<&local::World> = worlds.iter().filter(|world| world.compatible).collect();
	match compatible.as_slice() {
		[world] => Ok(world.pid),
		[] if worlds.is_empty() => Err("no worlds found with the debug library loaded. Start one with 'auxtools launch'.".into()),
		[] => Err(format!(
			"no compatible worlds found: {} use another debug protocol version than this CLI ({}). Use 'auxtools list'.",
			worlds.len(),
			PROTOCOL_VERSION
		)
		.into()),
		_ => Err("multiple compatible worlds found. Specify --pid; use 'auxtools list'.".into())
	}
}

/// What `wait pause` waits for
struct Wait {
	breaks: Vec<String>,
	/// Expressions for places to watch for a change
	changes: Vec<String>,
	/// The frame they're of, when the world is paused
	frame: u32,
	condition: Option<String>,
	runtimes: bool,
	/// Those inside a `try` too
	caught: bool,
	/// Runtime error message filter.
	runtime_filter: RuntimeFilter,
	timeout: Option<Duration>,
	hold: bool
}

/// How a step ended
enum Stepped {
	Paused(BreakpointReason),
	/// With the world running: nothing was left to pause in
	Ended,
	TimedOut
}

pub struct World {
	/// Its process
	pid: u32,
	stream: Shared,
	received: Receiver<io::Result<Response>>,
	/// Pause and resume events received while awaiting a response.
	events: Vec<Response>,
	/// A reload may have moved the breakpoints installed by `wait`.
	reloaded: bool,
	/// Server notifications, including evaluation errors.
	notes: Vec<String>,
	/// Condition errors already reported, to avoid duplicate diagnostics.
	failed_conditions: Vec<String>,
	/// Include the current instruction after instruction stepping.
	show_instruction: bool,
	/// What it prints is one JSON document (`--json`), with lists and objects
	/// to this depth
	json: Option<u32>,
	/// What `wait pause` adds to the JSON of a pause
	waited: Map<String, Value>,
	/// How long to wait for each response (`--response-timeout`).
	patience: Duration
}

impl World {
	fn connect(pid: u32) -> Result<Self> {
		let stream = Shared::new(local::connect(pid).map_err(|e| format!("cannot connect to the world with pid {}: {}.", pid, e))?);
		let (sender, received) = mpsc::channel();
		let mut reader = stream.clone();
		std::thread::spawn(move || loop {
			let response = local::receive(&mut reader);
			let failed = response.is_err();
			if sender.send(response).is_err() || failed {
				break;
			}
		});
		Ok(Self {
			pid,
			stream,
			received,
			events: vec![],
			reloaded: false,
			notes: vec![],
			failed_conditions: vec![],
			show_instruction: false,
			json: None,
			waited: Map::new(),
			patience: args::DEFAULT_RESPONSE_TIMEOUT
		})
	}

	/// Execute the command and print its result.
	fn command(&mut self, command: Command) -> Result<()> {
		// How it prints: as text, or as JSON, with lists and objects in it
		// written out to a depth
		let output = |json: bool, depth: Option<u32>| json.then(|| depth.unwrap_or(json::DEFAULT_DEPTH));
		match command {
			Command::Status(args) => {
				self.json = output(args.json, None);
				self.status()
			}
			Command::Stop(args) => stop::stop(self, &args),
			Command::Reboot(args) => stop::reboot(self, &args),
			Command::Log(args) => log::log(self, &args),
			Command::Stack(args) => {
				self.json = output(args.json, None);
				self.stack(args.all)
			}
			Command::Vars(args) => {
				self.json = output(args.json, args.depth);
				self.vars(args.frame.unwrap_or(0), args.globals)
			}
			Command::Eval(args) => {
				self.json = output(args.json, args.depth);
				let expression = expression(&args.expression, args.file.as_deref(), read_stdin)?;
				match &args.src {
					object if args.spawn => self.spawn(&expression, object.as_deref(), args.wait.then_some(args.timeout)),
					Some(object) => self.eval_on(object, &expression),
					None => self.eval(&expression, args.frame)
				}
			}
			Command::Proc(args) => {
				self.json = output(args.json, None);
				self.proc_info(&proc_ref(&args.proc)?)
			}
			Command::Disassemble(args) => {
				self.json = output(args.json, None);
				self.disassemble(&proc_ref(&args.proc)?)
			}
			Command::Pause(args) => {
				self.json = output(args.json, None);
				self.pause()
			}
			Command::Continue(args) => {
				self.json = output(args.json, None);
				self.resume()
			}
			Command::StepOver(args) => {
				self.json = output(args.json, None);
				self.show_instruction = args.instruction;
				let kind = match args.instruction {
					true => ContinueKind::StepOverInstruction { stack_id: 0 },
					false => ContinueKind::StepOver { stack_id: 0 }
				};
				self.step(kind, args.timeout)
			}
			Command::StepIn(args) => {
				self.json = output(args.json, None);
				self.show_instruction = args.instruction;
				let kind = match args.instruction {
					true => ContinueKind::StepIntoInstruction { stack_id: 0 },
					false => ContinueKind::StepInto { stack_id: 0 }
				};
				self.step(kind, args.timeout)
			}
			Command::StepOut(args) => {
				self.json = output(args.json, None);
				self.step(ContinueKind::StepOut { stack_id: 0 }, args.timeout)
			}
			Command::Wait(args::Wait {
				event: Some(WaitFor::Pause(args))
			}) => {
				self.json = output(args.json, None);
				self.wait(Wait {
					breaks: args.breaks,
					changes: args.changes.iter().map(|text| given_text(text)).collect::<Result<_>>()?,
					frame: args.frame.unwrap_or(0),
					condition: args.condition.as_deref().map(given_text).transpose()?,
					runtimes: args.runtimes || args.caught || args.runtime_filter.is_some(),
					caught: args.caught,
					runtime_filter: args.runtime_filter.as_deref().map(RuntimeFilter::parse).unwrap_or_default(),
					timeout: args.timeout,
					hold: !args.no_hold
				})
			}
			Command::Wait(args::Wait {
				event: Some(WaitFor::Reload(args))
			}) => log::wait_reload(self, args.since, args.timeout, args.json),
			Command::Wait(args::Wait {
				event: Some(WaitFor::Reboot(args))
			}) => log::wait_reboot(self, args.since, args.timeout, args.json),
			Command::Trace(args) => trace::trace(self, &args),
			// (these aren't a world's, and were done without one)
			Command::Wait(args::Wait { event: None })
			| Command::Launch(_)
			| Command::List(_)
			| Command::Dap
			| Command::Help(_)
			| Command::Version(_) => Ok(())
		}
	}

	/// Print auxiliary output to stderr in JSON mode.
	fn aside(&self, text: &str) {
		if self.json.is_some() {
			eprintln!("{}", text);
		} else {
			say!("{}", text);
		}
	}

	fn leave(&mut self) {
		let _ = local::send(&mut self.stream, &Request::Disconnect);
	}

	/// Receive the next non-notification message, or `None` at the deadline.
	fn next(&mut self, until: Option<Instant>) -> Result<Option<Response>> {
		loop {
			let response = match until {
				Some(until) => match self.received.recv_timeout(until.saturating_duration_since(Instant::now())) {
					Ok(response) => response,
					Err(RecvTimeoutError::Timeout) => return Ok(None),
					Err(RecvTimeoutError::Disconnected) => return Err(hung_up())
				},
				None => self.received.recv().map_err(|_| hung_up())?
			};
			match response.map_err(|_| hung_up())? {
				Response::Disconnect => return Err(hung_up()),
				Response::Notification { message } => self.notes.push(message),
				Response::CodeChanged => self.reloaded = true,
				// Report each breakpoint condition error once; failed hits are
				// skipped.
				Response::ConditionFailed { message } => {
					eprintln!("{}", message);
					self.failed_conditions.push(message);
				}
				Response::WatchEnded { reason, .. } => self.aside(&format!("watch ended: {}", reason)),
				response => return Ok(Some(response))
			}
		}
	}

	fn request(&mut self, request: Request) -> Result<Response> {
		let until = Instant::now() + self.patience;
		self.request_by(request, until)?.ok_or_else(|| self.unresponsive("respond"))
	}

	/// Send a request and receive its answer, or `None` at the deadline.
	fn request_by(&mut self, request: Request, until: Instant) -> Result<Option<Response>> {
		local::send(&mut self.stream, &request).map_err(|_| hung_up())?;
		loop {
			match self.next(Some(until))? {
				None => return Ok(None),
				Some(event @ (Response::BreakpointHit { .. } | Response::Resumed | Response::StepEnded | Response::Log { .. })) => {
					self.events.push(event)
				}
				Some(response) => return Ok(Some(response))
			}
		}
	}

	/// Report a response or pause timeout.
	fn unresponsive(&self, act: &str) -> Failure {
		Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for the world to {}; it is busy or hung. Set --response-timeout to wait longer.",
				seconds_text(self.patience),
				act
			)
		)
	}

	/// Why it's paused, if it is
	fn paused(&mut self) -> Result<Option<BreakpointReason>> {
		match self.request(Request::Status)? {
			Response::Status { paused } => Ok(paused.map(|stop| stop.reason)),
			other => Err(unexpected(&other))
		}
	}

	/// `status`: what the world is doing, how long it has been up, and what
	/// came of its last hot reload
	fn status(&mut self) -> Result<()> {
		let reason = self.paused()?;
		let (mut document, mut lines) = self.execution(reason.as_ref())?;
		let (loaded, up, world_time, loads, last_reload) = match self.request(Request::Uptime)? {
			Response::Uptime {
				loaded,
				up,
				world_time,
				loads,
				last_reload,
				..
			} => (loaded, up, world_time, loads, last_reload),
			other => return Err(unexpected(&other))
		};
		document.insert("uptime_ms".to_owned(), json!((up * 1000.0).round() as u64));
		document.insert("world_time".to_owned(), json!(world_time));
		document.insert("loads".to_owned(), json!(loads));
		document.insert("last_reload".to_owned(), last_reload.as_ref().map_or(Value::Null, log::to_json));
		lines.push(format!(
			"uptime: {} (loaded at {}), world.time {}, world loads: {}",
			log::duration(up),
			log::clock(loaded),
			log::number(world_time),
			loads
		));
		lines.push(match &last_reload {
			Some(entry) => format!("last reload: {}", log::block(entry)),
			None => "last reload: none".to_owned()
		});
		self.print(document, &lines);
		Ok(())
	}

	/// A command's result: the JSON document, or the lines that say the same
	fn print(&self, document: Map<String, Value>, lines: &[String]) {
		if self.json.is_some() {
			say!("{}", Value::Object(document));
		} else {
			for line in lines {
				say!("{}", line);
			}
		}
	}

	fn frames(&mut self, stack_id: u32, count: Option<u32>) -> Result<Vec<StackFrame>> {
		match self.request(Request::StackFrames {
			stack_id,
			start_frame: None,
			count
		})? {
			Response::StackFrames { frames, .. } => Ok(frames),
			other => Err(unexpected(&other))
		}
	}

	/// Build the shared execution status fields and their text representation.
	/// `reason` is `None` while running.
	fn execution(&mut self, reason: Option<&BreakpointReason>) -> Result<(Map<String, Value>, Vec<String>)> {
		let Some(reason) = reason else {
			return Ok((json::execution(self.pid, None), vec![format!("pid {}: running", self.pid)]));
		};
		let why = match reason {
			BreakpointReason::Breakpoint => "at a breakpoint".to_owned(),
			BreakpointReason::Step => "after a step".to_owned(),
			BreakpointReason::Pause => "by request".to_owned(),
			BreakpointReason::Runtime { message, caught: false } => format!("on a runtime error: {}", message),
			BreakpointReason::Runtime { message, caught: true } => format!("on a caught runtime error: {}", message),
			BreakpointReason::Data { changes } => {
				let changes: Vec<String> = changes
					.iter()
					.map(|change| format!("{} changed from {} to {}", change.name, change.old, change.new))
					.collect();
				format!("after a change: {}", changes.join("; "))
			}
		};
		let frames = self.frames(0, Some(20))?;
		let instruction = match (self.show_instruction, frames.first()) {
			(true, Some(top)) => Some(self.instruction_of(top)?),
			_ => None
		};
		let mut lines = vec![format!("pid {}: paused {}", self.pid, why)];
		for (i, frame) in frames.iter().enumerate() {
			lines.push(format!("#{} {}", i, place(frame)));
			if let (0, Some((offset, text))) = (i, &instruction) {
				lines.push(format!("   instruction {}: {}", offset, text));
			}
		}
		Ok((json::execution(self.pid, Some((reason, &frames, instruction))), lines))
	}

	/// Prints what the world is doing, with what `wait pause` adds to it
	fn report(&mut self, reason: Option<&BreakpointReason>) -> Result<()> {
		let (mut document, lines) = self.execution(reason)?;
		document.extend(std::mem::take(&mut self.waited));
		self.print(document, &lines);
		Ok(())
	}

	/// The offset of the instruction a frame is about to run, and its text
	fn instruction_of(&mut self, frame: &StackFrame) -> Result<(u32, String)> {
		let offset = frame.instruction.offset;
		let disassembly = self.instructions(frame.code)?;
		let text = disassembly
			.instructions
			.iter()
			.find(|instruction| u32::from(instruction.offset) == offset)
			.map(|instruction| instruction.text.clone())
			.filter(|text| !text.is_empty())
			.unwrap_or_else(|| "(not read)".to_owned());
		Ok((offset, text))
	}

	fn print_stack(&mut self, stack_id: u32, count: Option<u32>) -> Result<()> {
		for (i, frame) in self.frames(stack_id, count)?.iter().enumerate() {
			say!("#{} {}", i, place(frame));
		}
		Ok(())
	}

	fn need_pause(&mut self) -> Result<()> {
		match self.paused()? {
			Some(_) => Ok(()),
			None => Err("the world is running. Pause it first with 'auxtools pause' or 'auxtools wait pause'.".into())
		}
	}

	fn stack(&mut self, all: bool) -> Result<()> {
		self.need_pause()?;
		// Every sleeping proc, when they're asked for
		let stacks = match self.request(Request::Stacks {
			sleeping: Some(if all { u32::MAX } else { 0 })
		})? {
			Response::Stacks { stacks, .. } => stacks,
			other => return Err(unexpected(&other))
		};
		if self.json.is_some() {
			let mut listed = vec![];
			for stack in stacks.iter().filter(|s| all || !s.sleeping) {
				let frames: Vec<_> = self.frames(stack.id, None)?.iter().map(json::frame).collect();
				listed.push(match all {
					true => json!({ "id": stack.id, "name": stack.name, "sleeping": stack.sleeping, "frames": frames }),
					false => Value::Array(frames)
				});
			}
			let document = match all {
				true => Value::Array(listed),
				false => listed.pop().unwrap_or_else(|| json!([]))
			};
			say!("{}", document);
			return Ok(());
		}
		for stack in stacks.iter().filter(|s| all || !s.sleeping) {
			if all {
				say!("stack {}: {}{}", stack.id, stack.name, if stack.sleeping { " (sleeping)" } else { "" });
			}
			self.print_stack(stack.id, None)?;
		}
		Ok(())
	}

	/// The id of the `n`th frame of the paused stack
	fn frame_id(&mut self, n: u32) -> Result<u32> {
		self.need_pause()?;
		let frames = self.frames(0, Some(n.saturating_add(1)))?;
		match frames.get(n as usize) {
			Some(frame) => Ok(frame.id),
			None => Err(format!(
				"frame {} does not exist: the paused stack has {} frames (0 is innermost).",
				n,
				frames.len()
			)
			.into())
		}
	}

	fn vars(&mut self, frame: u32, globals: bool) -> Result<()> {
		let frame_id = self.frame_id(frame)?;
		let Response::Scopes {
			arguments,
			locals,
			globals: global_vars
		} = self.request(Request::Scopes { frame_id })?
		else {
			return Err("the world did not report the frame's variables.".into());
		};
		let mut scopes = vec![("arguments", arguments), ("locals", locals)];
		if globals {
			scopes.push(("globals", global_vars));
		}
		// The vars are a level down from the frame
		if let Some(depth) = self.json {
			let mut document = Map::new();
			for (name, vars) in scopes {
				let values = match vars {
					Some(vars) => {
						let depth = depth.saturating_sub(1);
						match self.request(Request::RenderVariables { vars, depth })? {
							Response::RenderVariables(values) => values?,
							other => return Err(unexpected(&other))
						}
					}
					None => vec![]
				};
				document.insert(name.to_owned(), json::named(&values));
			}
			say!("{}", Value::Object(document));
			return Ok(());
		}
		for (name, vars) in scopes {
			say!("{}:", name);
			let Some(vars) = vars else { continue };
			let listed = self.request(Request::Variables {
				vars,
				start: None,
				count: None
			})?;
			let Response::Variables { vars } = listed else {
				return Err(format!("the world did not list the {}.", name).into());
			};
			for var in vars {
				say!("  {} = {}", var.name, var.value);
			}
		}
		Ok(())
	}

	fn eval(&mut self, expression: &str, frame: Option<u32>) -> Result<()> {
		let frame_id = match frame {
			Some(n) => Some(self.frame_id(n)?),
			// The top frame, if it's paused
			None => match self.paused()? {
				Some(_) => Some(self.frame_id(0)?),
				None => None
			}
		};
		self.notes.clear();
		if let Some(depth) = self.json {
			let result = match self.request(Request::Render {
				frame_id,
				expression: expression.to_owned(),
				depth
			})? {
				Response::Render(result) => result,
				other => return Err(unexpected(&other))
			};
			for note in std::mem::take(&mut self.notes) {
				eprintln!("{}", note);
			}
			say!("{}", json::value(&result?));
			return Ok(());
		}
		let result = match self.request(Request::Eval {
			frame_id,
			command: expression.to_owned(),
			context: Some("repl".to_owned())
		})? {
			Response::Eval(result) => result,
			other => return Err(unexpected(&other))
		};
		// What the world said on the way
		for note in std::mem::take(&mut self.notes) {
			eprintln!("{}", note);
		}
		say!("{}", result?.value);
		Ok(())
	}

	/// `eval --src`: with `src` an object, in no frame
	fn eval_on(&mut self, object: &str, expression: &str) -> Result<()> {
		self.notes.clear();
		let response = self.request(Request::EvalOn {
			object: object.to_owned(),
			expression: expression.to_owned(),
			depth: self.json
		})?;
		for note in std::mem::take(&mut self.notes) {
			eprintln!("{}", note);
		}
		match response {
			Response::Render(result) => say!("{}", json::value(&result?)),
			Response::Eval(result) => say!("{}", result?.value),
			other => return Err(unexpected(&other))
		}
		Ok(())
	}

	/// Spawn evaluation on the next tick and optionally wait for its result.
	fn spawn(&mut self, expression: &str, object: Option<&str>, wait: Option<Option<Duration>>) -> Result<()> {
		// Capture the journal position before spawning so the result cannot be
		// missed.
		let newest = match self.request(Request::Journal {
			after: u64::MAX,
			kinds: vec![],
			limit: Some(0),
			follow: false
		})? {
			Response::Journal { last, .. } => last,
			other => return Err(unexpected(&other))
		};
		self.notes.clear();
		let number = match self.request(Request::Spawn {
			expression: expression.to_owned(),
			object: object.map(str::to_owned)
		})? {
			Response::Spawned(number) => number,
			other => return Err(unexpected(&other))
		};
		for note in std::mem::take(&mut self.notes) {
			eprintln!("{}", note);
		}
		let number = number?;
		let Some(timeout) = wait else {
			match self.json {
				Some(_) => say!("{}", json!({ "spawn": number })),
				None => say!("spawn {}", number)
			}
			return Ok(());
		};
		if self.paused()?.is_some() {
			eprintln!("the world is paused: spawn {} runs after execution resumes.", number);
		}

		let until = timeout.map(|timeout| Instant::now() + timeout);
		let mut follower = log::Follower::new(newest, vec![JournalKind::Spawned, JournalKind::World, JournalKind::Runtime]);
		loop {
			let Some(entry) = follower.next(self, until)? else {
				return Err(Failure(
					TIMED_OUT,
					format!(
						"timed out after {} waiting for spawn {} to finish. It keeps running.",
						seconds_text(timeout.unwrap_or_default()),
						number
					)
				));
			};
			match entry.event {
				JournalEvent::Spawned { spawn, result, .. } if spawn == number => {
					let value = result?;
					match self.json {
						Some(_) => say!("{}", json::value(&value.rendered)),
						None => say!("{}", value.text)
					}
					return Ok(());
				}
				JournalEvent::Loaded { .. } => return Err(Failure(HUNG_UP, format!("the world rebooted before spawn {} finished.", number))),
				// Report non-fatal callee errors. The journal cannot identify
				// which spawn caused them.
				JournalEvent::Runtime {
					message,
					caught: false,
					stack
				} if stack.len() > 1 && stack.last().is_some_and(|frame| frame.proc == "/auxtools_spawn_stub") => {
					eprintln!("{}", log::callee_runtime(&message, &stack));
				}
				_ => {}
			}
		}
	}

	/// What the world knows of a proc
	fn find_proc(&mut self, proc: &ProcRef) -> Result<ProcInfo> {
		match self.request(Request::ProcInfo { proc: proc.clone() })? {
			Response::ProcInfo(info) => Ok(info?),
			other => Err(unexpected(&other))
		}
	}

	fn proc_info(&mut self, proc: &ProcRef) -> Result<()> {
		let info = self.find_proc(proc)?;
		let reloaded = info.reloaded.map(|how| match how {
			Reloaded::Lines => "lines",
			Reloaded::Code => "code",
			Reloaded::Added => "added"
		});
		let document = json!({
			"proc": proc_name(&info.proc),
			"file": info.file,
			"line": info.line,
			"definitions": info.definitions,
			"reloaded": reloaded
		});
		let mut place = proc_name(&info.proc);
		match (&info.file, info.line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(Some(file), None) => place += &format!("  {}", file),
			(None, Some(line)) => place += &format!("  line {}", line),
			(None, None) => {}
		}
		let lines = [
			place,
			format!("definitions: {}", info.definitions),
			format!("reloaded: {}", match info.reloaded {
				None => "no",
				Some(Reloaded::Lines) => "lines (same code, moved to other lines)",
				Some(Reloaded::Code) => "code (new code)",
				Some(Reloaded::Added) => "added (new proc)"
			})
		];
		self.print(document.as_object().cloned().unwrap_or_default(), &lines);
		Ok(())
	}

	/// The instructions of some code, as the world reads them
	fn instructions(&mut self, code: CodeRef) -> Result<Disassembly> {
		match self.request(Request::Disassemble { code })? {
			Response::Disassemble(disassembly) => Ok(disassembly?),
			other => Err(unexpected(&other))
		}
	}

	fn disassemble(&mut self, proc: &ProcRef) -> Result<()> {
		let info = self.find_proc(proc)?;
		let disassembly = self.instructions(info.code)?;
		// One the world couldn't read says so where its text would be
		let text = |instruction: &BytecodeInstruction| match (&instruction.text, &disassembly.error) {
			(text, _) if !text.is_empty() => text.clone(),
			(_, Some(error)) => format!("(not read: {})", error),
			(_, None) => "(not read)".to_owned()
		};
		if self.json.is_some() {
			let instructions: Vec<_> = disassembly
				.instructions
				.iter()
				.map(|instruction| json!({ "offset": instruction.offset, "text": text(instruction), "line": instruction.line }))
				.collect();
			let document = json!({
				"proc": proc_name(&disassembly.proc),
				"file": disassembly.file,
				"instructions": instructions
			});
			say!("{}", document);
			return Ok(());
		}
		match &disassembly.file {
			Some(file) => say!("{}  {}", proc_name(&disassembly.proc), file),
			None => say!("{}", proc_name(&disassembly.proc))
		}
		let mut line = None;
		for instruction in &disassembly.instructions {
			if instruction.line != line && instruction.line.is_some() {
				line = instruction.line;
				say!("line {}:", line.unwrap());
			}
			say!("{:>6}  {}", instruction.offset, text(instruction));
		}
		Ok(())
	}

	/// Report resolved breakpoint locations on stderr and in JSON.
	fn say_bound(&mut self, place: &str, bound: &Bound) {
		eprintln!("{}", bound.describe());
		if self.json.is_some() {
			let said = json!([{ "place": place, "proc": proc_name(&bound.at.proc), "file": bound.file, "line": bound.line }]);
			self.say_waited("breakpoints", said, None);
		}
	}

	/// Add wait progress to text output or the pause result's JSON fields.
	/// Append array entries when the field already exists.
	fn say_waited(&mut self, key: &str, value: Value, text: Option<&str>) {
		if self.json.is_none() {
			if let Some(text) = text {
				say!("{}", text);
			}
			return;
		}
		match (self.waited.get_mut(key), value) {
			(Some(Value::Array(had)), Value::Array(more)) => had.extend(more),
			(_, value) => {
				self.waited.insert(key.to_owned(), value);
			}
		}
	}

	/// Waits for a pause, and keeps it held after we leave
	fn await_pause(&mut self, until: Option<Instant>, hold: bool) -> Result<Option<BreakpointReason>> {
		self.await_pause_for(until, hold, |_, _| true)
	}

	/// Wait for a pause accepted by `ours`, ignoring other clients' triggers.
	/// Pass whether reload has changed breakpoint locations in the meantime.
	/// Ignored pauses require another client to resume before our trigger can
	/// fire.
	fn await_pause_for(&mut self, until: Option<Instant>, hold: bool, ours: impl Fn(&Stop, bool) -> bool) -> Result<Option<BreakpointReason>> {
		let reason = loop {
			if let Some(i) = self.events.iter().position(|e| matches!(e, Response::BreakpointHit { .. })) {
				let Response::BreakpointHit(stop) = self.events.remove(i) else {
					unreachable!()
				};
				if !ours(&stop, self.reloaded) {
					continue;
				}
				break stop.reason;
			}
			match self.next(until)? {
				None => return Ok(None),
				Some(event @ Response::BreakpointHit { .. }) => self.events.push(event),
				Some(_) => {}
			}
		};
		if hold {
			self.request(Request::HoldPause)?;
		}
		Ok(Some(reason))
	}

	fn pause(&mut self) -> Result<()> {
		if let Some(reason) = self.paused()? {
			self.request(Request::HoldPause)?;
			return self.report(Some(&reason));
		}
		self.request(Request::Pause)?;
		// A running world pauses at its next instruction.
		let until = Instant::now() + self.patience;
		match self.await_pause(Some(until), true)? {
			Some(reason) => self.report(Some(&reason)),
			None => Err(self.unresponsive("pause"))
		}
	}

	/// `continue`: lets a paused world carry on, and doesn't wait for more
	fn resume(&mut self) -> Result<()> {
		if self.paused()?.is_none() {
			return Err("the world is not paused.".into());
		}
		self.request(Request::Continue {
			kind: ContinueKind::Continue
		})?;
		let document = json!({ "pid": self.pid, "resumed": true });
		self.print(document.as_object().cloned().unwrap_or_default(), &[format!("pid {}: resumed", self.pid)]);
		Ok(())
	}

	/// Resume for a step and wait for a stop or step-exhausted notification.
	fn step(&mut self, kind: ContinueKind, timeout: Option<Duration>) -> Result<()> {
		self.need_pause()?;
		self.events.clear();
		self.request(Request::Continue { kind })?;
		let until = timeout.map(|timeout| Instant::now() + timeout);
		let stepped = loop {
			if let Some(i) = self
				.events
				.iter()
				.position(|e| matches!(e, Response::BreakpointHit { .. } | Response::StepEnded))
			{
				match self.events.remove(i) {
					Response::BreakpointHit(stop) => break Stepped::Paused(stop.reason),
					_ => break Stepped::Ended
				}
			}
			match self.next(until)? {
				None => break Stepped::TimedOut,
				Some(event @ (Response::BreakpointHit { .. } | Response::StepEnded)) => self.events.push(event),
				Some(_) => {}
			}
		};
		match stepped {
			Stepped::Paused(reason) => {
				self.request(Request::HoldPause)?;
				self.report(Some(&reason))
			}
			Stepped::Ended => {
				eprintln!("step completed with the world running: no frame remained to pause in.");
				self.report(None)
			}
			Stepped::TimedOut => {
				// Query execution status rather than inferring it from missing
				// stop events.
				if let Some(reason) = self.observe(Duration::from_secs(2)) {
					self.report(reason.as_ref())?;
				}
				Err(Failure(
					TIMED_OUT,
					format!(
						"timed out after {} waiting for the step to complete. The world was neither paused nor resumed by this.",
						seconds_text(timeout.unwrap_or_default())
					)
				))
			}
		}
	}

	/// Query pause state within `patience`: `Some(None)` means running.
	fn observe(&mut self, patience: Duration) -> Option<Option<BreakpointReason>> {
		local::send(&mut self.stream, &Request::Status).ok()?;
		let until = Instant::now() + patience;
		loop {
			match self.next(Some(until)) {
				Ok(Some(Response::Status { paused })) => return Some(paused.map(|stop| stop.reason)),
				Ok(Some(_)) => {}
				Ok(None) | Err(_) => return None
			}
		}
	}

	fn wait(&mut self, wait: Wait) -> Result<()> {
		if self.json.is_some() {
			self.waited = json!({ "already": false, "breakpoints": [], "watching": [] })
				.as_object()
				.cloned()
				.unwrap_or_default();
		}
		let paused = self.paused()?;
		// Track reloads that may move the breakpoints installed below.
		self.reloaded = false;
		// Set requested breakpoints before installing watches.
		// With --change, conditions apply to the watches.
		let condition = if wait.changes.is_empty() { wait.condition.clone() } else { None };
		let mut ours_at = vec![];
		let mut ours_watches = vec![];
		for place in &wait.breaks {
			for bound in self.set_breakpoint(place, condition.clone(), None)? {
				self.say_bound(place, &bound);
				ours_at.push(bound.at.clone());
			}
		}
		if let (Some(reason), true) = (&paused, wait.changes.is_empty()) {
			if wait.hold {
				self.request(Request::HoldPause)?;
			}
			self.say_waited("already", Value::Bool(true), Some("already paused"));
			return self.report(Some(reason));
		}
		// What's watched from a pause is of a frame there, and the wait is
		// for the world to carry on and change it
		let frame_id = match &paused {
			Some(_) => Some(self.frame_id(wait.frame)?),
			None => None
		};
		for expression in &wait.changes {
			let watch = Request::WatchSet {
				place: WatchPlace::Expression {
					frame_id,
					expression: expression.clone()
				},
				condition: wait.condition.clone(),
				hit: None
			};
			match self.request(watch)? {
				Response::WatchSet(Ok(watch)) => {
					ours_watches.push(watch.id);
					let said = json!([{ "name": watch.name, "value": watch.value }]);
					self.say_waited("watching", said, Some(&format!("watching {}, now {}", watch.name, watch.value)));
				}
				Response::WatchSet(Err(why)) => return Err(format!("cannot watch {}: {}", expression, why).into()),
				other => return Err(unexpected(&other))
			}
		}
		if paused.is_some() {
			self.events.clear();
			self.request(Request::Continue {
				kind: ContinueKind::Continue
			})?;
		}
		if wait.runtimes {
			// It has no answer
			let catch = Request::CatchRuntimes {
				uncaught: Some(wait.runtime_filter.clone()),
				caught: wait.caught.then(|| wait.runtime_filter.clone())
			};
			local::send(&mut self.stream, &catch).map_err(|_| hung_up())?;
		}
		let until = wait.timeout.map(|t| Instant::now() + t);
		// With triggers, accept only matching pauses; otherwise accept any
		// pause.
		let triggered = !wait.breaks.is_empty() || !wait.changes.is_empty() || wait.runtimes;
		let ours = |stop: &Stop, reloaded: bool| {
			!triggered
				|| match &stop.reason {
					// Reloads do not report new breakpoint offsets. Accept a
					// hit in any proc containing one of our breakpoints after a
					// reload.
					BreakpointReason::Breakpoint => ours_at.contains(&stop.at) || (reloaded && ours_at.iter().any(|at| at.proc == stop.at.proc)),
					BreakpointReason::Data { changes } => changes.iter().any(|change| ours_watches.contains(&change.id)),
					// The world pauses when any client's runtime filter
					// matches.
					BreakpointReason::Runtime { caught, message } => wait.runtimes && (wait.caught || !caught) && wait.runtime_filter.takes(message),
					BreakpointReason::Step | BreakpointReason::Pause => false
				}
		};
		match self.await_pause_for(until, wait.hold, ours)? {
			Some(reason) => self.report(Some(&reason)),
			None => Err(Failure(
				TIMED_OUT,
				format!(
					"timed out after {} waiting for the world to pause.",
					seconds_text(wait.timeout.unwrap_or_default())
				)
			))
		}
	}

	/// Set a breakpoint at `FILE:LINE` or `/proc/path#N`. A source line can
	/// resolve to multiple procs. Reject targets without executable code.
	/// With `log`, hits emit `Response::Log` without pausing.
	fn set_breakpoint(&mut self, place: &str, condition: Option<String>, log: Option<Vec<LogPart>>) -> Result<Vec<Bound>> {
		let (instructions, asked_line) = if place.starts_with('/') {
			// Asked first, for what's wrong with it if it isn't there
			let proc = self
				.find_proc(&proc_ref(place)?)
				.map_err(|Failure(code, why)| Failure(code, format!("cannot set a breakpoint at {}: {}", place, why)))?;
			(vec![InstructionRef { proc: proc.proc, offset: 0 }], None)
		} else {
			let (file, line) = place
				.rsplit_once(':')
				.and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
				.ok_or_else(|| format!("invalid location '{}': expected FILE:LINE or /proc/path[#N].", place))?;
			match self.request(Request::Locate { file: file.to_owned(), line })? {
				Response::Locate { instructions } if instructions.is_empty() => return Err(self.nowhere(file, line)?),
				Response::Locate { instructions } => (instructions, Some(line)),
				other => return Err(unexpected(&other))
			}
		};
		let mut bound = vec![];
		for instruction in instructions {
			match self.request(Request::BreakpointSet {
				instruction: instruction.clone(),
				condition: condition.clone(),
				hit: None,
				log: log.clone()
			})? {
				Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at, file }
				} => bound.push(Bound {
					place: place.to_owned(),
					asked: instruction,
					at,
					file,
					line,
					asked_line
				}),
				Response::BreakpointSet {
					result: BreakpointSetResult::Refused { reason }
				} => return Err(format!("cannot set a breakpoint at {}: {}", place, reason).into()),
				Response::BreakpointSet { .. } => return Err(format!("cannot set a breakpoint at {}: no such proc.", place).into()),
				other => return Err(unexpected(&other))
			}
		}
		Ok(bound)
	}

	/// Remove a breakpoint and return the count of hits skipped by condition
	/// errors.
	fn unset_breakpoint(&mut self, bound: &Bound) -> Result<u32> {
		// By what it was set with, which is how the world knows it
		match self.request(Request::BreakpointUnset {
			instruction: bound.asked.clone()
		})? {
			Response::BreakpointUnset { skipped, .. } => Ok(skipped),
			other => Err(unexpected(&other))
		}
	}

	/// Why there's no code at `line` of `file`, or after it
	fn nowhere(&mut self, file: &str, line: u32) -> Result<Failure> {
		let lines = match self.request(Request::LinesWithCode {
			file: file.to_owned(),
			start: 0,
			end: u32::MAX
		})? {
			Response::LinesWithCode { lines } => lines,
			other => return Err(unexpected(&other))
		};
		let mut lines_known = || match self.request(Request::Files) {
			Ok(Response::Files { files }) => files.iter().any(|name| name != "auxtools_launcher.dm"),
			_ => true
		};
		Ok(match lines.last() {
			None if !lines_known() => format!("cannot set a breakpoint at {}:{}: {}.", file, line, crate::adapter::NO_LINES),
			None => format!(
				"cannot set a breakpoint at {}:{}: the world has no code from a file named {}.",
				file, line, file
			),
			Some(last) => format!("cannot set a breakpoint at {}:{}: {} has no code after line {}.", file, line, file, last)
		}
		.into())
	}
}

/// A breakpoint that was set
struct Bound {
	/// The place it was asked for, as it was given
	place: String,
	/// What it was set with
	asked: InstructionRef,
	/// Resolved proc and instruction.
	at: InstructionRef,
	file: Option<String>,
	line: Option<u32>,
	/// The line that was asked for, for one given as FILE:LINE
	asked_line: Option<u32>
}

impl Bound {
	/// Its proc and line, as a frame there is shown
	fn spot(&self) -> String {
		spot(&self.at.proc, false, self.file.as_deref(), self.line)
	}

	/// The same, for `wait pause` to say where it will pause
	fn describe(&self) -> String {
		let mut text = format!("breakpoint set at {}", self.spot());
		if let (Some(asked), Some(line)) = (self.asked_line, self.line) {
			if asked != line {
				text += &format!(" (line {} has no code; this is the next line that has)", asked);
			}
		}
		text
	}
}

fn place(frame: &StackFrame) -> String {
	spot(&frame.instruction.proc, frame.old_code, frame.file.as_deref(), frame.line)
}

/// A proc and where in it
fn spot(proc: &ProcRef, old_code: bool, file: Option<&str>, line: Option<u32>) -> String {
	let name = match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	};
	let name = if old_code { format!("{} (old code)", name) } else { name };
	match (file, line) {
		(Some(file), Some(line)) => format!("{}  {}:{}", name, file, line),
		(None, Some(line)) => format!("{}  line {}", name, line),
		// At its start, before its first line
		(Some(file), None) => format!("{}  {}", name, file),
		(None, None) => name
	}
}

fn hung_up() -> Failure {
	Failure(
		HUNG_UP,
		"world connection ended before the operation completed: the world exited or rebooted.".to_owned()
	)
}

fn unexpected(response: &Response) -> Failure {
	Failure(1, format!("unexpected response from the world: {:?}", response))
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::{expression, given_text, proc_ref, seconds_text, Failure};

	fn args(args: &[&str]) -> Vec<String> {
		args.iter().map(|arg| (*arg).to_owned()).collect()
	}

	/// Temporary test file removed on drop.
	struct Scratch(std::path::PathBuf);

	impl Scratch {
		fn new(name: &str, text: &str) -> Self {
			let path = std::env::temp_dir().join(format!("auxtools-cli-{}-{}", std::process::id(), name));
			std::fs::write(&path, text).unwrap();
			Self(path)
		}

		fn path(&self) -> &str {
			self.0.to_str().unwrap()
		}
	}

	impl Drop for Scratch {
		fn drop(&mut self) {
			let _ = std::fs::remove_file(&self.0);
		}
	}

	#[test]
	fn an_option_reads_its_text_from_a_file_after_an_at() {
		let file = Scratch::new("given", "\"a b\" == /datum/x\r\n");
		assert_eq!(given_text("x > 1").ok().unwrap(), "x > 1");
		assert_eq!(given_text(&format!("@{}", file.path())).ok().unwrap(), "\"a b\" == /datum/x");
		assert_eq!(given_text("@@home").ok().unwrap(), "@home");
		let Err(Failure(1, why)) = given_text("@/no/such/file") else {
			panic!("a file that isn't there was read");
		};
		assert!(why.starts_with("cannot read /no/such/file: "), "{}", why);
	}

	#[test]
	fn evals_expression_is_its_words_a_file_or_stdin() {
		let none = || -> super::Result<String> { panic!("stdin was read") };
		assert_eq!(expression(&args(&["6", "*", "7"]), None, none).ok().unwrap(), "6 * 7");
		let file = Scratch::new("expression", "list(1, \"two\")\n\n");
		assert_eq!(expression(&[], Some(file.0.as_path()), none).ok().unwrap(), "list(1, \"two\")");
		assert_eq!(expression(&args(&["-"]), None, || Ok("1 + 1".to_owned())).ok().unwrap(), "1 + 1");
		// A negative number is an expression, not stdin
		assert_eq!(expression(&args(&["-", "1"]), None, none).ok().unwrap(), "- 1");
		let Err(Failure(1, why)) = expression(&args(&["-"]), None, || Ok("\n".to_owned())) else {
			panic!("nothing was an expression");
		};
		assert_eq!(why, "stdin contains no expression.");
	}

	#[test]
	fn a_proc_is_its_path_and_which_definition() {
		let proc = proc_ref("/mob/proc/attack#2").ok().unwrap();
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/proc/attack", 2));
		let proc = proc_ref("/value").ok().unwrap();
		assert_eq!((proc.path.as_str(), proc.override_id), ("/value", 0));
		for wrong in ["value", "/value#", "/value#two", "#1"] {
			assert!(proc_ref(wrong).is_err(), "{}", wrong);
		}
	}

	#[test]
	fn a_time_is_said_in_seconds() {
		assert_eq!(seconds_text(Duration::from_secs(30)), "30 seconds");
		assert_eq!(seconds_text(Duration::from_secs(1)), "1 second");
		assert_eq!(seconds_text(Duration::from_millis(500)), "0.5 seconds");
		assert_eq!(seconds_text(Duration::ZERO), "0 seconds");
	}
}
