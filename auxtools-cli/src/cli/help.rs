//! Help pages assembled from prose and clap option definitions.
//! Each page includes usage, options, behavior, output, exit codes and examples
//! where applicable.

use clap::error::{ContextKind, ContextValue, ErrorKind};

use super::{args::Unparsed, Failure, Result};

/// A command's page
pub struct Page {
	/// The command, and its subcommand if it's one's: `["wait", "pause"]`
	pub path: &'static [&'static str],
	/// What it does, in the list of commands
	pub brief: &'static str,
	/// What it does, in a sentence
	pub summary: &'static str,
	/// Each form of its usage, a line each
	pub usage: &'static str,
	/// The `Arguments` section's lines, if it has arguments
	pub arguments: &'static str,
	/// The sections after the options, as they're printed
	pub sections: &'static str
}

/// The commands in the order and the groups the first page lists them in
const GROUPS: &[(&str, &[&str])] = &[
	("World management", &["launch", "list", "status", "stop", "reboot", "log"]),
	("Inspection", &["stack", "vars", "eval", "proc", "disassemble"]),
	("Execution", &["pause", "continue", "step-over", "step-in", "step-out", "wait", "trace"]),
	("Integration", &["dap", "help", "version"])
];

const STEP_SECTIONS: &str = "Behavior:
  Requires a paused world. Resume execution and wait for a pause or confirmed
  step completion. Report the resulting state and retain any resulting pause.
  A step can complete with the world running if no frame remains to pause in.
  An expired deadline stops waiting and returns exit code 2; it does not imply
  step completion or automatically pause or resume the world.

Output:
  Execution state and pause details, using the shared execution fields from
  status: pid, paused, reason, message, caught, changes, stack, and instruction.
  On timeout, include the last observed state when available; do not infer
  execution state from the absence of a pause event.

Exit codes:
  0  Execution paused or the step completed.
  1  Step request failed.
  2  Step deadline expired.
  3  World exited or rebooted before the operation completed.";

pub const PAGES: &[Page] = &[
	Page {
		path: &["launch"],
		brief: "Start a world with debugging and hot reload enabled",
		summary: "Start a compiled world with the debug library loaded.",
		usage: "auxtools launch DMB [OPTIONS] [-- ARGS...]",
		arguments: "DMB   Path to a compiled BYOND world.
ARGS  Arguments passed to the BYOND program. Replace the
      default -trusted and, for Windows DreamDaemon, -close.",
		sections: "Behavior:
  Foreground mode streams launcher and world output until the world exits.
  Interrupt once to request graceful shutdown; interrupt again to force exit.
  The launcher does not compile the DMB or pause execution. Other CLI commands
  and editors can attach to the world. Use reboot to load a rebuilt DMB.

Output:
  Detached mode reports the world's process ID, the launcher's process ID, and
  the launcher's log file. JSON contains pid, launcher, and log.

Exit codes:
  0  Startup completed in detached mode, or the foreground world exited with 0.
  1  Launch failed.
  2  Detached startup did not complete within 120 seconds; the world was ended.
  4  Foreground world exited with a nonzero code or was terminated."
	},
	Page {
		path: &["list"],
		brief: "List worlds with the debug library loaded",
		summary: "List local worlds with the debug library loaded.",
		usage: "auxtools list [--json]",
		arguments: "",
		sections: "Output:
  Text displays a table of worlds. JSON returns an array of world records.
  Both include process IDs, program and world information, protocol versions,
  and protocol compatibility. An empty JSON array means no worlds were found.
  Worlds with compatible=false require a matching CLI version.
  A world record contains pid, program, dmb, args, cwd, protocol, and compatible."
	},
	Page {
		path: &["status"],
		brief: "Show execution state, uptime, and hot reload status",
		summary: "Show execution state, uptime, and the latest hot reload result.",
		usage: "auxtools [--pid PID] status [--json]",
		arguments: "",
		sections: "Output:
  Text and JSON include execution state, pause details when available, uptime,
  world.time, and the latest hot reload result.
  JSON contains pid, paused, reason, message, caught, changes, stack,
  instruction, uptime_ms, world_time, loads, and last_reload.
  uptime_ms measures time since the latest world load; world_time is in
  deciseconds. loads counts the worlds this process has loaded: 1, plus one
  for each reboot. last_reload is a reload journal entry, or null if none exists.
  reason is \"breakpoint\", \"step\", \"pause\", \"runtime\", or \"change\" when paused.
  Runtime pauses populate message and caught; watch pauses populate changes
  with name, old, and new. Instruction steps include offset and text.
  Inapplicable scalar fields are null; changes and stack are empty arrays
  when unavailable. A running world has paused=false and no pause details."
	},
	Page {
		path: &["stop"],
		brief: "Request shutdown and wait for the process to exit",
		summary: "Request graceful shutdown and wait for the world process to exit.",
		usage: "auxtools [--pid PID] stop [--timeout SECONDS] [--kill [--kill-timeout SECONDS]] [--json]",
		arguments: "",
		sections: "Behavior:
  Calls world.Del(). A world started by launch also shuts down its launcher.
  world.Del() shuts down the debugger even if custom shutdown code leaves the
  process running. Include --kill in the initial request for a forced fallback.
  With --kill, reaching the graceful deadline starts forced termination.
  Success depends on the process exiting within the additional kill timeout.

Output:
  JSON contains pid, stopped, and killed. killed is true when the process was
  terminated after the graceful deadline.

Exit codes:
  0  Process exited.
  1  Shutdown or forced termination failed.
  2  Graceful shutdown timed out without --kill, or forced termination timed out."
	},
	Page {
		path: &["reboot"],
		brief: "Reboot a world and wait for debugger readiness",
		summary: "Reboot the world and wait for it to start running.",
		usage: "auxtools [--pid PID] reboot [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Calls world.Reboot() to reload the DMB. Existing debugger connections close.
  Worlds launched with hot reload wait for reloads to be restored before
  running any code. This command returns when the new world starts running.
  This command initiates a reboot; wait reboot only observes one.

Output:
  JSON contains pid and rebooted.

Exit codes:
  0  World rebooted and started running.
  1  Reboot request failed.
  2  Reboot timed out.
  3  World process exited instead of rebooting."
	},
	Page {
		path: &["log"],
		brief: "Read or follow world events",
		summary: "Read or follow the world's event journal.",
		usage: "auxtools [--pid PID] log [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  With no category options, include all categories. Multiple categories are
  combined. Entries are ordered oldest first; sequence numbers survive reboots.
  The journal retains only the most recent few thousand entries.
  --follow reconnects across reboots and ends when the world exits.
  Runtime logging does not require pausing the world.
  Captured output is available from dd and DreamDaemon launchers; DreamSeeker
  does not expose output for the launcher to capture.

Output:
  JSON entries contain number, time (Unix milliseconds), world_time
  (deciseconds), kind, and event-specific fields.
  kind is \"runtime\", \"reload\", \"output\", \"world\", or \"spawned\".

Exit codes:
  0  Entries were printed; with --follow, the world exited.
  1  Command failed."
	},
	Page {
		path: &["stack"],
		brief: "Show stack frames",
		summary: "Show stack frames, starting with the innermost frame.",
		usage: "auxtools [--pid PID] stack [--all] [--json]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.

Output:
  JSON is an array of frames: proc, file, line, offset, and old_code.
  With --all, JSON is an array of stacks: id, name, sleeping, and frames."
	},
	Page {
		path: &["vars"],
		brief: "Show frame arguments, local variables, and optional globals",
		summary: "Show arguments and local variables for a paused stack frame.",
		usage: "auxtools [--pid PID] vars [--frame N] [--globals] [--json [--depth N]]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.

Output:
  JSON contains arguments, locals, and optionally globals, using the same
  value encoding as eval. Each variable consumes one expansion level, so the
  default expands lists but does not expand object variables."
	},
	Page {
		path: &["eval"],
		brief: "Evaluate a DM expression",
		summary: "Evaluate a DM expression in the world or a paused stack frame.",
		usage: "auxtools [--pid PID] eval EXPRESSION [OPTIONS]
auxtools [--pid PID] eval --file FILE [OPTIONS]
auxtools [--pid PID] eval - [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  Use exactly one input source. '-' reads stdin.
  Put '--' before an expression that starts with '-', unless it is a number.
  In a paused world, expressions can access frame variables and src.
  In a running world, expressions can access globals and call procs.
  Several statements form a proc body: separate them with newlines or ';'.
  A proc body yields its return value, or the value of a final expression.
  Expressions can use the project's #define macros when the world was started
  by launch with hot reload enabled.
  Evaluation can change world state through assignments and proc calls.
  Evaluation errors exit with code 1; runtime diagnostics include locations
  and the call stack when available.
  Evaluated code cannot sleep unless --spawn is set. Spawned code runs outside
  any frame (--src can set src), pauses at breakpoints, and records its result
  in the journal (log --spawned). In a paused world it runs after execution
  resumes. --spawn requires a world started by launch.

Output:
  JSON represents numbers, text, and null directly. Lists use arrays, objects
  for text-keyed associations, or arrays of {key, value} pairs. Objects use
  {ref, type, name} with vars when expanded. Type paths use {path}; remaining
  values use {kind, text}. Non-finite numbers use {number: \"inf\"},
  {number: \"-inf\"}, or {number: \"nan\"}.
  Lists expand at depth >= 1; object variables expand at depth >= 2.
  Each nested value reduces the depth by one. Unexpanded lists use
  {list: true, len}; unexpanded objects omit vars.
  With --spawn and without --wait, the output is the spawn number: {spawn}.

Exit codes:
  0  Evaluation completed, or the code was spawned.
  1  Evaluation failed; with --wait, the spawned code ended in a runtime error.
  2  --wait deadline expired. The spawned code keeps running.
  3  World exited or rebooted before the spawned code finished (--wait)."
	},
	Page {
		path: &["proc"],
		brief: "Show a proc's source location, definitions, and reload status",
		summary: "Show a proc's source location, definition count, and hot reload status.",
		usage: "auxtools [--pid PID] proc /PATH[#N] [--json]",
		arguments: "/PATH  Proc path, with or without /proc/: /mob/proc/attack or /mob/attack.
#N     Zero-based definition index. [default: 0]",
		sections: "Output:
  JSON contains proc, file, line, definitions, and reloaded.
  reloaded is null, \"lines\", \"code\", or \"added\"."
	},
	Page {
		path: &["disassemble"],
		brief: "Show a proc's current bytecode",
		summary: "Disassemble a proc's current bytecode.",
		usage: "auxtools [--pid PID] disassemble /PATH[#N] [--json]",
		arguments: "/PATH  Proc path, with or without /proc/.
#N     Zero-based definition index. [default: 0]",
		sections: "Output:
  Instructions include bytecode offsets, instruction text, and source lines.
  After a hot reload, this shows the updated bytecode.
  JSON contains proc, file, and instructions; each instruction contains
  offset, text, and line."
	},
	Page {
		path: &["pause"],
		brief: "Pause execution and report the location",
		summary: "Pause execution and report the location.",
		usage: "auxtools [--pid PID] pause [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  The world remains paused after the command exits. Use continue to resume,
  or set --hold-max to limit the pause when no debugger is connected.
  If the world is already paused, retain that pause and report it.

Output:
  Execution state and pause details, using the shared execution fields from
  status: pid, paused, reason, message, caught, changes, stack, and instruction."
	},
	Page {
		path: &["continue"],
		brief: "Resume execution",
		summary: "Resume execution of a paused world.",
		usage: "auxtools [--pid PID] continue [--json]",
		arguments: "",
		sections: "Behavior:
  Requires a paused world.
  Return after the resume request completes; do not wait for another pause.

Output:
  JSON contains pid and resumed."
	},
	Page {
		path: &["step-over"],
		brief: "Advance to the next line, stepping over proc calls",
		summary: "Advance to the next source line, stepping over proc calls.",
		usage: "auxtools [--pid PID] step-over [--instruction] [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["step-in"],
		brief: "Advance to the next line, stepping into proc calls",
		summary: "Advance to the next source line, stepping into proc calls.",
		usage: "auxtools [--pid PID] step-in [--instruction] [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["step-out"],
		brief: "Run until the current proc returns",
		summary: "Run until the current proc returns to its caller.",
		usage: "auxtools [--pid PID] step-out [--timeout SECONDS] [--hold-max SECONDS] [--json]",
		arguments: "",
		sections: STEP_SECTIONS
	},
	Page {
		path: &["wait"],
		brief: "Wait for a pause, hot reload, or reboot",
		summary: "Wait for a debugger or world lifecycle event.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]
auxtools [--pid PID] wait reload [OPTIONS]
auxtools [--pid PID] wait reboot [OPTIONS]",
		arguments: "",
		sections: "Commands:
  pause   Wait for a pause, optionally installing breakpoints or watches.
  reload  Wait for a hot reload attempt and report its result.
  reboot  Wait for a world load and debugger readiness.

Specify pause, reload, or reboot. Bare `auxtools wait` shows this help.
Use `auxtools wait COMMAND --help` for options."
	},
	Page {
		path: &["wait", "pause"],
		brief: "Wait for a pause, optionally installing breakpoints or watches.",
		summary: "Wait for execution to pause and report the location and reason.",
		usage: "auxtools [--pid PID] wait pause [OPTIONS]",
		arguments: "",
		sections: "Behavior:
  With no triggers, observe pauses caused by other debugger clients.
  If already paused, return immediately unless --change is specified.
  With --change on a paused world, resolve watches in the selected frame,
  resume execution, and wait for a pause.
  Watches support object variables, list entries, globals, and proc variables.
  Breakpoints and watches belong to this invocation and are removed on exit.
  Invalid breakpoint locations fail before waiting. Resolved breakpoint
  locations are reported on stderr and included in JSON output.
  By default, the resulting pause persists after exit. --no-hold does not
  request a retained pause; an existing hold or another debugger can still
  keep the world paused.
  --if does not filter runtime errors or pauses caused by other clients.

Input:
  For EXPR and CONDITION, @FILE reads text from a file.
  Use @@ to represent a literal leading @.

Output:
  The shared execution fields from status plus already, breakpoints, and watching.
  already indicates that the world was paused before waiting.
  breakpoints lists requested and resolved locations.
  watching lists watch names and their initial values.

Exit codes:
  0  World paused, or was already paused.
  1  Setup or command failed.
  2  Wait timed out.
  3  World exited or rebooted before a pause was reported.

Examples:
  auxtools --pid 1234 wait pause --break code/combat.dm:42 --timeout 30 --json
  auxtools --pid 1234 wait pause --change src.health --if \"value < old\" --json
  auxtools --pid 1234 wait pause --runtimes --hold-max 60 --json"
	},
	Page {
		path: &["wait", "reload"],
		brief: "Wait for a hot reload attempt and report its result.",
		summary: "Wait for a hot reload attempt and report whether it was applied.",
		usage: "auxtools [--pid PID] wait reload [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Observe hot reload; do not trigger it or pause execution.
  Start waiting before saving source changes, or use --since with a previously
  recorded journal sequence number to include an attempt that already finished.
  Historical events are available only while retained in the journal.

Output:
  One reload journal entry, with the same fields as log --reloads.

Exit codes:
  0  Reload was applied.
  1  Reload was rejected or the command failed.
  2  Wait timed out.
  3  World exited before a reload result was received."
	},
	Page {
		path: &["wait", "reboot"],
		brief: "Wait for a world load and debugger readiness.",
		summary: "Wait for a world load and debugger readiness.",
		usage: "auxtools [--pid PID] wait reboot [--since N] [--timeout SECONDS] [--json]",
		arguments: "",
		sections: "Behavior:
  Observe a reboot; do not initiate it or pause execution.
  Reconnect after the reboot and return when the new world starts running,
  after its launcher has reapplied hot reload changes.
  With --since, a retained world-load event can satisfy the wait immediately.

Output:
  One world-load journal entry, with the same fields as log --world.

Exit codes:
  0  World-load event received from the responding debugger.
  1  Command failed.
  2  Wait timed out.
  3  World exited before the operation completed."
	},
	Page {
		path: &["trace"],
		brief: "Log or count breakpoint hits without pausing execution",
		summary: "Log or count breakpoint hits without pausing execution.",
		usage: "auxtools [--pid PID] trace LOCATION... [OPTIONS]",
		arguments: "LOCATION  FILE:LINE or /proc/path[#N]. Specify one or more locations.
          #N selects a zero-based definition. [default: 0]",
		sections: "Behavior:
  Default output identifies each hit location and caller.
  --by suppresses per-hit output and sorts counts from highest to lowest.
  --by and --log are mutually exclusive.
  If both --for and --count are set, stop when either limit is reached.
  Condition errors skip the hit, report a diagnostic once, and increment the
  skipped count. Grouping expression errors are counted under the error text.
  Trace breakpoints are removed when this command exits.

Input:
  For MESSAGE, CONDITION, and EXPR, @FILE reads text from a file.
  Use @@ to represent a literal leading @.

Exit codes:
  0  Duration or hit limit reached, or interrupted with Ctrl-C.
  1  Setup or command failed.
  3  World exited or rebooted."
	},
	Page {
		path: &["dap"],
		brief: "Serve the Debug Adapter Protocol over stdin and stdout",
		summary: "Serve the Debug Adapter Protocol over stdin and stdout.",
		usage: "auxtools dap",
		arguments: "",
		sections: "Behavior:
  Accept editor requests to launch or attach to worlds, debug execution,
  and hot reload source changes. Stdout is reserved for DAP messages."
	},
	Page {
		path: &["help"],
		brief: "Show command help",
		summary: "Show top-level or command-specific help.",
		usage: "auxtools help [COMMAND [SUBCOMMAND]]",
		arguments: "",
		sections: "Equivalent forms:
  auxtools --help
  auxtools COMMAND --help
  auxtools COMMAND SUBCOMMAND --help"
	},
	Page {
		path: &["version"],
		brief: "Show CLI and debug protocol versions",
		summary: "Show CLI and debug protocol versions.",
		usage: "auxtools version [--json]",
		arguments: "",
		sections: "Output:
  JSON contains version and protocol.

Aliases: --version, -V"
	}
];

/// Extra option rows for values that need their own explanation.
const MORE_ROWS: &[(&str, &str, &str, &str)] = &[("trace", "by", "--by caller", "Group hits by the calling proc and source line.")];

/// The first page: every command, a line each
pub fn overview() -> String {
	let mut text = String::from(
		"auxtools - Launch, inspect, and debug BYOND worlds

Usage:
  auxtools [--pid PID] COMMAND [ARGS] [OPTIONS]
  auxtools help [COMMAND [SUBCOMMAND]]
"
	);
	let width = GROUPS
		.iter()
		.flat_map(|(_, names)| names.iter())
		.map(|name| name.len())
		.max()
		.unwrap_or(0);
	for (group, names) in GROUPS {
		text += &format!("\n{}:\n", group);
		for name in *names {
			let brief = page(&[name]).map_or("", |page| page.brief);
			text += &format!("  {:width$}  {}\n", name, brief);
		}
	}
	text += "
World selection:
  --pid PID    Select a world process. Required when selection is ambiguous.
               Without --pid, select the only compatible world.
               Use `auxtools list` to find process IDs.

Output:
  --json       Request structured output where supported.
               log and trace emit JSON Lines; other commands emit one JSON value.
               Default: text. See command help for details.

Pause lifetime:
  pause, step-over, step-in, step-out, and wait pause retain the resulting pause.
  Use continue to resume. --hold-max SECONDS limits a retained pause when no
  debugger is connected. Default: AUXTOOLS_HOLD_MAX, or no limit if unset.

Help:
  auxtools COMMAND --help
  auxtools help COMMAND

Exit codes:
  0  Command completed successfully
  1  Command failed
  2  Operation timed out
  3  World connection ended before the operation completed
  4  Foreground launch: world exited unsuccessfully or was terminated

Commands that follow lifecycle events can reconnect across reboots.
";
	text
}

/// The page of the command at `path`
pub fn page(path: &[&str]) -> Option<&'static Page> {
	PAGES.iter().find(|page| page.path == path)
}

/// Lines, each two spaces in
fn indented(lines: &str) -> String {
	lines.lines().map(|line| format!("  {}\n", line)).collect()
}

/// An option as its row names it: `-f, --file FILE`
fn spec(arg: &clap::Arg) -> String {
	let mut names: Vec<String> = arg.get_short().iter().map(|short| format!("-{}", short)).collect();
	names.extend(arg.get_long().iter().map(|long| format!("--{}", long)));
	let mut text = names.join(", ");
	if arg.get_action().takes_values() {
		for name in arg.get_value_names().unwrap_or_default() {
			text += &format!(" {}", name);
		}
	}
	text
}

/// List command-specific options, then shared options.
/// Keep `--json` among command options when it has a custom description.
fn option_rows(command: &clap::Command, path: &[&str]) -> Vec<(String, String)> {
	const COMMON_JSON: &str = "Emit one JSON value on stdout.";
	let mut rows = vec![];
	let mut common_json = false;
	for arg in command.get_arguments() {
		let id = arg.get_id().as_str();
		if arg.is_positional() || arg.is_hide_set() || matches!(id, "help" | "pid") {
			continue;
		}
		let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
		if id == "json" && help == COMMON_JSON {
			common_json = true;
			continue;
		}
		rows.push((spec(arg), help));
		for (_, _, more, help) in MORE_ROWS.iter().filter(|(name, after, ..)| path == [*name] && *after == id) {
			rows.push(((*more).to_owned(), (*help).to_owned()));
		}
	}
	let selects_world = !matches!(path.first(), Some(&("launch" | "list" | "dap" | "help" | "version")));
	if selects_world {
		rows.push(("--pid PID".to_owned(), "Select the world process.".to_owned()));
	}
	if common_json {
		rows.push(("--json".to_owned(), COMMON_JSON.to_owned()));
	}
	rows.push(("-h, --help".to_owned(), "Show help for this command.".to_owned()));
	rows
}

/// A command's page, around the options clap has for it
pub fn render(page: &Page, command: &clap::Command) -> String {
	let mut text = format!("{}\n\nUsage:\n{}", page.summary, indented(page.usage));
	if !page.arguments.is_empty() {
		text += &format!("\nArguments:\n{}", indented(page.arguments));
	}
	// A command that only has commands of its own says what they are
	if !command.has_subcommands() {
		let rows = option_rows(command, page.path);
		let width = rows.iter().map(|(spec, _)| spec.len()).max().unwrap_or(0);
		text += "\nOptions:\n";
		for (spec, help) in rows {
			let mut lines = help.lines();
			text += &format!("  {:width$}  {}\n", spec, lines.next().unwrap_or_default());
			for line in lines {
				text += &format!("  {:width$}  {}\n", "", line);
			}
		}
	}
	if !page.sections.is_empty() {
		text += &format!("\n{}\n", page.sections);
	}
	text
}

/// Gives clap each command's page, for `--help` to show
pub fn attach(command: clap::Command) -> clap::Command {
	fn attached(mut command: clap::Command, path: &[&'static str]) -> clap::Command {
		let names: Vec<&'static str> = command
			.get_subcommands()
			.filter_map(|sub| {
				PAGES
					.iter()
					.flat_map(|page| page.path.iter())
					.find(|name| **name == sub.get_name())
					.copied()
			})
			.collect();
		for name in names {
			let mut below = path.to_vec();
			below.push(name);
			command = command.mut_subcommand(name, |sub| attached(sub, &below));
		}
		match page(path) {
			Some(page) => {
				let text = render(page, &command);
				command.about(page.brief).override_help(text)
			}
			None if path.is_empty() => command.override_help(overview()),
			None => command
		}
	}
	// Build first so inherited options are available to subcommands.
	let mut command = command;
	command.build();
	attached(command, &[])
}

/// `auxtools help [COMMAND [SUBCOMMAND]]`
pub fn show(path: &[String]) -> Result<()> {
	if path.is_empty() {
		print!("{}", overview());
		return Ok(());
	}
	let root = super::args::command();
	let mut command = &root;
	for (i, name) in path.iter().enumerate() {
		command = command
			.find_subcommand(name)
			.ok_or_else(|| Failure(1, unknown_command(&path[..i].iter().map(String::as_str).collect::<Vec<_>>(), name)))?;
	}
	let names: Vec<&str> = path.iter().map(String::as_str).collect();
	match page(&names) {
		Some(page) => print!("{}", render(page, command)),
		None => return Err(Failure(1, format!("no help page for '{}'.", path.join(" "))))
	}
	Ok(())
}

fn unknown_command(parents: &[&str], name: &str) -> String {
	match parents {
		[] => format!("unknown command '{}'. Run 'auxtools --help' for available commands.", name),
		_ => format!(
			"unknown {} command '{}'. Run 'auxtools {} --help' for available commands.",
			parents.join(" "),
			name,
			parents.join(" ")
		)
	}
}

/// Resolve the command path to select the relevant help page for an error.
fn command_path(args: &[String]) -> Vec<&'static str> {
	let mut path: Vec<&'static str> = vec![];
	let mut args = args.iter();
	while let Some(arg) = args.next() {
		if arg == "--pid" {
			args.next();
			continue;
		}
		if arg.starts_with('-') {
			continue;
		}
		let mut longer: Vec<&str> = path.clone();
		longer.push(arg);
		match page(&longer) {
			Some(page) => path = page.path.to_vec(),
			None => break
		}
		// (only `wait` has commands of its own)
		if PAGES.iter().all(|page| page.path.len() <= path.len() || !page.path.starts_with(&path)) {
			break;
		}
	}
	path
}

/// Convert clap output to requested help or a single-line argument error.
pub fn unparsed(error: &clap::Error, args: &[String]) -> Unparsed {
	if matches!(error.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand) {
		return Unparsed::Help(error.render().to_string());
	}
	let path = command_path(args);
	let text = |kind: ContextKind| match error.get(kind) {
		Some(ContextValue::String(text)) => Some(text.clone()),
		Some(ContextValue::Strings(texts)) => Some(texts.join(", ")),
		Some(other) => Some(other.to_string()),
		None => None
	};
	// An option as it's typed, without the name of its value
	let option = |spec: String| spec.split([' ', '=']).next().unwrap_or_default().to_owned();
	// The name of its value alone
	let value_name = |spec: &str| spec.split_once(' ').map(|(_, value)| value.trim_matches(['<', '>', '.']).to_owned());
	let usage = match path.as_slice() {
		[] => "Run 'auxtools --help' for usage.".to_owned(),
		_ => format!("Run 'auxtools {} --help' for usage.", path.join(" "))
	};
	let message = match error.kind() {
		ErrorKind::InvalidSubcommand => return Unparsed::Usage(unknown_command(&path, &text(ContextKind::InvalidSubcommand).unwrap_or_default())),
		ErrorKind::UnknownArgument => {
			let argument = text(ContextKind::InvalidArg).unwrap_or_default();
			match path.as_slice() {
				["wait"] => format!(
					"unexpected argument '{}' before a wait command: specify pause, reload, or reboot.",
					argument
				),
				_ => format!("unexpected argument '{}'.", argument)
			}
		}
		ErrorKind::InvalidValue | ErrorKind::ValueValidation | ErrorKind::NoEquals | ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
			let spec = text(ContextKind::InvalidArg).unwrap_or_default();
			let value = text(ContextKind::InvalidValue).unwrap_or_default();
			let why = std::error::Error::source(error).map(ToString::to_string);
			match (value.is_empty(), why) {
				(true, _) => match value_name(&spec) {
					Some(name) => format!("{} requires a value: {}.", option(spec), name),
					None => format!("{} requires a value.", option(spec))
				},
				(false, Some(why)) => format!("invalid value '{}' for {}: {}.", value, option(spec), why),
				(false, None) => format!("invalid value '{}' for {}.", value, option(spec))
			}
		}
		ErrorKind::MissingRequiredArgument => {
			let missing = text(ContextKind::InvalidArg).unwrap_or_default();
			format!("missing required argument: {}.", missing.replace(['<', '>'], "").replace("...", ""))
		}
		ErrorKind::ArgumentConflict => {
			let first = option(text(ContextKind::InvalidArg).unwrap_or_default());
			match text(ContextKind::PriorArg).map(option) {
				Some(second) if second != first => format!("{} and {} cannot be used together.", second, first),
				_ => format!("{} cannot be used more than once.", first)
			}
		}
		// Whatever else clap says, in its first line
		_ => {
			let rendered = error.render().to_string();
			let line = rendered.lines().next().unwrap_or_default();
			format!("{}.", line.trim_start_matches("error: ").trim_end_matches('.'))
		}
	};
	Unparsed::Usage(format!("{} {}", message, usage))
}

#[cfg(test)]
mod tests {
	use super::{command_path, overview, page, render, GROUPS, PAGES};

	/// Every command clap has, and those under it, by its path
	fn commands() -> Vec<(Vec<String>, clap::Command)> {
		fn under(command: &clap::Command, path: &[String], found: &mut Vec<(Vec<String>, clap::Command)>) {
			for sub in command.get_subcommands() {
				let mut path = path.to_vec();
				path.push(sub.get_name().to_owned());
				found.push((path.clone(), sub.clone()));
				under(sub, &path, found);
			}
		}
		let mut found = vec![];
		under(&crate::cli::args::command(), &[], &mut found);
		found
	}

	#[test]
	fn every_command_has_a_page_and_every_page_a_command() {
		let commands = commands();
		for (path, _) in &commands {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			assert!(page(&names).is_some(), "{} has no page", path.join(" "));
		}
		assert_eq!(commands.len(), PAGES.len(), "a page is of no command");
		for expected in [
			"launch",
			"list",
			"status",
			"stop",
			"reboot",
			"log",
			"stack",
			"vars",
			"eval",
			"proc",
			"disassemble",
			"pause",
			"continue",
			"step-over",
			"step-in",
			"step-out",
			"wait",
			"trace",
			"dap",
			"help",
			"version"
		] {
			assert!(page(&[expected]).is_some(), "{}", expected);
		}
		for gone in ["next", "step", "out", "dis", "--list", "run"] {
			assert!(page(&[gone]).is_none(), "{}", gone);
		}
	}

	#[test]
	fn the_first_page_lists_every_command_once() {
		let help = overview();
		let listed: Vec<&str> = GROUPS.iter().flat_map(|(_, names)| names.iter().copied()).collect();
		for page in PAGES.iter().filter(|page| page.path.len() == 1) {
			assert_eq!(listed.iter().filter(|name| **name == page.path[0]).count(), 1, "{}", page.path[0]);
			assert!(
				help.contains(&format!("  {:11}  {}\n", page.path[0], page.brief)),
				"{} isn't in the help",
				page.path[0]
			);
			assert!(!page.brief.ends_with('.'), "{}", page.brief);
		}
		assert_eq!(listed.len(), PAGES.iter().filter(|page| page.path.len() == 1).count());
	}

	#[test]
	fn a_page_starts_with_what_the_command_does_and_its_usage() {
		for (path, command) in commands() {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			let page = page(&names).unwrap();
			let text = render(page, &command);
			assert!(text.starts_with(&format!("{}\n\nUsage:\n  auxtools ", page.summary)), "{}", text);
			assert!(page.summary.ends_with('.'), "{}", page.summary);
			for usage in page.usage.lines() {
				assert!(usage.contains(&format!(" {}", path.join(" "))), "{}", usage);
				// Only a command to a world is given one
				let selects = !matches!(names[0], "launch" | "list" | "dap" | "help" | "version");
				assert_eq!(usage.starts_with("auxtools [--pid PID] "), selects, "{}", usage);
			}
			if command.has_subcommands() {
				assert!(!text.contains("\nOptions:\n"), "{}", text);
				continue;
			}
			assert!(text.contains("  -h, --help"), "{}", text);
			assert_eq!(
				text.contains("  --pid PID "),
				!matches!(names[0], "launch" | "list" | "dap" | "help" | "version"),
				"{}",
				text
			);
			assert!(!text.contains("--detached"), "{}", text);
		}
	}

	#[test]
	fn every_option_is_in_its_commands_page_and_says_what_it_does() {
		for (path, command) in commands() {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			let text = render(page(&names).unwrap(), &command);
			for arg in command.get_arguments().filter(|arg| !arg.is_positional() && !arg.is_hide_set()) {
				if arg.get_id() == "help" || (arg.get_id() == "pid" && !text.contains("--pid PID ")) {
					continue;
				}
				let long = arg.get_long().map(|long| format!("--{}", long));
				let short = arg.get_short().map(|short| format!("-{}", short));
				let name = long.or(short).unwrap();
				assert!(
					text.lines()
						.any(|line| line.trim_start().starts_with(&name) || line.contains(&format!(", {} ", name))),
					"{} {}",
					path.join(" "),
					name
				);
				let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
				// It starts with a verb, and ends as a sentence does (or with
				// its default)
				assert!(
					help.chars().next().is_some_and(char::is_uppercase),
					"{} {}: {:?}",
					path.join(" "),
					name,
					help
				);
				assert!(help.ends_with('.') || help.ends_with(']'), "{} {}: {:?}", path.join(" "), name, help);
			}
		}
	}

	#[test]
	fn no_line_of_the_help_is_wider_than_a_terminal() {
		let pages = commands().into_iter().map(|(path, command)| {
			let names: Vec<&str> = path.iter().map(String::as_str).collect();
			render(page(&names).unwrap(), &command)
		});
		for text in std::iter::once(overview()).chain(pages) {
			for line in text.lines() {
				assert!(line.chars().count() <= 100, "{} characters: {}", line.chars().count(), line);
				assert_eq!(line, line.trim_end(), "a line ends in spaces: {:?}", line);
			}
			assert!(
				text.ends_with('\n') && !text.ends_with("\n\n"),
				"{:?}",
				&text[text.len().saturating_sub(40)..]
			);
		}
	}

	#[test]
	fn the_command_an_error_is_about_is_found_among_the_arguments() {
		let args = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<String>>();
		assert_eq!(command_path(&args(&["--pid", "12", "wait", "pause", "--break", "x"])), ["wait", "pause"]);
		assert_eq!(command_path(&args(&["wait", "--break", "x"])), ["wait"]);
		assert_eq!(command_path(&args(&["eval", "wait", "pause"])), ["eval"]);
		assert_eq!(command_path(&args(&["frobnicate", "stop"])), Vec::<&str>::new());
		assert_eq!(command_path(&args(&["--json"])), Vec::<&str>::new());
	}

	#[test]
	fn options_are_listed_in_a_column() {
		let commands = commands();
		let (_, stop) = commands.iter().find(|(path, _)| path == &["stop".to_owned()]).unwrap();
		let text = render(page(&["stop"]).unwrap(), stop);
		assert!(
			text.contains(
				"\nOptions:
  --timeout SECONDS       Set the graceful shutdown deadline. [default: 10]
  --kill                  Terminate the process if graceful shutdown times out.
  --kill-timeout SECONDS  Set the wait after forced termination. [default: 5]
                          Requires --kill.
  --pid PID               Select the world process.
  --json                  Emit one JSON value on stdout.
  -h, --help              Show help for this command.
"
			),
			"{}",
			text
		);
		let (_, eval) = commands.iter().find(|(path, _)| path == &["eval".to_owned()]).unwrap();
		assert!(render(page(&["eval"]).unwrap(), eval).contains("\n  -f, --file FILE    Read the expression from FILE.\n"));
		let (_, launch) = commands.iter().find(|(path, _)| path == &["launch".to_owned()]).unwrap();
		assert!(
			render(page(&["launch"]).unwrap(), launch).contains("\n  -D NAME[=VALUE]   Set a preprocessor define used by hot reload. Repeatable.\n")
		);
		let (_, trace) = commands.iter().find(|(path, _)| path == &["trace".to_owned()]).unwrap();
		assert!(render(page(&["trace"]).unwrap(), trace).contains(
			"\n  --by EXPR       Group hits by expression value and print counts at exit.\n  --by caller     Group hits by the calling proc and \
			 source line.\n"
		));
	}
}
