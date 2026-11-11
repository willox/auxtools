//! Read and format debugger journal entries as text or JSON.

use std::{
	collections::VecDeque,
	io::Write,
	time::{Duration, Instant}
};

use auxtools_cli::{local, server_types::*};
use serde_json::json;

use super::{args, hung_up, seconds_text, unexpected, Failure, Result, World, HUNG_UP, TIMED_OUT};

/// `auxtools log`
pub fn log(world: &mut World, args: &args::Log) -> Result<()> {
	let kinds = kinds(args);
	let (entries, first, newest) = match world.request(Request::Journal {
		after: args.since.unwrap_or(0),
		kinds: kinds.clone(),
		limit: args.last,
		follow: false
	})? {
		Response::Journal { entries, first, last } => (entries, first, last),
		other => return Err(unexpected(&other))
	};
	if let Some(since) = args.since.filter(|since| first > since + 1) {
		eprintln!(
			"entries {} to {} are no longer retained: the journal keeps the most recent few thousand.",
			since + 1,
			first - 1
		);
	}
	for entry in &entries {
		if !print(entry, args.json) {
			return Ok(());
		}
	}
	if !args.follow {
		return Ok(());
	}

	// From where that left off, across reboots, until the world ends
	let mut follower = Follower::new(newest.max(args.since.unwrap_or(0)), kinds);
	loop {
		match follower.next(world, None) {
			Ok(Some(entry)) => {
				if !print(&entry, args.json) {
					return Ok(());
				}
			}
			Ok(None) => {}
			// The world exited, which is where following it ends
			Err(Failure(HUNG_UP, _)) => return Ok(()),
			Err(e) => return Err(e)
		}
	}
}

/// What a time limit is in an error, when there was one
fn limit(timeout: Option<Duration>) -> String {
	timeout.map(|timeout| format!(" after {}", seconds_text(timeout))).unwrap_or_default()
}

/// Wait for the next reload result, optionally after `since`.
/// Return exit code 1 if the reload was rejected.
pub fn wait_reload(world: &mut World, since: Option<u64>, timeout: Option<Duration>, json: bool) -> Result<()> {
	let until = timeout.map(|timeout| Instant::now() + timeout);
	let mut follower = Follower::new(since.unwrap_or(u64::MAX), vec![JournalKind::Reload]);
	let Some(entry) = follower.next(world, until)? else {
		return Err(Failure(
			TIMED_OUT,
			format!("timed out{} waiting for a hot reload attempt.", limit(timeout))
		));
	};
	print(&entry, json);
	match entry.event {
		JournalEvent::Reload { applied: true, .. } => Ok(()),
		_ => Err("the hot reload was rejected.".into())
	}
}

/// `wait reboot`: waits for the world to reboot (or for the first load after
/// entry `since`), and for the new world to answer
pub fn wait_reboot(world: &mut World, since: Option<u64>, timeout: Option<Duration>, json: bool) -> Result<()> {
	let until = timeout.map(|timeout| Instant::now() + timeout);
	let mut follower = Follower::new(since.unwrap_or(u64::MAX), vec![JournalKind::World]);
	loop {
		let Some(entry) = follower.next(world, until)? else {
			return Err(Failure(
				TIMED_OUT,
				format!("timed out{} waiting for the world to reboot.", limit(timeout))
			));
		};
		if let JournalEvent::Loaded { .. } = entry.event {
			// Wait for the launcher to release the newly loaded world before
			// sending commands.
			loop {
				match world.started(until)? {
					Some(true) => break,
					// An unlimited event wait still has response timeouts.
					None if until.is_none() => return Err(world.unresponsive("respond")),
					_ => {}
				}
				if until.is_some_and(|until| Instant::now() >= until) {
					return Err(Failure(
						TIMED_OUT,
						format!("timed out{} waiting for the rebooted world to start running.", limit(timeout))
					));
				}
				std::thread::sleep(Duration::from_millis(50));
			}
			print(&entry, json);
			return Ok(());
		}
	}
}

/// The kinds of entries the options ask for: all of them, if none are named
pub fn kinds(args: &args::Log) -> Vec<JournalKind> {
	[
		(args.runtimes, JournalKind::Runtime),
		(args.reloads, JournalKind::Reload),
		(args.output, JournalKind::Output),
		(args.world, JournalKind::World),
		(args.spawned, JournalKind::Spawned)
	]
	.into_iter()
	.filter(|(asked, _)| *asked)
	.map(|(_, kind)| kind)
	.collect()
}

/// Prints an entry. False once nothing reads what's printed (a closed pipe),
/// which ends the command quietly.
pub fn print(entry: &JournalEntry, json: bool) -> bool {
	let text = if json { to_json(entry).to_string() } else { block(entry) };
	let mut out = std::io::stdout().lock();
	writeln!(out, "{}", text).and_then(|()| out.flush()).is_ok()
}

/// Stream selected journal entries and reconnect after reboots,
/// resuming from the last entry received.
pub struct Follower {
	/// Last received entry number; `u64::MAX` starts at the current position.
	after: u64,
	kinds: Vec<JournalKind>,
	/// Entries read, and not handed out yet
	waiting: VecDeque<JournalEntry>,
	/// Whether this connection has asked for them
	asked: bool
}

impl Follower {
	pub const fn new(after: u64, kinds: Vec<JournalKind>) -> Self {
		Self {
			after,
			kinds,
			waiting: VecDeque::new(),
			asked: false
		}
	}

	/// Receive the next entry, or `None` at the deadline.
	/// Return `HUNG_UP` if the world exits.
	pub fn next(&mut self, world: &mut World, until: Option<Instant>) -> Result<Option<JournalEntry>> {
		loop {
			if let Some(entry) = self.waiting.pop_front() {
				self.after = entry.number;
				return Ok(Some(entry));
			}
			if !self.asked {
				let asked = Request::Journal {
					after: self.after,
					kinds: self.kinds.clone(),
					limit: None,
					follow: true
				};
				if local::send(&mut world.stream, &asked).is_err() {
					world.reconnect(until)?;
					continue;
				}
				self.asked = true;
			}
			match world.next(until) {
				Ok(None) => return Ok(None),
				Ok(Some(Response::Journal { entries, last, .. })) => {
					if self.after == u64::MAX {
						self.after = last;
					}
					self.waiting.extend(entries);
				}
				Ok(Some(Response::Journaled(entry))) if entry.number > self.after => self.waiting.push_back(entry),
				Ok(Some(_)) => {}
				Err(Failure(HUNG_UP, _)) => {
					self.asked = false;
					world.reconnect(until)?;
				}
				Err(e) => return Err(e)
			}
		}
	}
}

/// How often to try a world's socket while it reboots
const RECONNECT_INTERVAL: Duration = Duration::from_millis(100);

impl World {
	/// Reconnect after a reboot, or return `HUNG_UP` if the process exited.
	/// Connections opened during reboot wait for the new world.
	pub fn reconnect(&mut self, until: Option<Instant>) -> Result<()> {
		loop {
			if let Ok(world) = World::connect(self.pid) {
				// Preserve output options and timeout settings across
				// reconnects.
				self.stream = world.stream;
				self.received = world.received;
				self.events.clear();
				return Ok(());
			}
			if !local::alive(self.pid) {
				return Err(hung_up());
			}
			if until.is_some_and(|until| Instant::now() >= until) {
				return Err(Failure(
					TIMED_OUT,
					"timed out waiting for the world to respond after its connection ended.".to_owned()
				));
			}
			std::thread::sleep(RECONNECT_INTERVAL);
		}
	}
}

impl World {
	/// Whether the world has started executing rather than waiting for
	/// launcher configuration. Return `None` if either the response timeout
	/// or `until` expires.
	pub fn started(&mut self, until: Option<Instant>) -> Result<Option<bool>> {
		let patience = Instant::now() + self.patience;
		let until = until.map_or(patience, |until| until.min(patience));
		match self.request_by(Request::Uptime, until)? {
			Some(Response::Uptime { started, .. }) => Ok(Some(started)),
			Some(other) => Err(unexpected(&other)),
			None => Ok(None)
		}
	}
}

/// Seconds, as hours, minutes and seconds
pub fn duration(seconds: f64) -> String {
	let seconds = seconds as u64;
	match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
		(0, 0, s) => format!("{}s", s),
		(0, m, s) => format!("{}m {}s", m, s),
		(h, m, s) => format!("{}h {}m {}s", h, m, s)
	}
}

/// Format an entry number followed by its indented details.
pub fn block(entry: &JournalEntry) -> String {
	let (what, more) = describe(&entry.event);
	let mut text = format!(
		"#{} {} (world.time {}) {}",
		entry.number,
		clock(entry.time),
		number(entry.world_time),
		what
	);
	for line in more {
		text += "\n  ";
		text += &line;
	}
	text
}

/// What happened, in a line, and the lines that go under it
pub fn describe(event: &JournalEvent) -> (String, Vec<String>) {
	match event {
		JournalEvent::Runtime { message, caught, stack } => {
			let kind = if *caught { "caught runtime error" } else { "runtime error" };
			(format!("{}: {}", kind, message), stack.iter().map(frame).collect())
		}
		JournalEvent::Reload {
			applied,
			summary,
			errors,
			warnings
		} => (
			format!("hot reload {}: {}", if *applied { "applied" } else { "not applied" }, summary),
			errors
				.iter()
				.map(|e| format!("error: {}", e))
				.chain(warnings.iter().map(|w| format!("warning: {}", w)))
				.collect()
		),
		JournalEvent::Loaded { load: 1 } => ("world loaded".to_owned(), vec![]),
		JournalEvent::Loaded { load } => (format!("world rebooted (load {} in this process)", load), vec![]),
		JournalEvent::Output { lines } => ("output".to_owned(), lines.clone()),
		JournalEvent::HoldEnded { seconds } => (
			format!(
				"retained pause resumed automatically: no debugger was connected for {} seconds (--hold-max)",
				number(*seconds)
			),
			vec![]
		),
		JournalEvent::Spawned {
			spawn,
			expression,
			result: Ok(value)
		} => (format!("spawn {}: {} returned {}", spawn, expression, value.text), vec![]),
		JournalEvent::Spawned {
			spawn,
			expression,
			result: Err(error)
		} => {
			// Its first line, then the procs it was in
			let mut lines = error.lines().map(|line| line.trim_start().to_owned());
			(
				format!("spawn {}: {} failed: {}", spawn, expression, lines.next().unwrap_or_default()),
				lines.collect()
			)
		}
	}
}

fn frame(frame: &JournalFrame) -> String {
	match (&frame.file, frame.line) {
		(Some(file), Some(line)) => format!("{}  {}:{}", frame.proc, file, line),
		(None, Some(line)) => format!("{}  line {}", frame.proc, line),
		(Some(file), None) => format!("{}  {}", frame.proc, file),
		(None, None) => frame.proc.clone()
	}
}

/// A number without a fraction if it has none
pub fn number(n: f64) -> String {
	if n.fract() == 0.0 && n.abs() < 1e15 {
		format!("{}", n as i64)
	} else {
		format!("{:.1}", n)
	}
}

/// The time of day of a time in milliseconds since 1970, in UTC
pub fn clock(ms: u64) -> String {
	let of_day = ms % 86_400_000;
	format!(
		"{:02}:{:02}:{:02}.{:03} UTC",
		of_day / 3_600_000,
		of_day / 60_000 % 60,
		of_day / 1000 % 60,
		of_day % 1000
	)
}

/// Serialize an entry with number, Unix time in milliseconds, world time
/// in deciseconds, kind and kind-specific fields.
pub fn to_json(entry: &JournalEntry) -> serde_json::Value {
	let mut value = match &entry.event {
		JournalEvent::Runtime { message, caught, stack } => {
			let top = stack.first();
			json!({
				"kind": "runtime",
				"message": message,
				"caught": caught,
				"proc": top.map(|frame| &frame.proc),
				"file": top.and_then(|frame| frame.file.as_ref()),
				"line": top.and_then(|frame| frame.line),
				"stack": stack.iter().map(|frame| json!({ "proc": frame.proc, "file": frame.file, "line": frame.line })).collect::<Vec<_>>()
			})
		}
		JournalEvent::Reload {
			applied,
			summary,
			errors,
			warnings
		} => json!({ "kind": "reload", "applied": applied, "summary": summary, "errors": errors, "warnings": warnings }),
		JournalEvent::Loaded { load } => json!({ "kind": "world", "event": "loaded", "load": load }),
		JournalEvent::Output { lines } => json!({ "kind": "output", "lines": lines }),
		JournalEvent::HoldEnded { seconds } => json!({ "kind": "world", "event": "hold_ended", "seconds": seconds }),
		JournalEvent::Spawned { spawn, expression, result } => json!({
			"kind": "spawned",
			"spawn": spawn,
			"expression": expression,
			"value": result.as_ref().ok().map(|value| super::json::value(&value.rendered)),
			"text": result.as_ref().ok().map(|value| &value.text),
			"error": result.as_ref().err()
		})
	};
	value["number"] = entry.number.into();
	value["time"] = entry.time.into();
	value["world_time"] = entry.world_time.into();
	value
}

#[cfg(test)]
mod tests {
	use auxtools_cli::server_types::{JournalEntry, JournalEvent, JournalFrame};

	use super::{block, clock, duration, to_json};

	fn runtime() -> JournalEntry {
		JournalEntry {
			number: 7,
			time: 86_400_000 + 3_723_004,
			world_time: 45.0,
			event: JournalEvent::Runtime {
				message: "cannot read from list".to_owned(),
				caught: false,
				stack: vec![
					JournalFrame {
						proc: "/fail".to_owned(),
						file: Some("code.dm".to_owned()),
						line: Some(30)
					},
					JournalFrame {
						proc: "/run".to_owned(),
						file: None,
						line: None
					},
				]
			}
		}
	}

	#[test]
	fn the_time_is_of_the_day_in_utc() {
		assert_eq!(clock(86_400_000 + 3_723_004), "01:02:03.004 UTC");
	}

	#[test]
	fn a_duration_is_in_hours_minutes_and_seconds() {
		assert_eq!(
			(duration(5.9), duration(65.0), duration(3725.0)),
			("5s".to_owned(), "1m 5s".to_owned(), "1h 2m 5s".to_owned())
		);
	}

	#[test]
	fn a_runtime_is_a_line_and_its_stack() {
		assert_eq!(
			block(&runtime()),
			"#7 01:02:03.004 UTC (world.time 45) runtime error: cannot read from list\n  /fail  code.dm:30\n  /run"
		);
	}

	#[test]
	fn a_runtime_in_json_says_where_it_was() {
		let value = to_json(&runtime());
		assert_eq!(value["kind"], "runtime");
		assert_eq!(value["number"], 7);
		assert_eq!(
			(&value["proc"], &value["file"], &value["line"]),
			(&"/fail".into(), &"code.dm".into(), &30.into())
		);
		assert_eq!(value["stack"][1]["proc"], "/run");
	}

	#[test]
	fn a_reload_lists_its_errors_and_warnings() {
		let entry = JournalEntry {
			event: JournalEvent::Reload {
				applied: false,
				summary: "couldn't read the code".to_owned(),
				errors: vec!["a.dm, line 3: bad".to_owned()],
				warnings: vec!["careful".to_owned()]
			},
			..runtime()
		};
		assert!(block(&entry).ends_with("hot reload not applied: couldn't read the code\n  error: a.dm, line 3: bad\n  warning: careful"));
	}
}
