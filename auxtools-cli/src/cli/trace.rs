//! Trace execution using logpoints. Print each hit or count hits by expression
//! value or caller, then summarize. Tracing does not pause the world.

use std::{
	collections::HashMap,
	io::Write,
	sync::atomic::{AtomicBool, Ordering}
};

use serde_json::{json, Value};

use super::*;

/// How often it looks to see whether it's been interrupted
const TICK: Duration = Duration::from_millis(100);

/// What hits are counted by
enum By {
	/// The value of an expression at each hit
	Value(String),
	/// The proc and line that called the one hit
	Caller
}

/// Why it stopped
#[derive(Clone, Copy)]
enum Ended {
	Time,
	Count,
	Interrupted,
	/// What it prints to was closed (`| head`)
	Closed,
	/// The world went away
	HungUp
}

impl Ended {
	const fn name(self) -> &'static str {
		match self {
			Self::Time => "time",
			Self::Count => "count",
			Self::Interrupted => "interrupted",
			Self::Closed => "closed",
			Self::HungUp => "world"
		}
	}
}

pub fn trace(world: &mut World, args: &args::Trace) -> Result<()> {
	// (an expression or a message can be given as @FILE, as elsewhere)
	let by = match args.by.as_deref() {
		Some("caller") => Some(By::Caller),
		Some(expression) => Some(By::Value(super::given_text(expression)?)),
		None => None
	};
	let log = args.log.as_deref().map(super::given_text).transpose()?;
	let condition = args.condition.as_deref().map(super::given_text).transpose()?;
	let (count, json) = (args.count, args.json);

	// Group by the evaluated log message when counting expression values.
	// Every hit also includes its location and caller.
	let message = match (&by, &log) {
		(Some(By::Value(expression)), _) => vec![LogPart::Expr(expression.clone())],
		(_, Some(message)) => LogPart::parse(message),
		_ => vec![]
	};
	let printed_message = log.is_some();

	// Validate targets while installing. Disconnecting removes any
	// breakpoints installed before a failure.
	let mut out = Out::new();
	let mut bound = vec![];
	for place in &args.locations {
		bound.extend(world.set_breakpoint(place, condition.clone(), Some(message.clone()))?);
	}
	for each in &bound {
		let line = if json {
			json!({
				"event": "bound",
				"place": each.place,
				"proc": each.at.proc.path,
				"override": each.at.proc.override_id,
				"file": each.file,
				"line": each.line
			})
			.to_string()
		} else {
			format!("tracing {}", each.spot())
		};
		if !out.line(&line) {
			return Ok(());
		}
	}

	interruptible();
	let until = args.duration.map(|time| Instant::now() + time);
	let mut hits: u64 = 0;
	let mut counted: HashMap<String, (u64, Value)> = HashMap::new();
	let ended = loop {
		if INTERRUPTED.load(Ordering::SeqCst) {
			break Ended::Interrupted;
		}
		if until.is_some_and(|until| Instant::now() >= until) {
			break Ended::Time;
		}
		let tick = Instant::now() + TICK;
		let next = match world.events.iter().position(|e| matches!(e, Response::Log { .. })) {
			Some(at) => Some(world.events.remove(at)),
			None => match world.next(Some(until.map_or(tick, |until| until.min(tick)))) {
				Ok(next) => next,
				Err(_) => break Ended::HungUp
			}
		};
		let Some(Response::Log {
			message,
			file,
			line,
			proc,
			caller
		}) = next
		else {
			continue;
		};
		hits += 1;
		let called_by = match &caller {
			Some(caller) => place(caller),
			None => NO_CALLER.to_owned()
		};
		match &by {
			Some(By::Value(_)) => counted.entry(message).or_insert((0, Value::Null)).0 += 1,
			Some(By::Caller) => {
				let described = caller.as_ref().map(|caller| {
					json!({
						"proc": caller.instruction.proc.path,
						"override": caller.instruction.proc.override_id,
						"file": caller.file,
						"line": caller.line
					})
				});
				counted.entry(called_by).or_insert((0, described.unwrap_or(Value::Null))).0 += 1;
			}
			None => {
				let printed = if json {
					let mut hit = json!({
						"event": "hit",
						"proc": proc.path,
						"override": proc.override_id,
						"file": file,
						"line": line,
						"caller": caller.as_ref().map(|caller| json!({
							"proc": caller.instruction.proc.path,
							"override": caller.instruction.proc.override_id,
							"file": caller.file,
							"line": caller.line
						}))
					});
					if printed_message {
						hit["message"] = json!(message);
					}
					hit.to_string()
				} else if printed_message {
					message
				} else {
					format!("{}, called by {}", spot(&proc, false, file.as_deref(), line), called_by)
				};
				if !out.line(&printed) {
					break Ended::Closed;
				}
			}
		}
		if count.is_some_and(|count| hits >= count) {
			break Ended::Count;
		}
	};

	// Remove explicitly to collect condition-error counts before disconnecting.
	let mut skipped = None;
	if !matches!(ended, Ended::HungUp) {
		let mut sum = 0;
		for each in &bound {
			match world.unset_breakpoint(each) {
				Ok(n) => sum += n,
				Err(_) => break
			}
		}
		skipped = Some(sum);
	}
	if matches!(ended, Ended::Closed) {
		return Ok(());
	}

	// Sort by descending count, then by value.
	let mut table: Vec<(String, u64, Value)> = counted.into_iter().map(|(value, (n, described))| (value, n, described)).collect();
	table.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
	if json {
		let mut summary = json!({
			"event": "summary",
			"hits": hits,
			"skipped": skipped,
			"ended": ended.name()
		});
		if let Some(by) = &by {
			summary["by"] = table
				.iter()
				.map(|(value, n, described)| match by {
					By::Value(_) => json!({ "value": value, "hits": n }),
					By::Caller => json!({ "value": value, "caller": described, "hits": n })
				})
				.collect();
		}
		out.line(&summary.to_string());
	} else {
		let width = table.iter().map(|(_, n, _)| n.to_string().len()).max().unwrap_or(1);
		for (value, n, _) in &table {
			if !out.line(&format!("{:>width$}  {}", n, value)) {
				return Ok(());
			}
		}
		let skipped = skipped.map(|n| format!(", {} skipped", n)).unwrap_or_default();
		out.line(&format!(
			"summary: {} {}{}; ended: {}",
			hits,
			if hits == 1 { "hit" } else { "hits" },
			skipped,
			ended.name()
		));
	}
	match ended {
		Ended::HungUp => Err(hung_up()),
		_ => Ok(())
	}
}

/// Label for hits without a caller.
const NO_CALLER: &str = "nothing (spawned, or called by the world)";

/// Lines to stdout, until it's closed
struct Out {
	open: bool
}

impl Out {
	const fn new() -> Self {
		Self { open: true }
	}

	/// Write a line; return false if the output pipe is closed.
	/// Unix handles this through SIGPIPE; Windows needs an explicit check.
	fn line(&mut self, text: &str) -> bool {
		if self.open {
			let mut stdout = io::stdout().lock();
			self.open = writeln!(stdout, "{}", text).and_then(|_| stdout.flush()).is_ok();
		}
		self.open
	}
}

/// Record interruption so tracing can print its summary before exiting.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn interruptible() {
	extern "C" fn interrupted(_: libc::c_int) {
		INTERRUPTED.store(true, Ordering::SeqCst);
	}
	unsafe {
		let handler = interrupted as extern "C" fn(libc::c_int) as libc::sighandler_t;
		libc::signal(libc::SIGINT, handler);
		libc::signal(libc::SIGTERM, handler);
	}
}

#[cfg(windows)]
fn interruptible() {
	// Not in the parts of winapi the launcher has
	#[link(name = "kernel32")]
	extern "system" {
		fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
	}
	// Ctrl-C and Ctrl-Break; a closed console ends the process anyway
	unsafe extern "system" fn interrupted(_: u32) -> i32 {
		INTERRUPTED.store(true, Ordering::SeqCst);
		1
	}
	unsafe {
		SetConsoleCtrlHandler(Some(interrupted), 1);
	}
}

#[cfg(not(any(unix, windows)))]
fn interruptible() {}
