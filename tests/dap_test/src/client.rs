//! DAP test client shared by dap_test and launcher_test. Run the launcher
//! and exchange messages over stdin/stdout as an editor would.

use std::{
	collections::VecDeque,
	io::{BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
	process::{Child, ChildStdin, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use serde_json::{json, Value};

/// Timeout for expected launcher messages.
pub const TIMEOUT: Duration = Duration::from_secs(90);

/// A file next to this program, where cargo puts the launcher and the debug
/// library
pub fn next_to_us(name: &str) -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();
	path.push(name);
	assert!(path.is_file(), "couldn't find {}; build it first", path.display());
	path
}

pub fn launcher_path() -> PathBuf {
	next_to_us(if cfg!(windows) { "auxtools.exe" } else { "auxtools" })
}

pub fn library_path() -> PathBuf {
	next_to_us(if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" })
}

/// The program that runs the worlds: `LAUNCHER_TEST_SERVER`, or the one whose
/// output the launcher can pass on
pub fn server() -> String {
	std::env::var("LAUNCHER_TEST_SERVER").unwrap_or_else(|_| "dreamdaemon".to_owned())
}

/// The program's arguments, when `LAUNCHER_TEST_ARGS` replaces them
pub fn test_args() -> Option<Vec<String>> {
	std::env::var("LAUNCHER_TEST_ARGS")
		.ok()
		.map(|args| args.split_whitespace().map(str::to_owned).collect())
}

/// Runs the launcher with `args`, for its exit code and what it printed
pub fn launcher_command(args: &[&str]) -> (i32, String) {
	let out = Command::new(launcher_path()).args(args).output().unwrap();
	let printed = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
	(out.status.code().unwrap_or(-1), printed)
}

/// Discover worlds belonging to this test run by current directory
/// or process-specific path. Exclude other concurrent runs.
pub fn listed_worlds() -> Vec<Value> {
	let list = Command::new(launcher_path()).args(["list", "--json"]).output().unwrap();
	let worlds: Value = serde_json::from_slice(&list.stdout).unwrap();
	let plain = |path: &str| path.replace('\\', "/").to_lowercase();
	let here = plain(&std::env::current_dir().unwrap().to_string_lossy());
	let made = format!("-{}/", std::process::id());
	worlds
		.as_array()
		.unwrap()
		.iter()
		.filter(|world| {
			let dmb = plain(world["dmb"].as_str().unwrap_or_default());
			dmb.starts_with(&here) || dmb.contains(&made)
		})
		.cloned()
		.collect()
}

/// The process running `dmb`, as the launcher's `--list` finds it
pub fn listed_world(dmb: &str) -> u64 {
	let worlds = listed_worlds();
	let world = worlds
		.iter()
		.find(|world| world["dmb"].as_str().is_some_and(|path| path.ends_with(dmb)))
		.unwrap_or_else(|| panic!("--list didn't list the world: {:?}", worlds));
	world["pid"].as_u64().unwrap()
}

/// Terminate a process immediately.
pub fn kill_process(pid: u64) {
	let killed = if cfg!(windows) {
		Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
	} else {
		Command::new("kill").args(["-9", &pid.to_string()]).output()
	};
	assert!(killed.is_ok_and(|out| out.status.success()), "couldn't kill process {}", pid);
}

/// A stop: why, and the top frame's proc and line
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
	pub reason: String,
	pub proc: String,
	pub line: u64,
	/// The thread it stopped in
	pub thread: u64,
	/// The breakpoints it stopped for, by their ids
	pub hit: Vec<u64>
}

pub struct Launcher {
	pub child: Child,
	stdin: ChildStdin,
	messages: mpsc::Receiver<Value>,
	seq: i64,
	/// Events that came while waiting for something else
	pub events: VecDeque<Value>,
	/// Everything the launcher passed on: the world's output and its own
	pub output: String,
	/// The same by category (`stdout`, `stderr`, `console`)
	pub categories: Vec<(String, String)>,
	/// The output events as they came, whole
	pub outputs: Vec<Value>,
	/// Whether to print what's passed on
	pub echo: bool,
	/// Last stopped thread ID used for stepping; 0 selects the running stack
	/// before the first stop.
	pub thread: u64
}

impl Launcher {
	pub fn start() -> Self {
		let mut child = Command::new(launcher_path())
			.arg("dap")
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.spawn()
			.unwrap();
		let stdin = child.stdin.take().unwrap();
		let mut stdout = BufReader::new(child.stdout.take().unwrap());

		let (sender, messages) = mpsc::channel();
		std::thread::spawn(move || loop {
			let mut length = 0;
			loop {
				let mut line = String::new();
				if stdout.read_line(&mut line).unwrap_or(0) == 0 {
					return;
				}
				let line = line.trim();
				if line.is_empty() {
					break;
				}
				if let Some(value) = line.strip_prefix("Content-Length:") {
					// A launcher that's on its way out can leave half a line
					let Ok(given) = value.trim().parse() else {
						return;
					};
					length = given;
				}
			}
			let mut body = vec![0; length];
			if stdout.read_exact(&mut body).is_err() {
				return;
			}
			if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
				return;
			}
		});

		Self {
			child,
			stdin,
			messages,
			seq: 1,
			events: VecDeque::new(),
			output: String::new(),
			categories: vec![],
			outputs: vec![],
			echo: true,
			thread: 0
		}
	}

	/// Read the next message, or `None` on deadline or launcher exit.
	fn poll_message(&mut self, deadline: Instant) -> Option<Value> {
		let message = self.messages.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
		if message["type"] == "event" && message["event"] == "stopped" {
			self.thread = message["body"]["threadId"].as_u64().expect("a stop in no thread");
		}
		if message["type"] == "event" && message["event"] == "output" {
			// Windows' programs end their lines with a carriage return too
			let output = message["body"]["output"].as_str().unwrap_or_default().replace("\r\n", "\n");
			if self.echo {
				print!("  | {}", output);
			}
			self.output.push_str(&output);
			self.outputs.push(message["body"].clone());
			self.categories
				.push((message["body"]["category"].as_str().unwrap_or_default().to_owned(), output));
		}
		Some(message)
	}

	fn next_message(&mut self, deadline: Instant) -> Value {
		self.poll_message(deadline)
			.unwrap_or_else(|| panic!("timed out waiting for launcher response; output:\n{}", self.output))
	}

	pub fn send(&mut self, command: &str, arguments: Value) -> i64 {
		let seq = self.seq;
		self.seq += 1;
		let message = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }).to_string();
		write!(self.stdin, "Content-Length: {}\r\n\r\n{}", message.len(), message).unwrap();
		self.stdin.flush().unwrap();
		seq
	}

	/// Send a request and return its successful response body.
	pub fn request(&mut self, command: &str, arguments: Value) -> Value {
		let response = self.try_request(command, arguments);
		assert_eq!(response["success"], true, "{} failed: {}", command, response);
		response["body"].clone()
	}

	/// Sends a request and returns its whole response
	pub fn try_request(&mut self, command: &str, arguments: Value) -> Value {
		let seq = self.send(command, arguments);
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let message = self.next_message(deadline);
			if message["type"] == "response" && message["request_seq"] == seq {
				return message;
			}
			if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
	}

	/// Send a request expected to fail and return its diagnostic.
	pub fn refused(&mut self, command: &str, arguments: Value) -> String {
		let response = self.try_request(command, arguments);
		assert_eq!(response["success"], false, "{} should have failed: {}", command, response);
		response["message"].as_str().unwrap_or_default().to_owned()
	}

	/// Waits for an event, skipping others
	pub fn event(&mut self, name: &str) -> Value {
		self.event_within(name, TIMEOUT)
			.unwrap_or_else(|| panic!("missing {} event; output:\n{}", name, self.output))
	}

	/// Waits up to `wait` for an event, skipping others
	pub fn event_within(&mut self, name: &str, wait: Duration) -> Option<Value> {
		if let Some(at) = self.events.iter().position(|e| e["event"] == name) {
			return self.events.remove(at);
		}
		let deadline = Instant::now() + wait;
		loop {
			let message = self.poll_message(deadline)?;
			if message["type"] == "event" {
				if message["event"] == name {
					return Some(message);
				}
				self.events.push_back(message);
			}
		}
	}

	/// Whether an event of this name has come and not been taken
	pub fn has_event(&mut self, name: &str) -> bool {
		// Whatever has arrived is looked at first
		while let Some(message) = self.poll_message(Instant::now()) {
			if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
		self.events.iter().any(|e| e["event"] == name)
	}

	/// Forgets the events of this name that have come
	pub fn drop_events(&mut self, name: &str) {
		self.has_event(name);
		self.events.retain(|e| e["event"] != name);
	}

	/// Waits for the world to stop, and checks why
	pub fn stopped(&mut self, reason: &str) -> Value {
		let event = self.event("stopped");
		assert_eq!(event["body"]["reason"], reason, "stopped for the wrong reason: {}", event);
		event
	}

	/// Waits for the world to stop, and says why and where
	pub fn stop(&mut self) -> Stop {
		let event = self.event("stopped");
		let frame = self.top_frame();
		let ids = |list: &Value| -> Vec<u64> {
			list.as_array()
				.map(|ids| ids.iter().filter_map(Value::as_u64).collect())
				.unwrap_or_default()
		};
		Stop {
			reason: event["body"]["reason"].as_str().unwrap_or_default().to_owned(),
			proc: frame["name"].as_str().unwrap_or_default().to_owned(),
			line: frame["line"].as_u64().unwrap_or(0),
			thread: event["body"]["threadId"].as_u64().unwrap(),
			hit: ids(&event["body"]["hitBreakpointIds"])
		}
	}

	/// Waits for the world to stop at a breakpoint in `proc` on `line`
	pub fn stop_at(&mut self, reason: &str, proc: &str, line: u64) {
		let stop = self.stop();
		assert_eq!(
			(stop.reason.as_str(), stop.proc.as_str(), stop.line),
			(reason, proc, line),
			"stopped somewhere else"
		);
	}

	pub fn frames(&mut self, thread: u64) -> Vec<Value> {
		let trace = self.request("stackTrace", json!({ "threadId": thread }));
		trace["stackFrames"].as_array().unwrap().clone()
	}

	pub fn top_frame(&mut self) -> Value {
		let threads = self.request("threads", json!({}));
		let names: Vec<&str> = threads["threads"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
		assert!(
			!names.iter().any(|name| name.contains("auxtools_keepalive")),
			"the debugger's own loop is a thread: {:?}",
			names
		);
		// Request frames for the stopped thread and verify it appears
		// in the thread list.
		let thread = self.thread;
		assert!(
			thread == 0 || threads["threads"].as_array().unwrap().iter().any(|t| t["id"] == thread),
			"the world stopped in thread {}, which isn't listed: {}",
			thread,
			threads
		);
		let trace = self.request("stackTrace", json!({ "threadId": thread }));
		trace["stackFrames"][0].clone()
	}

	/// The id of the paused proc's frame
	pub fn frame_id(&mut self) -> Value {
		self.top_frame()["id"].clone()
	}

	/// The variables reference of a frame's scope
	pub fn scope(&mut self, frame: &Value, name: &str) -> Value {
		let scopes = self.request("scopes", json!({ "frameId": frame }));
		scopes["scopes"]
			.as_array()
			.unwrap()
			.iter()
			.find(|s| s["name"] == name)
			.unwrap_or_else(|| panic!("no {} scope: {}", name, scopes))["variablesReference"]
			.clone()
	}

	pub fn variables(&mut self, reference: &Value) -> Vec<Value> {
		let vars = self.request("variables", json!({ "variablesReference": reference }));
		vars["variables"].as_array().unwrap().clone()
	}

	/// A scope of the paused proc's frame, as names and values
	pub fn scope_values(&mut self, name: &str) -> Vec<(String, String)> {
		let frame = self.frame_id();
		let reference = self.scope(&frame, name);
		named(&self.variables(&reference))
	}

	/// A variable in a scope of the paused proc's frame
	pub fn variable(&mut self, scope: &str, name: &str) -> Value {
		let frame = self.frame_id();
		let reference = self.scope(&frame, scope);
		let vars = self.variables(&reference);
		vars.iter()
			.find(|v| v["name"] == name)
			.unwrap_or_else(|| panic!("no {} among the {}: {:?}", name, scope, named(&vars)))
			.clone()
	}

	/// The value of a local of the paused proc
	pub fn local(&mut self, name: &str) -> String {
		self.variable("Locals", name)["value"].as_str().unwrap().to_owned()
	}

	/// What's inside a variable, as names and values
	pub fn children(&mut self, variable: &Value) -> Vec<(String, String)> {
		assert_ne!(variable["variablesReference"], 0, "{} can't be opened", variable["name"]);
		let vars = self.variables(&variable["variablesReference"].clone());
		named(&vars)
	}

	pub fn evaluate(&mut self, frame: &Value, expression: &str) -> String {
		let result = self.request("evaluate", json!({ "expression": expression, "frameId": frame["id"], "context": "repl" }));
		result["result"].as_str().unwrap().to_owned()
	}

	/// An expression in the paused proc's frame
	pub fn eval(&mut self, expression: &str) -> String {
		let frame = self.top_frame();
		self.evaluate(&frame, expression)
	}

	/// An expression in no frame, which works while the world runs
	pub fn eval_global(&mut self, expression: &str) -> String {
		let result = self.request("evaluate", json!({ "expression": expression, "context": "repl" }));
		result["result"].as_str().unwrap().to_owned()
	}

	pub fn resume(&mut self) {
		self.request("continue", json!({ "threadId": self.thread }));
	}

	/// `next`, `stepIn` or `stepOut` in the thread that stopped, and where
	/// it stopped
	pub fn step(&mut self, command: &str) -> Stop {
		self.request(command, json!({ "threadId": self.thread }));
		self.stop()
	}

	/// A step that has to stop after a step in `proc` on `line`
	pub fn step_to(&mut self, command: &str, proc: &str, line: u64) {
		let stop = self.step(command);
		assert_eq!(
			(stop.reason.as_str(), stop.proc.as_str(), stop.line),
			("step", proc, line),
			"{} stopped somewhere else",
			command
		);
	}

	pub fn initialize(&mut self) -> Value {
		self.initialize_as(json!({}))
	}

	/// The same, as a client that says more of what it can do
	pub fn initialize_as(&mut self, abilities: Value) -> Value {
		let mut arguments = json!({ "adapterID": "auxtools", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path" });
		for (name, value) in abilities.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		let capabilities = self.request("initialize", arguments);
		assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
		capabilities
	}

	/// Launches the world of `dme` with the debug library next to this
	/// program, and waits for the launcher to ask for breakpoints. `more` adds
	/// to the launch's arguments, or replaces them.
	pub fn launch(&mut self, byond: &Path, dme: &Path, more: Value) {
		let mut arguments = json!({
			"dme": dme,
			"byondPath": byond,
			"server": server(),
			"debugLibrary": library_path(),
			"args": test_args(),
			"hotReload": false
		});
		for (name, value) in more.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		self.request("launch", arguments);
		self.event("initialized");
	}

	/// Attaches to the world in process `pid`, and says there are no
	/// breakpoints to wait for
	pub fn attach(&mut self, pid: u64, more: Value) {
		let mut arguments = json!({ "processId": pid });
		for (name, value) in more.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		self.request("attach", arguments);
		self.event("initialized");
		self.request("configurationDone", json!({}));
	}

	/// Sets a file's breakpoints: each a line, or a whole breakpoint
	pub fn set_breakpoints(&mut self, file: &Path, breakpoints: Value) -> Vec<Value> {
		let breakpoints: Vec<Value> = breakpoints
			.as_array()
			.unwrap()
			.iter()
			.map(|b| if b.is_object() { b.clone() } else { json!({ "line": b }) })
			.collect();
		let set = self.request("setBreakpoints", json!({ "source": { "path": file }, "breakpoints": breakpoints }));
		set["breakpoints"].as_array().unwrap().clone()
	}

	/// Sets a file's breakpoints, which all have to be taken
	pub fn break_at(&mut self, file: &Path, lines: &[u64]) {
		for (breakpoint, line) in self.set_breakpoints(file, json!(lines)).iter().zip(lines) {
			assert_eq!(breakpoint["verified"], true, "the breakpoint on line {} wasn't set: {}", line, breakpoint);
		}
	}

	pub fn function_breakpoints(&mut self, names: &[&str]) -> Vec<Value> {
		let breakpoints: Vec<Value> = names.iter().map(|name| json!({ "name": name })).collect();
		let set = self.request("setFunctionBreakpoints", json!({ "breakpoints": breakpoints }));
		set["breakpoints"].as_array().unwrap().clone()
	}

	/// Whether to pause on runtime errors that nothing catches
	pub fn catch_runtimes(&mut self, catch: bool) {
		let filters: &[&str] = if catch { &["runtimes"] } else { &[] };
		self.request("setExceptionBreakpoints", json!({ "filters": filters }));
	}

	/// Which runtime errors to pause on: the filters ticked without a
	/// condition, and those with one, as `(filter, condition)`
	pub fn exception_filters(&mut self, filters: &[&str], with_conditions: &[(&str, &str)]) {
		let options: Vec<Value> = with_conditions
			.iter()
			.map(|(filter, condition)| json!({ "filterId": filter, "condition": condition }))
			.collect();
		self.request("setExceptionBreakpoints", json!({ "filters": filters, "filterOptions": options }));
	}

	/// Sends every request before reading an answer, as a client does with
	/// its watches, and returns the responses in the requests' order
	pub fn pipeline(&mut self, requests: &[(&str, Value)]) -> Vec<Value> {
		let seqs: Vec<i64> = requests
			.iter()
			.map(|(command, arguments)| self.send(command, arguments.clone()))
			.collect();
		let deadline = Instant::now() + TIMEOUT;
		let mut responses: Vec<Option<Value>> = vec![None; seqs.len()];
		while responses.iter().any(Option::is_none) {
			let message = self.next_message(deadline);
			if message["type"] == "response" {
				if let Some(at) = seqs.iter().position(|seq| message["request_seq"] == *seq) {
					responses[at] = Some(message);
				}
			} else if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
		responses.into_iter().flatten().collect()
	}

	/// The ids and names of the threads
	pub fn threads(&mut self) -> Vec<(u64, String)> {
		let threads = self.request("threads", json!({}));
		threads["threads"]
			.as_array()
			.unwrap()
			.iter()
			.map(|t| (t["id"].as_u64().unwrap(), t["name"].as_str().unwrap().to_owned()))
			.collect()
	}

	/// An expression in a frame, in a context
	pub fn evaluate_in(&mut self, frame: &Value, expression: &str, context: &str) -> Value {
		self.request("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }))
	}

	/// Waits for `text` in what the launcher passes on
	pub fn wait_output(&mut self, text: &str) {
		self.wait_count(text, 1);
	}

	/// Waits for `text` to have been passed on `count` times
	pub fn wait_count(&mut self, text: &str, count: usize) {
		let deadline = Instant::now() + TIMEOUT;
		while self.output.matches(text).count() < count {
			let message = self.next_message(deadline);
			if message["type"] == "event" && message["event"] != "output" {
				self.events.push_back(message);
			}
		}
	}

	/// Wait until accumulated output satisfies `done`.
	pub fn wait_until(&mut self, what: &str, done: impl Fn(&str) -> bool) {
		let deadline = Instant::now() + TIMEOUT;
		while !done(&self.output) {
			let Some(message) = self.poll_message(deadline) else {
				panic!("timed out waiting for {}; output:\n{}", what, self.output);
			};
			if message["type"] == "event" && message["event"] != "output" {
				self.events.push_back(message);
			}
		}
	}

	/// How many times `text` has been passed on
	pub fn count(&self, text: &str) -> usize {
		self.output.matches(text).count()
	}

	/// Says goodbye and waits for the launcher to exit
	pub fn disconnect(mut self, arguments: Value) {
		self.try_request("disconnect", arguments);
		let deadline = Instant::now() + Duration::from_secs(20);
		loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				assert!(status.success(), "the launcher exited with {}", status);
				return;
			}
			assert!(Instant::now() < deadline, "timed out waiting for launcher exit");
			std::thread::sleep(Duration::from_millis(100));
		}
	}
}

/// Variables as names and values
pub fn named(vars: &[Value]) -> Vec<(String, String)> {
	vars.iter()
		.map(|v| (v["name"].as_str().unwrap().to_owned(), v["value"].as_str().unwrap().to_owned()))
		.collect()
}

/// The value under `name`
pub fn value_of<'a>(vars: &'a [(String, String)], name: &str) -> &'a str {
	vars.iter()
		.find(|(n, _)| n == name)
		.map(|(_, value)| value.as_str())
		.unwrap_or_else(|| panic!("no {} in {:?}", name, vars))
}
