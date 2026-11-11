//! The world the groups debug (`tests/dap_test_host`), launched by a launcher,
//! and its scenarios: the test sets the world's `scenario` var through the
//! debugger, and the world's loop runs `/proc/sc_NAME`.

use std::{
	collections::HashMap,
	ops::{Deref, DerefMut},
	path::PathBuf
};

use serde_json::{json, Value};

use crate::{client::Launcher, paths};

pub fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/dap_test_host").join(name)
}

/// The world's code: where the lines the tests look for are. Each has a
/// `// @name` comment, and no two files share a name.
#[derive(Clone)]
pub struct Marks {
	/// The directory the code is in
	pub dir: PathBuf
}

impl Marks {
	/// The code's files, those in its directories after its own
	fn files(&self) -> Vec<PathBuf> {
		fn files_in(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
			let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
			entries.sort();
			files.extend(
				entries
					.iter()
					.filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "dm"))
					.cloned()
			);
			for directory in entries.iter().filter(|path| path.is_dir()) {
				files_in(directory, files);
			}
		}
		let mut files = vec![];
		files_in(&self.dir, &mut files);
		files
	}

	/// The file and line marked `// @name`
	pub fn place(&self, name: &str) -> (PathBuf, u64) {
		let marker = format!("// @{}", name);
		for file in self.files() {
			let source = std::fs::read_to_string(&file).unwrap();
			if let Some(at) = source.lines().position(|line| line.trim_end().ends_with(&marker)) {
				return (file, at as u64 + 1);
			}
		}
		panic!("no line marked {} in {}", marker, self.dir.display())
	}

	/// The line marked `// @name`
	pub fn line(&self, name: &str) -> u64 {
		self.place(name).1
	}

	pub fn file(&self, name: &str) -> PathBuf {
		let path = self.dir.join(name);
		assert!(path.is_file(), "there's no {}", path.display());
		path
	}

	/// The line of `file` that starts with `text`
	pub fn line_starting(&self, file: &str, text: &str) -> u64 {
		let source = std::fs::read_to_string(self.file(file)).unwrap();
		source
			.lines()
			.position(|line| line.starts_with(text))
			.unwrap_or_else(|| panic!("no line starting {:?} in {}", text, file)) as u64
			+ 1
	}

	/// Writes `file` again with `old` replaced by `new`, once
	pub fn edit(&self, file: &str, old: &str, new: &str) {
		let path = self.file(file);
		let source = std::fs::read_to_string(&path).unwrap();
		assert!(source.contains(old), "{} has no {:?}", file, old);
		std::fs::write(&path, source.replacen(old, new, 1)).unwrap();
	}
}

pub struct World {
	pub launcher: Launcher,
	pub marks: Marks,
	/// The files `break_on` last set breakpoints in
	with_breakpoints: Vec<PathBuf>,
	/// How many times each scenario had finished when it was last run
	finished: HashMap<String, usize>
}

impl Deref for World {
	type Target = Launcher;

	fn deref(&self) -> &Launcher {
		&self.launcher
	}
}

impl DerefMut for World {
	fn deref_mut(&mut self) -> &mut Launcher {
		&mut self.launcher
	}
}

impl World {
	/// Launches the world in `dir` (the host's own directory unless it's a
	/// copy). It waits for `configure`, as a client's breakpoints go in first.
	pub fn launch_in(dir: PathBuf, more: Value) -> Self {
		Self::launch_as(json!({}), dir, more)
	}

	/// The same, for a client with `abilities` of its own
	pub fn launch_as(abilities: Value, dir: PathBuf, more: Value) -> Self {
		let mut launcher = Launcher::start();
		launcher.initialize_as(abilities);
		launcher.launch(&paths::find_byond(), &dir.join("dap_test_host.dme"), more);
		Self {
			launcher,
			marks: Marks { dir },
			with_breakpoints: vec![],
			finished: HashMap::new()
		}
	}

	/// Attaches another launcher to the world in process `pid`, whose code is
	/// in `dir`
	pub fn attach_in(dir: PathBuf, pid: u64) -> Self {
		let mut launcher = Launcher::start();
		launcher.initialize();
		launcher.attach(pid, json!({}));
		Self {
			launcher,
			marks: Marks { dir },
			with_breakpoints: vec![],
			finished: HashMap::new()
		}
	}

	pub fn attach(pid: u64) -> Self {
		Self::attach_in(host_file(""), pid)
	}

	pub fn launch(more: Value) -> Self {
		Self::launch_in(host_file(""), more)
	}

	/// Lets the world run, and waits for its code to
	pub fn configure(&mut self) {
		self.request("configurationDone", json!({}));
		self.wait_output("dap: world/New");
	}

	/// A world that's running, which doesn't pause on runtime errors
	pub fn start() -> Self {
		let mut world = Self::launch(json!({}));
		world.catch_runtimes(false);
		world.configure();
		world
	}

	/// Makes the marked lines the world's line breakpoints, all of them: the
	/// ones set before go
	pub fn break_on(&mut self, names: &[&str]) {
		let marked: Vec<(&str, &str)> = names.iter().map(|name| (*name, "")).collect();
		self.break_if(&marked);
	}

	/// The same, each with a condition (or `""` for none). They all have to
	/// be taken.
	pub fn break_if(&mut self, marked: &[(&str, &str)]) {
		for (breakpoint, (name, _)) in self.try_break_if(marked).iter().zip(marked) {
			let mut breakpoint = breakpoint.clone();
			// One given in the moment a reload lands waits for the launcher to
			// know the world has the file's change. A client is told when
			// it's set, which is at once.
			if breakpoint["message"] == "pending hot reload" {
				let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
				while breakpoint["verified"] != true && std::time::Instant::now() < deadline {
					match self.launcher.event_within("breakpoint", std::time::Duration::from_secs(1)) {
						Some(event) if event["body"]["breakpoint"]["id"] == breakpoint["id"] => breakpoint = event["body"]["breakpoint"].clone(),
						_ => {}
					}
				}
			}
			assert_eq!(breakpoint["verified"], true, "the breakpoint at {} wasn't set: {}", name, breakpoint);
		}
	}

	/// The same, for the breakpoints as the launcher answered, in the order
	/// given
	pub fn try_break_if(&mut self, marked: &[(&str, &str)]) -> Vec<Value> {
		let mut by_file: Vec<(PathBuf, Vec<(usize, Value)>)> = self.with_breakpoints.drain(..).map(|file| (file, vec![])).collect();
		for (i, (name, condition)) in marked.iter().enumerate() {
			let (file, line) = self.marks.place(name);
			let breakpoint = if condition.is_empty() {
				json!({ "line": line })
			} else {
				json!({ "line": line, "condition": condition })
			};
			match by_file.iter_mut().find(|(f, _)| *f == file) {
				Some((_, breakpoints)) => breakpoints.push((i, breakpoint)),
				None => by_file.push((file, vec![(i, breakpoint)]))
			}
		}
		let mut answered = vec![Value::Null; marked.len()];
		for (file, breakpoints) in by_file {
			let wanted: Vec<Value> = breakpoints.iter().map(|(_, b)| b.clone()).collect();
			let set = self.launcher.set_breakpoints(&file, Value::Array(wanted));
			for ((i, _), breakpoint) in breakpoints.iter().zip(set) {
				answered[*i] = breakpoint;
			}
			if !breakpoints.is_empty() {
				self.with_breakpoints.push(file);
			}
		}
		answered
	}

	/// Waits for the world to stop for `reason` in `proc`, on the marked line
	pub fn stop_on(&mut self, reason: &str, proc: &str, name: &str) {
		let line = self.marks.line(name);
		self.launcher.stop_at(reason, proc, line);
	}

	/// A step that has to stop in `proc` on the marked line
	pub fn step_on(&mut self, command: &str, proc: &str, name: &str) {
		let line = self.marks.line(name);
		self.launcher.step_to(command, proc, line);
	}

	/// Runs a scenario to its breakpoint on the marked line, the only one,
	/// which is then taken away
	pub fn run_to(&mut self, scenario: &str, proc: &str, name: &str) {
		self.break_on(&[name]);
		self.run(scenario);
		self.stop_on("breakpoint", proc, name);
		self.break_on(&[]);
	}

	/// An argument of the paused proc
	pub fn argument(&mut self, name: &str) -> String {
		self.launcher.variable("Arguments", name)["value"].as_str().unwrap().to_owned()
	}

	/// Has the world run a scenario
	pub fn run(&mut self, scenario: &str) {
		let finished = self.count(&format!("dap: finished {}\n", scenario));
		self.finished.insert(scenario.to_owned(), finished);
		let set = self.eval_global(&format!("scenario = \"{}\"", scenario));
		assert_eq!(set, format!("\"{}\"", scenario), "couldn't ask for the scenario");
	}

	/// Waits for the scenario last run to be over
	pub fn finish(&mut self, scenario: &str) {
		let finished = self.finished[scenario];
		self.wait_count(&format!("dap: finished {}\n", scenario), finished + 1);
	}

	/// Runs a scenario that has to finish without the world stopping
	pub fn run_through(&mut self, scenario: &str) {
		self.run(scenario);
		self.finish(scenario);
		assert!(!self.has_event("stopped"), "the world stopped during {}", scenario);
	}

	/// Runs a scenario to its end, letting the world carry on from each stop
	/// on the way, and returns what `at_stop` made of each
	pub fn run_collecting<T>(&mut self, scenario: &str, mut at_stop: impl FnMut(&mut World) -> T) -> Vec<T> {
		self.run(scenario);
		let finished = self.finished[scenario];
		let over = format!("dap: finished {}\n", scenario);
		let deadline = std::time::Instant::now() + crate::client::TIMEOUT;
		let mut seen = vec![];
		loop {
			// What the launcher has sent comes in order: a stop before the
			// end is seen before it
			if self.launcher.event_within("stopped", std::time::Duration::from_millis(20)).is_some() {
				seen.push(at_stop(self));
				self.resume();
			} else if self.count(&over) > finished {
				return seen;
			}
			assert!(std::time::Instant::now() < deadline, "timed out waiting for {}", scenario);
		}
	}

	/// Lets the world carry on from a stop, to the end of the scenario
	pub fn resume_through(&mut self, scenario: &str) {
		self.resume();
		self.finish(scenario);
		assert!(!self.has_event("stopped"), "the world stopped again during {}", scenario);
	}

	/// Ends the world and the launcher
	pub fn end(self) {
		self.launcher.disconnect(json!({}));
	}
}
