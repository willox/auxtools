//! The world's record of what happened in it, read from the command line:
//! `log`, and the waits that come from it

use std::{
	io::{BufRead, BufReader, Read},
	path::Path,
	process::{Child, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use crate::client::{launcher_command, launcher_path};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs a command against the world in process `pid`, and checks its exit code
pub fn run(pid: u64, args: &[&str], code: i32) -> String {
	let pid = pid.to_string();
	let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
	let (got, printed) = launcher_command(&args);
	assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
	printed
}

/// A command left running, whose lines are read as they come
pub struct Background {
	child: Child,
	lines: mpsc::Receiver<String>,
	/// What it printed, so far
	pub printed: String
}

impl Background {
	pub fn start(pid: u64, args: &[&str]) -> Self {
		let mut child = Command::new(launcher_path())
			.arg("--pid")
			.arg(pid.to_string())
			.args(args)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let (sender, lines) = mpsc::channel();
		let stdout = child.stdout.take().unwrap();
		let errors = sender.clone();
		std::thread::spawn(move || {
			for line in BufReader::new(stdout).lines().map_while(Result::ok) {
				let _ = sender.send(line);
			}
		});
		let mut stderr = child.stderr.take().unwrap();
		std::thread::spawn(move || {
			let mut text = String::new();
			let _ = stderr.read_to_string(&mut text);
			for line in text.lines() {
				let _ = errors.send(line.to_owned());
			}
		});
		Self {
			child,
			lines,
			printed: String::new()
		}
	}

	/// Waits for a line that has `text` in it
	pub fn line_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let line = self
				.lines
				.recv_timeout(deadline.saturating_duration_since(Instant::now()))
				.unwrap_or_else(|_| panic!("no line with {:?} came; it printed {}", text, self.printed));
			self.printed += &line;
			self.printed += "\n";
			if line.contains(text) {
				return line;
			}
		}
	}

	/// Waits for it to exit: its exit code and everything it printed
	pub fn finish(mut self) -> (i32, String) {
		let deadline = Instant::now() + TIMEOUT;
		let code = loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				break status.code().unwrap_or(-1);
			}
			assert!(Instant::now() < deadline, "timed out waiting for process exit; output: {}", self.printed);
			std::thread::sleep(Duration::from_millis(50));
		};
		// What's left to read is read by now, or soon
		while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(2)) {
			self.printed += &line;
			self.printed += "\n";
		}
		(code, self.printed)
	}

	pub fn stop(mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// The number of the newest entry in the record
pub fn newest(pid: u64) -> u64 {
	let printed = run(pid, &["log", "--last", "1", "--json"], 0);
	printed.lines().last().map_or(0, |line| {
		serde_json::from_str::<serde_json::Value>(line).unwrap()["number"].as_u64().unwrap()
	})
}

/// Runtime errors go in the record, with their stacks, while the world runs
/// on: `log --runtimes` prints them, and `--follow` as they come
pub fn runtimes(pid: u64, dir: &Path) {
	let before = newest(pid);
	let since = before.to_string();
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let deadline = Instant::now() + TIMEOUT;
	let printed = loop {
		let printed = run(pid, &["log", "--runtimes", "--since", &since], 0);
		if !printed.is_empty() {
			break printed;
		}
		assert!(Instant::now() < deadline, "runtime error missing from journal");
		std::thread::sleep(Duration::from_millis(200));
	};
	let mut lines = printed.lines();
	let first = lines.next().unwrap();
	assert!(
		first.starts_with('#') && first.ends_with("runtime error: cannot read from list"),
		"log --runtimes said {}",
		printed
	);
	assert!(
		lines.next().is_some_and(|line| line.starts_with("  /fail  attach_host.dm:")),
		"log --runtimes said {}",
		printed
	);
	assert_eq!(run(pid, &["status"], 0).lines().next(), Some(format!("pid {}: running", pid).as_str()));
	println!("ok: runtime journal and stack");

	let printed = run(pid, &["log", "--since", &since, "--json"], 0);
	let entry: serde_json::Value = serde_json::from_str(printed.lines().find(|line| line.contains("\"runtime\"")).unwrap()).unwrap();
	assert_eq!(
		(&entry["kind"], &entry["caught"], &entry["proc"]),
		(&"runtime".into(), &false.into(), &"/fail".into()),
		"{}",
		entry
	);
	assert!(
		entry["number"].as_u64().unwrap() > before && entry["line"].as_u64().is_some(),
		"{}",
		entry
	);
	println!("ok: journal JSON Lines");

	// It starts with what's there already, after --since
	let mut follow = Background::start(pid, &["log", "--runtimes", "--follow", "--json", "--since", &before.to_string()]);
	follow.line_with("cannot read from list");
	let now = newest(pid);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let line = follow.line_with("cannot read from list");
	let entry: serde_json::Value = serde_json::from_str(&line).unwrap();
	assert!(
		entry["number"].as_u64().unwrap() > now && entry["stack"].is_array(),
		"log --follow printed {}",
		follow.printed
	);
	follow.stop();
	println!("ok: follow runtime journal");
}

/// Hot reloads go in the record, applied or not, reported by the launcher
/// that reloads the world: `wait reload` waits for one, and says whether
/// it was applied in its exit code; an attached launcher shows them
pub fn reloads(pid: u64, code: &Path) {
	let source = std::fs::read_to_string(code).unwrap();
	assert!(source.contains("\treturn 2\n"), "unexpected value() source after test setup");
	let printed = run(pid, &["wait", "reload", "--timeout", "1"], 2);
	assert_eq!(printed.trim(), "error: timed out after 1 second waiting for a hot reload attempt.");

	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("\treturn 2\n", "\treturn 3\n")).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "60"], 0);
	assert!(
		printed.starts_with('#') && printed.contains(" hot reload applied: reloaded /value"),
		"unexpected wait reload output: {}",
		printed
	);
	println!("ok: successful reload result");

	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("\treturn 2\n", "\treturn nothing_called_this\n")).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "60", "--json"], 1);
	let entry: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap()).unwrap();
	assert_eq!((&entry["kind"], &entry["applied"]), (&"reload".into(), &false.into()), "{}", printed);
	let errors = entry["errors"].to_string();
	assert!(
		errors.contains("/value") && errors.contains("nothing_called_this"),
		"the errors were {}",
		errors
	);
	assert!(
		printed.ends_with("\nerror: the hot reload was rejected.\n"),
		"unexpected wait reload output: {}",
		printed
	);
	println!("ok: rejected reload diagnostics ({})", errors);

	// One with an error the launcher finds before the world is sent anything
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("\treturn 2\n", "\treturn ]\n")).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "60"], 1);
	let error = printed
		.lines()
		.find(|line| line.starts_with("  error: "))
		.unwrap_or_else(|| panic!("unexpected wait reload output: {}", printed));
	let place = error.trim_start_matches("  error: ").split(": ").next().unwrap();
	assert!(
		place
			.rsplit_once(':')
			.is_some_and(|(file, line)| file.ends_with("attach_host.dm") && line.parse::<u32>().is_ok()),
		"the error isn't at a line of the file: {}",
		printed
	);
	println!(
		"ok: wait reload says a hot reload with a compile error wasn't applied, and where the error is ({})",
		error.trim()
	);

	// One attached to the world, which doesn't reload it, hears of it
	let mut attached = crate::client::Launcher::start();
	attached.initialize();
	attached.request("attach", serde_json::json!({ "processId": pid, "hotReload": false }));
	attached.event("initialized");
	attached.request("configurationDone", serde_json::json!({}));

	// The next one, waited for as it comes
	let waiting = Background::start(pid, &["wait", "reload", "--timeout", "60"]);
	std::thread::sleep(Duration::from_secs(1));
	std::fs::write(code, &source).unwrap();
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.contains("hot reload applied: reloaded /value"),
		"wait reload gave {}: {}",
		exit,
		printed
	);
	attached.wait_output("Hot reload (by another launcher): reloaded /value");
	attached.disconnect(serde_json::json!({}));
	println!("ok: attached launcher reload reports");

	let printed = run(pid, &["log", "--reloads", "--last", "4"], 0);
	let headers: Vec<&str> = printed.lines().filter(|line| line.starts_with('#')).collect();
	assert!(
		headers.len() == 4
			&& headers[0].contains("applied: reloaded /value")
			&& headers[1].contains("not applied")
			&& headers[2].contains("not applied")
			&& headers[3].contains("applied: reloaded /value"),
		"log --reloads said {}",
		printed
	);
	assert!(
		printed.contains("\n  error: ") && printed.contains("nothing_called_this"),
		"log --reloads said {}",
		printed
	);
	println!("ok: reload history and errors");

	let printed = run(pid, &["log", "--output", "--last", "5"], 0);
	assert!(
		printed.contains(" output\n  attach test: "),
		"unexpected log --output output: {}",
		printed
	);
	println!("ok: world output journal");

	let printed = run(pid, &["status"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	assert!(
		lines.len() == 3
			&& lines[0] == format!("pid {}: running", pid)
			&& lines[1].starts_with("uptime: ")
			&& lines[1].contains(" UTC), world.time ")
			&& lines[1].ends_with(", world loads: 1")
			&& lines[2].starts_with("last reload: #")
			&& lines[2].ends_with(" hot reload applied: reloaded /value"),
		"status said {}",
		printed
	);
	println!("ok: uptime and latest reload status");

	// A wait must recognize its breakpoint after reload moves it within the
	// proc.
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\t\ttotal += i // first loop"));
	let waiting = Background::start(pid, &["wait", "pause", "--break", &place, "--timeout", "60"]);
	std::thread::sleep(Duration::from_secs(1));
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", "/proc/two_loops()\n\tvar/moved = 1\n")).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "60"], 0);
	run(pid, &["eval", "--spawn", "two_loops()"], 0);
	let (exit, printed) = waiting.finish();
	assert_eq!(exit, 0, "wait pause after a reload gave: {}", printed);
	run(pid, &["continue"], 0);
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "60"], 0);
	println!("ok: wait pause keeps its breakpoint through a reload");
}

/// A reboot goes in the record, whose numbers go on in the new world:
/// `wait reboot` waits for it, and for the new world to answer
pub fn reboot(pid: u64, dir: &Path) {
	let before = newest(pid);
	let waiting = Background::start(pid, &["wait", "reboot", "--timeout", "60"]);
	std::thread::sleep(Duration::from_secs(1));
	std::fs::write(dir.join("reboot"), "yes").unwrap();
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.contains(" world rebooted (load 2 in this process)"),
		"wait reboot gave {}: {}",
		exit,
		printed
	);
	let number: u64 = printed.trim_start_matches('#').split(' ').next().unwrap().parse().unwrap();
	assert!(number > before, "the reboot is entry {}, before {}", number, before);
	let printed = run(pid, &["status"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: running\nuptime: ", pid)) && printed.contains(", world loads: 2\n"),
		"status said {}",
		printed
	);
	// The load that's in the record already is one to wait for, given where to
	// look from
	let printed = run(pid, &["wait", "reboot", "--since", &before.to_string(), "--timeout", "5", "--json"], 0);
	let entry: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected wait reboot --json output: {}", printed));
	assert!(
		entry["kind"] == "world" && entry["event"] == "loaded" && entry["load"] == 2 && entry["number"] == number,
		"{}",
		entry
	);
	println!("ok: wait for reboot readiness");

	let printed = run(pid, &["log", "--world", "--json"], 0);
	let loads: Vec<serde_json::Value> = printed
		.lines()
		.map(|line| serde_json::from_str(line).unwrap())
		.filter(|entry: &serde_json::Value| entry["event"] == "loaded")
		.collect();
	assert!(
		loads.len() == 2
			&& loads[0]["load"] == 1
			&& loads[1]["load"] == 2
			&& loads[1]["number"] == number
			&& loads[0]["number"].as_u64() < Some(number),
		"log --world said {}",
		printed
	);
	// The launcher gives the new world the changes made since it was built
	let printed = run(pid, &["wait", "reload", "--since", &number.to_string(), "--timeout", "60"], 0);
	assert!(printed.contains("hot reload applied: "), "unexpected wait reload output: {}", printed);
	println!("ok: journal continuity across reboot");
}

/// A `wait reboot` for a world that's about to end, not reboot
pub fn wait_for_reboot(pid: u64) -> Background {
	let waiting = Background::start(pid, &["wait", "reboot", "--timeout", "60"]);
	std::thread::sleep(Duration::from_secs(1));
	waiting
}

/// The world ended: the wait says it went away
pub fn world_went_away(waiting: Background) {
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 3 && printed.contains("error: world connection ended before the operation completed"),
		"wait reboot gave {}: {}",
		exit,
		printed
	);
	println!("ok: world exit during reboot wait");
}

/// Waits for the world to be running again, and says how long that took
fn until_running(pid: u64) -> Duration {
	let started = Instant::now();
	loop {
		if run(pid, &["status"], 0).starts_with(&format!("pid {}: running\n", pid)) {
			return started.elapsed();
		}
		assert!(started.elapsed() < TIMEOUT, "the world stayed paused");
		std::thread::sleep(Duration::from_millis(200));
	}
}

/// A held pause with --hold-max ends by itself when nobody debugging comes
/// back to it, and the record says so; AUXTOOLS_HOLD_MAX does the same
pub fn hold_max(pid: u64) {
	let printed = run(pid, &["pause", "--hold-max", "2"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused by request", pid)),
		"unexpected pause output: {}",
		printed
	);
	assert!(
		run(pid, &["status"], 0).starts_with(&format!("pid {}: paused", pid)),
		"the pause wasn't held"
	);
	let took = until_running(pid);
	assert!(took > Duration::from_secs(1), "the pause was let go after {:?}", took);
	let printed = run(pid, &["log", "--world", "--last", "1"], 0);
	assert!(
		printed.contains("retained pause resumed automatically: no debugger was connected for 2 seconds (--hold-max)"),
		"log --world said {}",
		printed
	);
	println!("ok: held-pause expiry journal");

	let out = Command::new(launcher_path())
		.args(["--pid", &pid.to_string(), "pause"])
		.env("AUXTOOLS_HOLD_MAX", "1")
		.output()
		.unwrap();
	assert!(out.status.success(), "unexpected pause output: {}", String::from_utf8_lossy(&out.stderr));
	until_running(pid);
	// Without it, a held pause lasts
	run(pid, &["pause"], 0);
	std::thread::sleep(Duration::from_secs(3));
	assert!(
		run(pid, &["status"], 0).starts_with(&format!("pid {}: paused", pid)),
		"a pause without --hold-max was let go"
	);
	run(pid, &["continue"], 0);
	println!("ok: environment pause limit and unlimited holds");
}
