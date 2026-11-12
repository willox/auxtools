//! Drives the launcher the way VS Code would, in the Debug Adapter Protocol,
//! against a world with none of auxtools' procs in it: it has to load the
//! debugger itself, stop at breakpoints, pick up again after the world
//! reboots, and go away when the world ends. Then a world launched without
//! debugging is found with `--list`, two more launchers attach to it at once,
//! and the first hot reloads it. Then the launcher's command line pauses it
//! and looks around, a command at a time. Last, the command line starts
//! worlds itself (`launch`, in the foreground and in the background), reboots
//! them and stops them.
//!
//! Build the launcher and the debug library first:
//! `cargo build -p auxtools-cli -p debug_server -p launcher_test`, then run
//! this from the repository's root with `BYOND_PATH` set.
//! `LAUNCHER_TEST_SERVER` picks the program on Windows (`dd` unless set:
//! `dreamseeker` and `dreamdaemon` need a display). `LAUNCHER_TEST_ARGS`
//! replaces the program's arguments (Windows' DreamDaemon asks before it runs a
//! world `-trusted`).

// The debug client, which dap_test uses more of
#[allow(dead_code)]
#[path = "../../dap_test/src/client.rs"]
mod client;
mod inspect;
mod journal;
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;

use std::{
	io::{BufRead, BufReader, Read},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::{Arc, Mutex},
	time::{Duration, Instant}
};

use client::{launcher_command, launcher_path, library_path, test_args, Launcher};
use paths::ByondCommand;
use serde_json::json;

/// The line `target` doubles its argument on
const TARGET_LINE: u64 = 28;

fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/launcher_test_host").join(name)
}

/// What the client checks at the breakpoint in `target`, in each world
fn at_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("target"), "stopped in {}", frame);
	assert_eq!(frame["line"], TARGET_LINE);
	let source = frame["source"]["path"].as_str().unwrap_or_default();
	assert!(source.ends_with("launcher_test_host.dm"), "the frame's source is {:?}", source);

	let scopes = launcher.request("scopes", json!({ "frameId": frame["id"] }));
	let arguments = scopes["scopes"]
		.as_array()
		.unwrap()
		.iter()
		.find(|s| s["name"] == "Arguments")
		.expect("no arguments scope")
		.clone();
	let vars = launcher.request("variables", json!({ "variablesReference": arguments["variablesReference"] }));
	let n = vars["variables"]
		.as_array()
		.unwrap()
		.iter()
		.find(|v| v["name"] == "n")
		.expect("no argument n")
		.clone();
	assert_eq!(n["value"], "21");
	assert_eq!(launcher.evaluate(&frame, "n + 1"), "22");
	println!("ok: line breakpoint and local value");
	launcher.resume();
}

fn at_second_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("second_target"), "stopped in {}", frame);
	println!("ok: function breakpoint");
	launcher.resume();
}

fn at_runtime(launcher: &mut Launcher) {
	launcher.stopped("exception");
	let info = launcher.request("exceptionInfo", json!({ "threadId": 0 }));
	assert_eq!(info["exceptionId"], "runtime error");
	let message = info["description"].as_str().unwrap();
	assert!(message.contains("cannot read from list"), "the runtime was {:?}", message);
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("fail"), "the runtime was in {}", frame);
	println!("ok: runtime stop ({})", message);
	launcher.resume();
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(host_file("launcher_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");
	let _ = std::fs::remove_file(host_file("rebooted"));

	command_line_shape();
	another_version();

	let mut launcher = Launcher::start();
	launcher.initialize();

	let server = client::server();
	let library = library_path();
	launcher.request(
		"launch",
		json!({
			"dme": host_file("launcher_test_host.dme"),
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);

	launcher.event("initialized");
	println!("ok: debug library initialization");
	let set = launcher.request(
		"setBreakpoints",
		json!({
			"source": { "path": host_file("launcher_test_host.dm") },
			"breakpoints": [{ "line": TARGET_LINE }]
		})
	);
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	let set = launcher.request("setFunctionBreakpoints", json!({ "breakpoints": [{ "name": "/proc/second_target" }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the function breakpoint wasn't set: {}", set);
	launcher.request("setExceptionBreakpoints", json!({ "filters": ["runtimes"] }));
	launcher.request("configurationDone", json!({}));

	at_target(&mut launcher);
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	// The world reboots, and the launcher gives the new one the breakpoints
	at_target(&mut launcher);
	assert!(
		launcher.output.contains("The world restarted"),
		"the launcher didn't see the world restart"
	);
	println!("ok: breakpoint restoration after reboot");
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	// The world ends, and the launcher with it
	launcher.event("terminated");
	launcher.event("exited");
	// The programs with windows show world.log in them
	if server == "dd" || cfg!(unix) {
		assert!(
			launcher.output.contains("launcher test: world/New"),
			"the world's output wasn't passed on"
		);
		assert!(launcher.output.contains("launcher test: done"), "world did not complete the test");
		println!("ok: world output before exit");
	} else {
		println!("ok: world exit");
	}

	launcher.disconnect(json!({}));
	println!("ok: launcher exit");
	let _ = std::fs::remove_file(host_file("rebooted"));

	attach_and_reload(&server, &library);
	launch_from_the_command_line(&server);
	without_debug_info(&server);
	unread_statics(&server);
	println!("all ok");
}

/// The command line, a command at a time: each connects, does it and leaves,
/// and the world stays paused between them
fn command_line(pid: u64, code: &Path) {
	let pid = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};

	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	let printed = run(&["wait", "pause", "--timeout", "1"], 2);
	assert_eq!(printed.trim(), "error: timed out after 1 second waiting for the world to pause.");

	let line = line_of(code, "\treturn 2").to_string();
	let printed = run(&["wait", "pause", "--break", &format!("attach_host.dm:{}", line), "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value  attach_host.dm:", pid)) && printed.contains("#1 /run_until_stopped"),
		"unexpected wait output: {}",
		printed
	);
	let printed = run(&["status"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"it didn't stay paused: {}",
		printed
	);
	// The pid goes after the command as well as before it
	let (got, after) = launcher_command(&["status", "--pid", &pid]);
	assert_eq!(
		(got, after.lines().next()),
		(0, printed.lines().next()),
		"unexpected status --pid output: {}",
		after
	);
	run(&["continue"], 0);

	// A line with no code of its own breaks on the next that has
	let declared = line_of(code, "/proc/value()");
	let printed = run(
		&["wait", "pause", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "60"],
		0
	);
	assert!(
		printed.starts_with(&format!(
			"pid {}: paused at a breakpoint\n#0 /value  attach_host.dm:{}\n",
			pid,
			declared + 1
		)),
		"unexpected wait output: {}",
		printed
	);
	assert_eq!(run(&["eval", "6", "*", "7"], 0).trim(), "42");
	let printed = run(&["step-over"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-over said {}",
		printed
	);
	let printed = run(&["vars"], 0);
	assert!(printed.contains("locals:"), "unexpected vars output: {}", printed);
	assert_eq!(run(&["continue"], 0).trim(), format!("pid {}: resumed", pid));
	assert_eq!(run(&["continue"], 1).trim(), "error: the world is not paused.");
	assert_eq!(
		run(&["stack"], 1).trim(),
		"error: the world is running. Pause it first with 'auxtools pause' or 'auxtools wait pause'."
	);
	println!("ok: CLI breakpoint inspection and stepping");

	let printed = run(&["pause"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused by request\n#0 /run_until_stopped", pid)),
		"unexpected pause output: {}",
		printed
	);
	let printed = run(&["wait", "pause", "--break", "/proc/value"], 0);
	assert!(
		printed.starts_with(&format!("already paused\npid {}: paused by request\n", pid)),
		"unexpected wait output: {}",
		printed
	);
	let resumed: serde_json::Value = serde_json::from_str(&run(&["continue", "--json"], 0)).unwrap();
	assert_eq!(resumed, json!({ "pid": pid.parse::<u64>().unwrap(), "resumed": true }));
	let printed = run(&["wait", "pause", "--break", "/proc/value", "--if", "1 == 2", "--timeout", "2"], 2);
	assert!(
		printed.contains("error: timed out after 2 seconds waiting for the world to pause."),
		"unexpected wait output: {}",
		printed
	);
	let printed = run(&["wait", "pause", "--break", "nowhere.dm:3"], 1);
	assert!(
		printed.contains("the world has no code from a file named nowhere.dm"),
		"unexpected wait output: {}",
		printed
	);
	println!("ok: CLI pause and wait timeouts");
}

/// What the command line is, before any world: a word for each command, and
/// help that says what each does
fn command_line_shape() {
	// Alone, it says how it's used, and doesn't wait to be spoken to
	let (code, alone) = launcher_command(&[]);
	assert_eq!(code, 0, "{}", alone);
	assert!(
		alone.starts_with("auxtools - Launch, inspect, and debug BYOND worlds\n"),
		"auxtools said {}",
		alone
	);
	assert!(
		alone.contains("Usage:\n  auxtools [--pid PID] COMMAND [ARGS] [OPTIONS]\n"),
		"auxtools said {}",
		alone
	);
	for command in [
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
		assert!(
			alone.lines().any(|line| line.starts_with(&format!("  {} ", command))),
			"{} isn't in the help:\n{}",
			command,
			alone
		);
	}
	for args in [&["help"][..], &["--help"][..], &["-h"][..]] {
		let (code, printed) = launcher_command(args);
		assert_eq!(
			(code, printed == alone),
			(0, true),
			"unexpected output from `auxtools {}`: {}",
			args.join(" "),
			printed
		);
	}
	println!("ok: command listing and top-level help");

	// A command's page, and one of a command of a command
	let (code, of_wait) = launcher_command(&["help", "wait"]);
	assert_eq!(code, 0);
	assert!(
		of_wait.starts_with("Wait for a debugger or world lifecycle event.\n\nUsage:\n  auxtools [--pid PID] wait pause [OPTIONS]\n"),
		"help wait said {}",
		of_wait
	);
	assert_eq!(launcher_command(&["wait", "--help"]), (0, of_wait.clone()));
	// Alone, it's told what it can wait for, and that it waited for nothing
	let (code, bare) = launcher_command(&["wait"]);
	assert_eq!(code, 1, "{}", bare);
	assert_eq!(bare, format!("{}error: wait requires a command: pause, reload, or reboot.\n", of_wait));
	let (code, of_pause) = launcher_command(&["help", "wait", "pause"]);
	assert_eq!(code, 0);
	assert!(
		of_pause
			.starts_with("Wait for execution to pause and report the location and reason.\n\nUsage:\n  auxtools [--pid PID] wait pause [OPTIONS]\n")
			&& of_pause.contains("\n  --timeout SECONDS           Set the wait deadline. [default: unlimited]\n")
			&& of_pause.contains("\n  --pid PID                   Select the world process.\n")
			&& of_pause.contains("\n  --response-timeout SECONDS  Set the wait for each response from the world. [default: 30]\n")
			&& of_pause.contains("\n  -h, --help                  Show help for this command.\n")
			&& of_pause.contains("\nExit codes:\n  0  World paused, or was already paused.\n")
			&& !of_pause.contains("--log"),
		"help wait pause said {}",
		of_pause
	);
	assert_eq!(launcher_command(&["wait", "pause", "--help"]), (0, of_pause.clone()));
	assert_eq!(launcher_command(&["wait", "pause", "-h"]), (0, of_pause));
	let (code, of_trace) = launcher_command(&["help", "trace"]);
	assert_eq!(code, 0);
	assert!(
		of_trace
			.starts_with("Log or count breakpoint hits without pausing execution.\n\nUsage:\n  auxtools [--pid PID] trace LOCATION... [OPTIONS]\n")
			&& of_trace.contains("  --by caller "),
		"help trace said {}",
		of_trace
	);
	for (args, said) in [
		(
			&["help", "frobnicate"][..],
			"error: unknown command 'frobnicate'. Run 'auxtools --help' for available commands."
		),
		(
			&["help", "wait", "soon"],
			"error: unknown wait command 'soon'. Run 'auxtools wait --help' for available commands."
		),
		(
			&["frobnicate"],
			"error: unknown command 'frobnicate'. Run 'auxtools --help' for available commands."
		),
		(
			&["wait", "--break", "/proc/x"],
			"error: unexpected argument '--break' before a wait command: specify pause, reload, or reboot. Run 'auxtools wait --help' for usage."
		),
		(&["next"], "error: unknown command 'next'. Run 'auxtools --help' for available commands.")
	] {
		let (code, printed) = launcher_command(args);
		assert_eq!((code, printed.trim()), (1, said), "`auxtools {}`", args.join(" "));
	}
	println!("ok: command help and unknown-command diagnostics");

	let (code, version) = launcher_command(&["version"]);
	assert_eq!(code, 0);
	assert!(
		version.starts_with("auxtools ") && protocol(&version) > 0,
		"unexpected version output: {}",
		version
	);
	assert_eq!(launcher_command(&["--version"]), (0, version.clone()));
	assert_eq!(launcher_command(&["-V"]), (0, version.clone()));
	let (code, as_json) = launcher_command(&["version", "--json"]);
	let as_json: serde_json::Value = serde_json::from_str(&as_json).unwrap_or_else(|_| panic!("unexpected version --json output: {}", as_json));
	assert_eq!(code, 0);
	assert_eq!(as_json["protocol"], protocol(&version));
	assert_eq!(
		version.trim(),
		format!("auxtools {} (protocol {})", as_json["version"].as_str().unwrap(), as_json["protocol"])
	);
	println!("ok: version and protocol output");

	// What's wrong with a command's options is said before any world is looked
	// for: there's none here
	for (args, said) in [
		(&["vars", "--depth", "3"][..], "error: --depth requires --json."),
		(&["eval", "1", "--wait"], "error: --wait requires --spawn."),
		(
			&["eval", "1", "--frame", "0", "--src", "0x1"],
			"error: --frame and --src cannot be used together."
		),
		(
			&["eval", "1", "--frame", "0", "--spawn"],
			"error: --frame and --spawn cannot be used together."
		),
		(&["eval"], "error: eval requires an expression: EXPRESSION, --file FILE, or - for stdin."),
		(&["stop", "--kill-timeout", "1"], "error: --kill-timeout requires --kill."),
		(
			&["trace", "/proc/x", "--by", "n", "--log", "x"],
			"error: --by and --log cannot be used together."
		),
		(
			&["wait", "pause", "--no-hold", "--hold-max", "3"],
			"error: --no-hold and --hold-max cannot be used together."
		),
		(&["launch", "nothing.dmb", "--json"], "error: --json requires --detach."),
		(
			&["--pid", "1", "list"],
			"error: --pid cannot be used with 'list': it does not select an existing world."
		),
		(
			&["version", "--pid", "1"],
			"error: --pid cannot be used with 'version': it does not select an existing world."
		),
		(
			&["list", "--response-timeout", "1"],
			"error: --response-timeout cannot be used with 'list': it does not select an existing world."
		),
		(
			&["--response-timeout", "1", "stop"],
			"error: --response-timeout cannot be used with 'stop': it waits for the process to exit; use --timeout."
		),
		(
			&["trace"],
			"error: missing required argument: LOCATION. Run 'auxtools trace --help' for usage."
		),
		(
			&["trace", "/proc/x", "--count", "0"],
			"error: invalid value '0' for --count: expected a positive integer. Run 'auxtools trace --help' for usage."
		),
		(
			&["reboot", "--timeout", "soon"],
			"error: invalid value 'soon' for --timeout: expected a non-negative number of seconds. Run 'auxtools reboot --help' for usage."
		),
		(
			&["stack", "--sideways"],
			"error: unexpected argument '--sideways'. Run 'auxtools stack --help' for usage."
		),
		(
			&["vars", "--frame"],
			"error: --frame requires a value: N. Run 'auxtools vars --help' for usage."
		),
		(
			&["--pid", "soon", "status"],
			"error: invalid value 'soon' for --pid: expected a process ID. Run 'auxtools status --help' for usage."
		)
	] {
		let (code, printed) = launcher_command(args);
		assert_eq!((code, printed.trim()), (1, said), "`auxtools {}`", args.join(" "));
	}
	println!("ok: option validation before connection");

	let (code, list) = launcher_command(&["list", "--json"]);
	assert_eq!(code, 0, "{}", list);
	assert!(
		serde_json::from_str::<serde_json::Value>(&list).is_ok_and(|worlds| worlds.is_array()),
		"list --json said {}",
		list
	);
	let (code, list) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", list);
	assert!(
		list.starts_with("PID ") || list == "No worlds found with the debug library loaded.\n",
		"list said {}",
		list
	);
	for gone in ["--list", "--build"] {
		let (code, printed) = launcher_command(&[gone]);
		assert_eq!(code, 1, "unexpected output from `auxtools {}`: {}", gone, printed);
	}
	println!("ok: world listing and legacy flag rejection");
}

/// The protocol version in what `auxtools version` prints
fn protocol(version: &str) -> u32 {
	version
		.trim()
		.rsplit_once("(protocol ")
		.and_then(|(_, number)| number.strip_suffix(')'))
		.and_then(|number| number.parse().ok())
		.unwrap_or_else(|| panic!("unexpected version output: {}", version))
}

/// A world whose debug library speaks another version of the protocol: it's
/// listed as one that can't be attached to, and attaching says why. The
/// stand-in is a BYOND server with no debug library, and this test listening
/// where the library of the version before ours would.
fn another_version() {
	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};

	let mut plain = Command::new(paths::find_dreamdaemon())
		.with_byond_paths()
		.arg(host_file("launcher_test_host.dmb"))
		.args(test_args().unwrap_or_else(|| vec!["-trusted".to_owned()]))
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let pid = plain.id();
	let ours = protocol(&launcher_command(&["version"]).1);
	let theirs = ours - 1;
	let name = format!("auxtools-debug-v{}-{}", theirs, pid);
	let listener = ListenerOptions::new()
		.name(name.to_ns_name::<GenericNamespaced>().unwrap())
		.create_sync()
		.unwrap();
	// Whoever connects is let in, and told nothing
	std::thread::spawn(move || {
		for client in listener.incoming() {
			drop(client);
		}
	});

	let (code, list) = launcher_command(&["list", "--json"]);
	assert_eq!(code, 0, "{}", list);
	let worlds: serde_json::Value = serde_json::from_str(&list).unwrap();
	let listed = worlds
		.as_array()
		.unwrap()
		.iter()
		.find(|world| world["pid"] == pid)
		.unwrap_or_else(|| panic!("the other version's world isn't listed: {}", list));
	assert_eq!(
		(&listed["protocol"], &listed["compatible"]),
		(&json!(theirs), &json!(false)),
		"{}",
		listed
	);
	// The table says the same
	let (code, table) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", table);
	let header: Vec<&str> = table.lines().next().unwrap_or_default().split_whitespace().collect();
	assert_eq!(
		header,
		["PID", "PROGRAM", "PROTOCOL", "COMPATIBLE", "RESPONSIVE", "DMB", "CWD", "ARGS"],
		"list said {}",
		table
	);
	let row: Vec<&str> = table
		.lines()
		.find(|line| line.split_whitespace().next() == Some(pid.to_string().as_str()))
		.unwrap_or_else(|| panic!("the other version's world isn't in the table: {}", table))
		.split_whitespace()
		.collect();
	assert_eq!((row[2], row[3], row[4]), (theirs.to_string().as_str(), "no", "-"), "list said {}", table);
	assert_eq!(listed["responsive"], json!(null), "{}", listed);

	let pid_text = pid.to_string();
	let (code, printed) = launcher_command(&["--pid", &pid_text, "status"]);
	assert_eq!(code, 1, "{}", printed);
	assert!(
		printed.contains(&format!(
			"its debug library speaks protocol {}, and this auxtools speaks {}",
			theirs, ours
		)),
		"attaching to another version's world said {}",
		printed
	);
	let _ = plain.kill();
	let _ = plain.wait();
	println!("ok: incompatible protocol discovery and attach rejection");
}

/// The rest of the command line: the other steps, the stacks, a frame's
/// variables, runtime errors, and what it says to what it doesn't understand
fn command_line_more(pid: u64, dir: &Path) {
	let pid = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};

	let printed = run(&["wait", "pause", "--break", "/proc/value", "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"unexpected wait output: {}",
		printed
	);
	let printed = run(&["step-out", "--timeout", "30"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-out said {}",
		printed
	);
	let printed = run(&["step-in"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-in said {}",
		printed
	);
	println!("ok: CLI Step Out and Step Into");

	let printed = run(&["stack"], 0);
	assert!(
		printed.starts_with("#0 /run_until_stopped") && !printed.contains("stack 0"),
		"stack said {}",
		printed
	);
	let printed = run(&["stack", "--all"], 0);
	// The running proc's first, then the sleeping ones, each with the id it
	// keeps
	let first = printed.split_once(": ").map(|(_, rest)| rest).unwrap_or_default();
	assert!(
		printed.starts_with("stack ")
			&& first.starts_with("/run_until_stopped\n#0 /run_until_stopped")
			&& printed.contains(": /doze (sleeping)\n#0 /doze  attach_host.dm:"),
		"stack --all said {}",
		printed
	);
	// Exclude the sleeping debugger helper.
	assert!(!printed.contains("auxtools_keepalive"), "stack --all said {}", printed);
	println!("ok: CLI running and sleeping stacks");

	let printed = run(&["vars"], 0);
	assert!(
		printed.contains("arguments:\n  src = null") && printed.contains("locals:") && !printed.contains("globals:"),
		"unexpected vars output: {}",
		printed
	);
	let printed = run(&["vars", "--frame", "0", "--globals"], 0);
	assert!(printed.contains("globals:"), "unexpected vars --globals output: {}", printed);
	let printed = run(&["vars", "--frame", "7"], 1);
	assert!(
		printed.starts_with("error: frame 7 does not exist: the paused stack has "),
		"vars --frame 7 said {}",
		printed
	);
	assert_eq!(run(&["eval", "--frame", "0", "6", "*", "7"], 0).trim(), "42");
	// Its options go after its words as well
	assert_eq!(run(&["eval", "6", "*", "7", "--frame", "0"], 0).trim(), "42");
	assert_eq!(run(&["eval", "-1"], 0).trim(), "-1");
	assert_eq!(run(&["eval", "--", "-rounds", "<", "0"], 0).trim(), "1");
	let printed = run(&["eval", "--frame", "7", "1"], 1);
	assert!(
		printed.starts_with("error: frame 7 does not exist: the paused stack has "),
		"eval --frame 7 said {}",
		printed
	);
	let printed = run(&["eval", "1", "+"], 1);
	assert!(printed.starts_with("error: parser error: "), "unexpected eval output: {}", printed);
	assert_eq!(
		run(&["eval", "nothing_called_this"], 1).trim(),
		"error: undefined var: nothing_called_this"
	);
	assert_eq!(run(&["eval", "1", "/", "0"], 1).trim(), "error: runtime error: Division by zero");
	println!("ok: CLI frame variables, globals and evaluation");
	// A proc's body, as one argument with newlines in it
	assert_eq!(run(&["eval", "var/n = 0\nfor(var/i in 1 to 3)\n\tn += i\nn"], 0).trim(), "6");
	assert_eq!(
		run(&["eval", "var/x = 1\nx = nothing_called_this"], 1).trim(),
		"error: line 2: undefined var: nothing_called_this"
	);
	println!("ok: CLI proc-body evaluation");
	run(&["continue"], 0);

	// The world fails when it finds this file
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = run(&["wait", "pause", "--runtimes", "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused on a runtime error: cannot read from list\n#0 /fail", pid)),
		"wait --runtimes said {}",
		printed
	);
	run(&["continue"], 0);
	println!("ok: CLI runtime wait");

	// Only runtime messages accepted by the filter should pause.
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = run(&["wait", "pause", "--runtime-filter", "!cannot read", "--timeout", "3"], 2);
	assert!(printed.contains("timed out"), "wait --runtime-filter said {}", printed);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = run(&["wait", "pause", "--runtime-filter", "bounds, FROM LIST", "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused on a runtime error: cannot read from list", pid)),
		"wait --runtime-filter said {}",
		printed
	);
	run(&["continue"], 0);
	println!("ok: CLI runtime filter");

	// A wait with its own breakpoint must ignore another client's pause.
	let waiting = std::process::Command::new(launcher_path())
		.args(["--pid", pid.as_str(), "wait", "pause", "--break", "/proc/fail", "--timeout", "60"])
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.unwrap();
	std::thread::sleep(Duration::from_secs(2));
	let printed = run(&["wait", "pause", "--break", "/proc/tick", "--timeout", "60"], 0);
	assert!(printed.contains("#0 /tick"), "the other wait said {}", printed);
	std::thread::sleep(Duration::from_secs(1));
	run(&["continue"], 0);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let out = waiting.wait_with_output().unwrap();
	let printed = String::from_utf8_lossy(&out.stdout);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /fail", pid)),
		"the waiting client said {}{}",
		printed,
		String::from_utf8_lossy(&out.stderr)
	);
	run(&["continue"], 0);
	println!("ok: CLI wait passes over another client's pause");

	// A wait that doesn't hold its pause: the world carries on when it's gone
	let printed = run(&["wait", "pause", "--break", "/proc/value", "--no-hold", "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"wait --no-hold said {}",
		printed
	);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	println!("ok: wait without retained pause");

	// A condition that fails with an error at each hit doesn't pause, and is
	// said once
	let printed = run(
		&["wait", "pause", "--break", "/proc/value", "--if", "rounds.len > 0", "--timeout", "2"],
		2
	);
	assert_eq!(printed.matches("reported once").count(), 1, "unexpected wait --if output: {}", printed);
	assert!(
		printed.contains("failed; skipping affected hits (reported once): runtime error: Cannot read ") && printed.contains("timed out"),
		"unexpected wait --if output: {}",
		printed
	);
	println!("ok: wait condition error reported once");

	// A global, watched from a running world; then a var of the proc it
	// paused in, until that changes
	let printed = run(&["wait", "pause", "--change", "rounds", "--timeout", "60"], 0);
	assert!(
		printed.starts_with("watching rounds, now ")
			&& printed.contains(&format!("\npid {}: paused after a change: rounds changed from ", pid))
			&& printed.contains("\n#0 /run_until_stopped"),
		"unexpected wait --change output: {}",
		printed
	);
	let printed = run(&["wait", "pause", "--change", "said", "--if", "value > old", "--timeout", "60"], 0);
	assert!(
		printed.starts_with("watching said, now ")
			&& printed.contains(&format!("\npid {}: paused after a change: said changed from ", pid))
			&& printed.contains("\n#0 /run_until_stopped"),
		"wait --change from a pause said {}",
		printed
	);
	let printed = run(&["wait", "pause", "--change", "1 + 1"], 1);
	assert!(
		printed.starts_with("error: cannot watch 1 + 1: "),
		"unexpected wait --change output: {}",
		printed
	);
	run(&["continue"], 0);
	println!("ok: CLI global and frame watches");

	for (args, said) in [
		(&["frobnicate"][..], "error: unknown command 'frobnicate'."),
		(
			&["vars", "--frame", "top"][..],
			"error: invalid value 'top' for --frame: expected a non-negative integer."
		),
		(
			&["wait", "pause", "--break", "somewhere"][..],
			"error: invalid location 'somewhere': expected FILE:LINE or /proc/path[#N]."
		),
		(&["wait", "now"][..], "error: unknown wait command 'now'."),
		(
			&["wait", "pause", "--break", "/proc/value", "--log", "hit"][..],
			"error: unexpected argument '--log'."
		),
		(
			&["wait", "pause", "--break", "/proc/value", "--if", "1 =="][..],
			"error: cannot set a breakpoint at /proc/value: the condition doesn't compile: parser error: "
		)
	] {
		let printed = run(args, 1);
		assert!(printed.starts_with(said), "unexpected output from `{}`: {}", args.join(" "), printed);
	}
	let (code, printed) = launcher_command(&["--pid", "4000000000", "status"]);
	assert_eq!(code, 1);
	assert!(
		printed.starts_with("error: cannot connect to the world with pid 4000000000: "),
		"status said {}",
		printed
	);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	println!("ok: invalid command diagnostics");
}

/// `trace`: breakpoints that print or count their hits and never pause
fn command_line_trace(pid: u64, code: &Path) {
	let pid = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	// Hot reloaded to return 2 by now
	let value = line_of(code, "\treturn 2");
	let called = line_of(code, "\t\tworld.log << \"attach test: [value()]\"");
	let every = line_of(code, "\ttick(1)");
	let other = line_of(code, "\t\ttick(2)");

	// A line a hit, saying where and what called it
	let printed = run(&["trace", "/proc/value", "--count", "3"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	let hit = format!(
		"/value  attach_host.dm:{}, called by /run_until_stopped  attach_host.dm:{}",
		value, called
	);
	let summary = "summary: 3 hits, 0 skipped; ended: count".to_owned();
	assert_eq!(
		lines,
		[
			format!("tracing /value  attach_host.dm:{}", value),
			hit.clone(),
			hit.clone(),
			hit,
			summary
		],
		"unexpected trace output: {}",
		printed
	);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	println!("ok: trace locations, callers and count limit");

	let printed = run(
		&[
			"trace",
			&format!("attach_host.dm:{}", value),
			"--log",
			"rounds then: {rounds > 0}",
			"--count",
			"2"
		],
		0
	);
	assert_eq!(
		printed.lines().skip(1).collect::<Vec<_>>(),
		["rounds then: 1", "rounds then: 1", "summary: 2 hits, 0 skipped; ended: count"],
		"unexpected trace --log output: {}",
		printed
	);
	println!("ok: trace message interpolation");

	// tick(1) is called every round, tick(2) every other
	let started = Instant::now();
	let printed = run(&["trace", "/proc/tick", "--by", "caller", "--for", "3"], 0);
	assert!(started.elapsed() >= Duration::from_secs(3), "trace --for 3 ended early");
	let lines: Vec<&str> = printed.lines().collect();
	let count = |line: &str| line.split_whitespace().next().unwrap().parse::<u32>().unwrap();
	assert!(
		lines.len() == 4
			&& lines[1].ends_with(&format!("  /tally  attach_host.dm:{}", every))
			&& lines[2].ends_with(&format!("  /tally  attach_host.dm:{}", other))
			&& count(lines[1]) >= count(lines[2])
			&& count(lines[2]) >= 2
			&& lines[3] == format!("summary: {} hits, 0 skipped; ended: time", count(lines[1]) + count(lines[2])),
		"trace --by caller said {}",
		printed
	);
	let printed = run(&["trace", "/proc/tick", "--by", "n", "--count", "6"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	assert!(
		lines.len() == 4
			&& lines[1].ends_with("  1")
			&& lines[2].ends_with("  2")
			&& count(lines[1]) == 6 - count(lines[2])
			&& lines[3] == "summary: 6 hits, 0 skipped; ended: count",
		"trace --by n said {}",
		printed
	);
	println!("ok: trace grouping and limits");

	// A condition that fails is said once, and the hits it skipped counted
	let printed = run(&["trace", "/proc/tick", "--if", "n.len > 0", "--for", "2"], 0);
	assert_eq!(printed.matches("reported once").count(), 1, "unexpected trace --if output: {}", printed);
	assert!(
		printed.contains("failed; skipping affected hits (reported once): runtime error: Cannot read ")
			&& printed
				.lines()
				.any(|line| line.starts_with("summary: 0 hits, ") && !line.contains(", 0 skipped") && line.ends_with(" skipped; ended: time"))
			&& !printed.contains("called by"),
		"unexpected trace --if output: {}",
		printed
	);
	println!("ok: trace condition errors and skipped-hit count");

	let printed = run(&["trace", "/proc/value", "--json", "--count", "1"], 0);
	let events: Vec<serde_json::Value> = printed.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
	assert!(
		events.len() == 3
			&& events[0]["event"] == "bound"
			&& events[0]["line"] == value
			&& events[1]["event"] == "hit"
			&& events[1]["caller"]["proc"] == "/run_until_stopped"
			&& events[1]["caller"]["line"] == called
			&& events[2]["event"] == "summary"
			&& events[2]["hits"] == 1
			&& events[2]["ended"] == "count",
		"trace --json said {}",
		printed
	);
	println!("ok: trace JSON Lines");

	// Read from by something that stops reading: it ends quietly
	let mut tracing = Command::new(client::launcher_path())
		.args(["--pid", &pid, "trace", "/proc/value"])
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let mut lines = BufReader::new(tracing.stdout.take().unwrap()).lines();
	assert!(lines.next().unwrap().unwrap().starts_with("tracing "));
	assert!(lines.next().unwrap().unwrap().starts_with("/value "));
	drop(lines);
	let traced = tracing.wait_with_output().unwrap();
	let err = String::from_utf8_lossy(&traced.stderr);
	assert!(
		err.is_empty() && traced.status.code().unwrap_or(0) == 0,
		"a trace into a closed pipe said {} ({})",
		err,
		traced.status
	);
	println!("ok: trace handles closed output pipe");

	// Closing stdout must also work for output larger than the pipe buffer.
	let mut evaluating = Command::new(client::launcher_path())
		.args(["--pid", &pid, "eval", "jointext(new/list(100000), \"xxxxxxxx\")"])
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let mut out = evaluating.stdout.take().unwrap();
	let mut start = [0; 9];
	out.read_exact(&mut start).unwrap();
	assert_eq!(&start, b"\"xxxxxxxx");
	drop(out);
	let evaluated = evaluating.wait_with_output().unwrap();
	let err = String::from_utf8_lossy(&evaluated.stderr);
	assert!(
		err.is_empty() && evaluated.status.code().unwrap_or(0) == 0,
		"eval failed after stdout closed: {} ({})",
		err,
		evaluated.status
	);
	println!("ok: eval handles closed output pipe");

	#[cfg(unix)]
	{
		let tracing = Command::new(client::launcher_path())
			.args(["--pid", &pid, "trace", "/proc/tick", "--by", "n"])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		std::thread::sleep(Duration::from_secs(2));
		Command::new("kill").args(["-INT", &tracing.id().to_string()]).status().unwrap();
		let traced = tracing.wait_with_output().unwrap();
		let out = String::from_utf8_lossy(&traced.stdout);
		assert!(
			traced.status.success() && out.contains(" skipped; ended: interrupted\n") && out.contains("  1\n"),
			"an interrupted trace said {}",
			out
		);
		println!("ok: interrupted trace summary");
	}

	for (args, said) in [
		(
			&["trace", "nowhere.dm:3"][..],
			"error: cannot set a breakpoint at nowhere.dm:3: the world has no code from a file named nowhere.dm."
		),
		(
			&["trace", "/proc/value", "/proc/nothing_called_this"][..],
			"error: cannot set a breakpoint at /proc/nothing_called_this: "
		),
		(
			&["trace", "/proc/value", "--if", "1 =="][..],
			"error: cannot set a breakpoint at /proc/value: the condition doesn't compile: "
		),
		(
			&["trace", "/proc/value", "--for=-1"][..],
			"error: invalid value '-1' for --for: expected a non-negative number of seconds."
		)
	] {
		let printed = run(args, 1);
		assert!(printed.contains(said), "unexpected output from `{}`: {}", args.join(" "), printed);
	}
	// The breakpoint set before the place that failed went with it
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	let printed = run(&["trace", "/proc/value", "--count", "1"], 0);
	assert_eq!(printed.lines().count(), 3, "unexpected trace output: {}", printed);
	println!("ok: invalid trace target diagnostics");
}

/// The line of `file` that `text` starts
fn line_of(file: &Path, text: &str) -> u64 {
	let source = std::fs::read_to_string(file).unwrap();
	source.lines().position(|line| line.starts_with(text)).expect("no such line") as u64 + 1
}

/// A world launched without debugging, found with `--list`, attached to by
/// two launchers at once, and hot reloaded by the one that launched it
fn attach_and_reload(server: &str, library: &Path) {
	let (dir, dme) = attach_host_copy("attach");
	let code = dir.join("attach_host.dm");

	let mut runner = Launcher::start();
	runner.initialize();
	runner.request(
		"launch",
		json!({
			"noDebug": true,
			"dme": dme,
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);
	runner.event("initialized");
	runner.request("configurationDone", json!({}));
	runner.wait_output("attach test: 1");
	runner.wait_output("Hot reload: watching");
	println!("ok: noDebug launch with source watching");

	let pid = client::listed_world("attach_host.dmb");
	println!("ok: world discovered with PID {}", pid);

	// Two debuggers: one finds the code from the world's command line, the
	// other is told where it is
	let mut first = Launcher::start();
	first.initialize();
	first.request("attach", json!({ "processId": pid.to_string() }));
	first.event("initialized");
	first.request("configurationDone", json!({}));
	let mut second = Launcher::start();
	second.initialize();
	// The original launcher already owns hot reload for this world.
	second.request("attach", json!({ "processId": pid, "dme": dme, "hotReload": true }));
	second.event("initialized");
	second.request("configurationDone", json!({}));
	second.wait_output("Hot reload disabled: the launcher (pid ");
	assert!(
		second.output.contains("already hot reloads this world"),
		"unexpected second-session output: {}",
		second.output
	);
	println!("ok: one reloader per world");

	// Either one's breakpoint stops the world for both, and either can let it
	// carry on
	let line = line_of(&code, "\treturn 1");
	let set = first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [{ "line": line }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	first.stopped("breakpoint");
	second.stopped("breakpoint");
	let frame = second.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("value"), "stopped in {}", frame);
	first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [] }));
	second.resume();
	first.event("continued");
	println!("ok: concurrent launcher sessions");

	// The launcher that started it reloads it as the code changes
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	runner.wait_output("Hot reload: reloaded /value");
	runner.wait_output("attach test: 2");
	assert!(
		!runner.output.contains("parsing the full project"),
		"a change to a proc's code read all the code again"
	);
	println!("ok: single-file source reload");

	// Leaving doesn't end a world that was attached to
	first.disconnect(json!({}));
	second.disconnect(json!({ "terminateDebuggee": false }));
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 2);
	println!("ok: attached-session disconnect preserves world");

	command_line(pid, &code);
	command_line_more(pid, &dir);
	inspect::command_line_inspect(pid, &dir, &code);
	command_line_trace(pid, &code);
	journal::runtimes(pid, &dir);
	journal::reloads(pid, &code);
	journal::hold_max(pid);
	journal::reboot(pid, &dir);

	// A launcher that attaches to a world that's paused hears it is
	let (code_given, printed) = launcher_command(&["--pid", &pid.to_string(), "pause"]);
	assert_eq!(code_given, 0, "unexpected pause output: {}", printed);
	let mut late = Launcher::start();
	late.initialize();
	late.request("attach", json!({ "processId": pid.to_string() }));
	late.event("initialized");
	late.request("configurationDone", json!({}));
	late.stopped("pause");
	late.resume();
	late.disconnect(json!({}));
	println!("ok: attach to a retained pause");
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 1);
	println!("ok: CLI resume");

	let waiting = journal::wait_for_reboot(pid);
	// A trace with no end of its own ends with the world, and says what it
	// counted
	let tracing = Command::new(client::launcher_path())
		.args(["--pid", &pid.to_string(), "trace", "/proc/value", "--by", "caller"])
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 2);
	std::fs::write(dir.join("stop"), "yes").unwrap();
	let traced = tracing.wait_with_output().unwrap();
	let (out, err) = (String::from_utf8_lossy(&traced.stdout), String::from_utf8_lossy(&traced.stderr));
	assert_eq!(traced.status.code(), Some(3), "unexpected trace output: {}{}", out, err);
	assert!(
		out.contains("; ended: world\n") && err.starts_with("error: world connection ended before the operation completed"),
		"unexpected trace output: {}{}",
		out,
		err
	);
	println!("ok: world exit ends trace with code 3");
	runner.event("terminated");
	journal::world_went_away(waiting);
	assert!(runner.output.contains("attach test: done"), "world did not complete the test");
	runner.disconnect(json!({}));
	println!("ok: world and launcher exit");
	let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of the world that runs until told to stop, built, in a directory of
/// its own named after `name`, so its code can be edited
fn attach_host_copy(name: &str) -> (PathBuf, PathBuf) {
	let dir = std::env::temp_dir().join(format!("launcher_test_{}-{}", name, std::process::id()));
	// A run before this one may have had this process's id (they come round
	// again quickly on Windows), and left its directory behind when a file in
	// it was still open: what it told its world to do (`refuse_del`) isn't
	// for this one
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	for left in ["stop", "fail", "reboot", "call", "refuse_del", "fail_del"] {
		let _ = std::fs::remove_file(dir.join(left));
		assert!(
			!dir.join(left).exists(),
			"{} is left from another run and can't be removed",
			dir.join(left).display()
		);
	}
	for file in ["attach_host.dme", "attach_host.dm"] {
		std::fs::copy(host_file(file), dir.join(file)).unwrap();
	}
	let dme = dir.join("attach_host.dme");
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");
	(dir, dme)
}

/// Processes the test started, ended when it's done with them, whether it
/// passes or fails: a world, a launcher in the background, a launcher in the
/// foreground (`child`)
#[derive(Default)]
struct Started {
	child: Option<Child>,
	pids: Vec<u64>
}

impl Drop for Started {
	fn drop(&mut self) {
		if let Some(child) = &mut self.child {
			let _ = child.kill();
			let _ = child.wait();
		}
		// A check that failed may not have got as far as hearing which
		// processes there were: the worlds of this run's copies are found, and
		// their launchers go with them
		if std::thread::panicking() {
			let list = Command::new(launcher_path()).args(["list", "--json"]).output().ok();
			let worlds: serde_json::Value = list.and_then(|list| serde_json::from_slice(&list.stdout).ok()).unwrap_or_default();
			let ours = format!("launcher_test_launch-{}", std::process::id());
			for world in worlds.as_array().into_iter().flatten() {
				if world["dmb"].as_str().is_some_and(|dmb| dmb.contains(&ours)) {
					self.pids.extend(world["pid"].as_u64());
				}
			}
		}
		for pid in &self.pids {
			let _ = if cfg!(windows) {
				Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
			} else {
				Command::new("kill").args(["-9", &pid.to_string()]).output()
			};
		}
	}
}

/// Whether process `pid` is running
fn alive(pid: u64) -> bool {
	if cfg!(windows) {
		let listed = Command::new("tasklist")
			.args(["/FI", &format!("PID eq {}", pid), "/NH"])
			.output()
			.unwrap();
		String::from_utf8_lossy(&listed.stdout)
			.split_whitespace()
			.any(|word| word == pid.to_string())
	} else {
		// One that's ended and not waited for yet is a zombie, `Z`
		std::fs::read_to_string(format!("/proc/{}/stat", pid))
			.is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
	}
}

fn gone_within(pid: u64, seconds: u64) -> bool {
	let until = Instant::now() + Duration::from_secs(seconds);
	while alive(pid) {
		if Instant::now() > until {
			return false;
		}
		std::thread::sleep(Duration::from_millis(100));
	}
	true
}

/// Waits for `text` to be in what `read` gives
fn wait_for_text(what: &str, text: &str, read: impl Fn() -> String) {
	let until = Instant::now() + Duration::from_secs(60);
	loop {
		let now = read();
		if now.contains(text) {
			return;
		}
		assert!(Instant::now() < until, "{} didn't say {:?}:\n{}", what, text, now);
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// `auxtools launch DMB` and the options every launch here has
fn launch_args(dmb: &Path, server: &str, more: &[&str]) -> Vec<String> {
	let mut args: Vec<String> = vec!["launch".into(), dmb.to_string_lossy().into_owned()];
	args.extend([
		"--byond".into(),
		paths::find_byond().to_string_lossy().into_owned(),
		"--server".into(),
		server.to_owned()
	]);
	args.extend(more.iter().map(|arg| (*arg).to_owned()));
	if let Some(theirs) = test_args() {
		args.push("--".into());
		args.extend(theirs);
	}
	args
}

/// `auxtools launch` in the foreground, its output collected as it comes
fn launch_here(dmb: &Path, server: &str, started: &mut Started) -> Arc<Mutex<String>> {
	let mut child = Command::new(launcher_path())
		.args(launch_args(dmb, server, &[]))
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let output = Arc::new(Mutex::new(String::new()));
	for stream in [
		Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
		Box::new(child.stderr.take().unwrap())
	] {
		let output = output.clone();
		std::thread::spawn(move || {
			for line in BufReader::new(stream).lines() {
				let Ok(line) = line else { return };
				let mut output = output.lock().unwrap();
				output.push_str(&line);
				output.push('\n');
			}
		});
	}
	started.child = Some(child);
	output
}

/// The exit code of the foreground launcher, once it's gone
fn launcher_exit(started: &mut Started, output: &Mutex<String>) -> i32 {
	let child = started.child.as_mut().unwrap();
	let until = Instant::now() + Duration::from_secs(30);
	loop {
		if let Some(status) = child.try_wait().unwrap() {
			return status.code().unwrap_or(-1);
		}
		assert!(Instant::now() < until, "timed out waiting for launcher exit:\n{}", output.lock().unwrap());
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// The process id in `launch`'s line about the program it started
fn started_pid(output: &str) -> u64 {
	output
		.lines()
		.find_map(|line| line.strip_prefix("Started ")?.rsplit_once("(process ")?.1.strip_suffix(')')?.parse().ok())
		.unwrap_or_else(|| panic!("launch output missing process ID:\n{}", output))
}

/// `auxtools launch`, `stop` and `reboot`: worlds the command line starts and
/// ends itself, in the foreground and in the background
fn launch_from_the_command_line(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let code = dir.join("attach_host.dm");

	// The launcher never builds: a world that isn't built isn't launched
	let missing = dir.join("nothing.dmb");
	for more in [&[][..], &["--detach"][..]] {
		let args = launch_args(&missing, server, more);
		let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
		assert_eq!(got, 1, "launch of a world that isn't there gave {}: {}", got, printed);
		assert!(
			printed.contains("nothing.dmb does not exist. Compile the world first"),
			"launch said {}",
			printed
		);
	}
	println!("ok: missing .dmb launch error");

	// Check foreground output, reloads and shutdown. The source baseline
	// includes a proc and var absent from the .dmb.
	let source = std::fs::read_to_string(&code).unwrap();
	let unbuilt = "\n/datum/held/var/unbuilt = 1\n\n/datum/held/proc/unbuilt_proc()\n\treturn src\n";
	std::fs::write(&code, format!("{}{}", source, unbuilt)).unwrap();
	let mut started = Started::default();
	let output = launch_here(&dmb, server, &mut started);
	let read = || output.lock().unwrap().clone();
	wait_for_text("launch", "Hot reload: watching", read);
	wait_for_text("launch", "attach test: 1", read);
	started.pids.push(started_pid(&read()));
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	wait_for_text("launch", "Hot reload: reloaded /value", read);
	wait_for_text("launch", "attach test: 2", read);
	// Removing these definitions must not claim runtime removals.
	wait_for_text("launch", "var unbuilt was removed from source and isn't in the running world", read);
	let printed = read();
	assert!(
		printed.contains("Hot reload: reloaded /value in ")
			&& !printed.contains("unlinked /")
			&& printed.contains("unbuilt_proc: not found in the running world; skipping removal")
			&& !printed.contains("retained in the running world"),
		"launch said:\n{}",
		printed
	);
	println!("ok: removing code the world never had");
	std::fs::write(dir.join("stop"), "yes").unwrap();
	let exit = launcher_exit(&mut started, &output);
	let printed = read();
	assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
	assert!(
		printed.contains("attach test: done") && printed.contains("The world exited with code 0"),
		"launch said:\n{}",
		printed
	);
	assert!(
		!printed.contains("Debug server disconnected") && !printed.contains("waiting to reconnect"),
		"launch took the world's end for a lost connection:\n{}",
		printed
	);
	std::fs::remove_file(dir.join("stop")).unwrap();
	std::fs::write(&code, source).unwrap();
	drop(started);
	println!("ok: foreground output and reloads");

	// Interrupted, it has the world end itself
	if cfg!(unix) {
		let mut started = Started::default();
		let output = launch_here(&dmb, server, &mut started);
		let read = || output.lock().unwrap().clone();
		wait_for_text("launch", "Hot reload: watching", read);
		started.pids.push(started_pid(&read()));
		let launcher = started.child.as_ref().unwrap().id().to_string();
		assert!(Command::new("kill").args(["-TERM", &launcher]).status().unwrap().success());
		let exit = launcher_exit(&mut started, &output);
		let printed = read();
		assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
		assert!(
			printed.contains("Interrupted: requesting graceful shutdown") && printed.contains("attach test: world/Del"),
			"launch said:\n{}",
			printed
		);
		println!("ok: interrupt requests graceful shutdown");
	}

	// In the background: it returns once the world is up, and other
	// commands work with it
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let log = PathBuf::from(up["log"].as_str().unwrap());
	assert_eq!(log, dir.join("attach_host.auxtools.log"));
	let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
	assert!(
		read_log().contains("Hot reload: watching"),
		"the world was up before its code was watched:\n{}",
		read_log()
	);
	let listed = client::listed_worlds();
	assert!(
		listed
			.iter()
			.any(|world| world["pid"] == pid && world["dmb"].as_str().is_some_and(|dmb| dmb.contains("launcher_test_launch-"))),
		"list doesn't show the world: {:?}",
		listed
	);
	let pid_text = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid_text.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	assert_eq!(run(&["eval", "6", "*", "7"], 0).trim(), "42");
	wait_for_text("the background launcher's log", "attach test: 1", read_log);
	println!("ok: detached launch readiness and commands");

	// Keep stalled worlds visible in both text and JSON listings.
	#[cfg(unix)]
	{
		let mine = |listed: Vec<serde_json::Value>| listed.into_iter().find(|world| world["pid"] == pid);
		let listed = mine(client::listed_worlds()).unwrap_or_else(|| panic!("list doesn't show the world"));
		assert_eq!(listed["responsive"], json!(true), "{}", listed);
		// Run checks concurrently to minimize time spent under SIGSTOP.
		assert!(Command::new("kill").args(["-STOP", &pid_text]).status().unwrap().success());
		let asked: Vec<_> = [
			&["list"][..],
			&["list", "--json"],
			&["--pid", &pid_text, "status", "--response-timeout", "1"],
			&["--pid", &pid_text, "--response-timeout", "1.5", "pause"]
		]
		.iter()
		.map(|args| {
			std::thread::spawn({
				let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
				move || launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>())
			})
		})
		.collect();
		let mut answers = asked.into_iter().map(|asking| asking.join().unwrap());
		let (code, table) = answers.next().unwrap();
		let stopped = answers.next().unwrap().1;
		let (status, paused) = (answers.next().unwrap(), answers.next().unwrap());
		assert!(Command::new("kill").args(["-CONT", &pid_text]).status().unwrap().success());
		let worlds: serde_json::Value = serde_json::from_str(&stopped).unwrap_or_else(|_| panic!("list --json said {}", stopped));
		let listed = worlds.as_array().unwrap().iter().find(|world| world["pid"] == pid);
		assert!(
			listed.is_some_and(|world| world["responsive"] == false && world["compatible"] == true && world["dmb"].is_null()),
			"list --json said {}",
			stopped
		);
		let row = table.lines().find(|line| line.split_whitespace().next() == Some(pid_text.as_str()));
		assert!(
			code == 0 && row.is_some_and(|row| row.split_whitespace().nth(4) == Some("no")),
			"list said {}",
			table
		);
		println!("ok: list shows unresponsive worlds");
		for ((code, printed), seconds) in [(status, "1 second"), (paused, "1.5 seconds")] {
			assert_eq!(
				(code, printed.as_str()),
				(
					2,
					format!(
						"error: timed out after {} waiting for the world to respond; it is busy or hung. Set --response-timeout to wait longer.\n",
						seconds
					)
					.as_str()
				),
				"a command to a stopped world"
			);
		}
		println!("ok: response timeouts for stalled worlds");
	}

	let printed = run(&["reboot"], 0);
	assert_eq!(printed.trim(), format!("pid {}: rebooted", pid));
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	let again: serde_json::Value = serde_json::from_str(&run(&["reboot", "--json", "--timeout", "60"], 0)).unwrap();
	assert_eq!(again, json!({ "pid": pid, "rebooted": true }));
	wait_for_text("the background launcher's log", "The world restarted", read_log);
	println!("ok: reboot waits for reconnection");

	// Reboot must wait for hot reloads to be restored.
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, format!("{}\n/proc/added_by_reload()\n\treturn 99\n", source)).unwrap();
	wait_for_text("the background launcher's log", "Hot reload: added /added_by_reload", read_log);
	run(&["reboot", "--timeout", "60"], 0);
	assert_eq!(run(&["eval", "added_by_reload()"], 0).trim(), "99");
	let loaded: serde_json::Value = serde_json::from_str(&run(&["log", "--world", "--last", "1", "--json"], 0)).unwrap();
	let status: serde_json::Value = serde_json::from_str(&run(&["status", "--json"], 0)).unwrap();
	assert!(
		loaded["event"] == "loaded"
			&& status["last_reload"]["applied"] == true
			&& status["last_reload"]["number"].as_u64() > loaded["number"].as_u64(),
		"after reboot, the world loaded at {} and the last reload is {}",
		loaded,
		status["last_reload"]
	);
	println!("ok: reboot waits for hot reloads to be reapplied");

	// Pause the launcher so it receives a source edit and reboot together.
	// The rebuilt world already contains the edit; do not apply it again.
	#[cfg(unix)]
	{
		let rebaselined = || read_log().matches("using the rebuilt .dmb as the source baseline").count();
		let before = rebaselined();
		let launcher_text = launcher.to_string();
		assert!(Command::new("kill").args(["-STOP", &launcher_text]).status().unwrap().success());
		let edited = std::fs::read_to_string(&code).unwrap();
		std::fs::write(&code, format!("{}\n/proc/zz_agent_probe()\n\treturn 7\n", edited)).unwrap();
		let built = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).output().unwrap();
		std::fs::write(dir.join("reboot"), "yes").unwrap();
		// The world reboots by itself within half a second
		std::thread::sleep(Duration::from_secs(3));
		assert!(Command::new("kill").args(["-CONT", &launcher_text]).status().unwrap().success());
		assert!(built.status.success(), "dreammaker build failed");
		let started = Instant::now();
		while rebaselined() == before {
			assert!(
				started.elapsed() < Duration::from_secs(60),
				"the launcher did not adopt the rebuilt .dmb:\n{}",
				read_log()
			);
			std::thread::sleep(Duration::from_millis(100));
		}
		assert_eq!(run(&["eval", "zz_agent_probe()"], 0).trim(), "7");
		let printed = run(&["proc", "/zz_agent_probe"], 0);
		assert!(
			printed.contains("definitions: 1\n") && printed.contains("reloaded: no\n"),
			"proc already included in the rebuilt .dmb was reloaded: {}\nlauncher log:\n{}",
			printed,
			read_log()
		);
		println!("ok: rebuilt worlds skip reloads already included in the build");
	}

	// Whenever in the world's round it's asked: a proc that wakes between the
	// world's Del() and the next world loading is the old world's, and the
	// debugger waits for the new one
	for moment in 0..8u64 {
		std::thread::sleep(Duration::from_millis(moment * 70));
		run(&["reboot", "--timeout", "60"], 0);
	}
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), format!("pid {}: running", pid));
	assert_eq!(run(&["eval", "6", "*", "7"], 0).trim(), "42");
	let said = read_log();
	assert!(
		!said.contains("ID mismatch") && !said.contains("panicked"),
		"the launcher's log has:\n{}",
		said
	);
	println!("ok: reboot throughout world execution");

	let before = read_log().len();
	let printed = run(&["stop"], 0);
	assert_eq!(printed.trim(), format!("pid {}: stopped", pid));
	assert!(!alive(pid), "world process is still running");
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	std::fs::write(&code, source).unwrap();
	let said = read_log();
	assert!(
		said.contains("attach test: world/Del") && said.contains("The world exited with code 0"),
		"the log has:\n{}",
		said
	);
	// Each reboot occurs before the shutdown notification, so clients
	// reconnect.
	assert!(
		!said[before..].contains("waiting to reconnect"),
		"the launcher took the world's end for a lost connection:\n{}",
		said
	);
	drop(started);
	println!("ok: stop shuts down world and detached launcher");

	// A world whose world/Del() doesn't end it
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--no-hot-reload"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let lines: Vec<&str> = printed.lines().collect();
	let pid: u64 = lines
		.first()
		.and_then(|line| line.strip_prefix("pid ")?.strip_suffix(": started")?.parse().ok())
		.unwrap_or_else(|| panic!("unexpected launch --detach output: {}", printed));
	let launcher: u64 = lines
		.get(1)
		.and_then(|line| line.strip_prefix("launcher: pid ")?.parse().ok())
		.unwrap_or_else(|| panic!("unexpected launch --detach output: {}", printed));
	assert_eq!(
		lines.get(2).copied(),
		Some(format!("log: {}", dir.join("attach_host.auxtools.log").display()).as_str()),
		"unexpected launch --detach output: {}",
		printed
	);
	started.pids.extend([pid, launcher]);
	std::fs::write(dir.join("refuse_del"), "yes").unwrap();
	let pid_text = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid_text.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	// Its code isn't read, so it has no macros, and says so
	let printed = run(&["eval", "DOUBLE_IT(1)"], 1);
	assert!(
		printed.starts_with("error: proc not found: /proc/DOUBLE_IT (if that's a #define: source macros unavailable"),
		"unexpected eval output: {}",
		printed
	);
	println!("ok: missing source-macro diagnostic");
	// A failed world/Del must leave the running world debuggable.
	std::fs::remove_file(dir.join("refuse_del")).unwrap();
	std::fs::write(dir.join("fail_del"), "yes").unwrap();
	let printed = run(&["stop", "--timeout", "10"], 1);
	assert!(
		printed.starts_with(&format!(
			"error: world.Del() failed with a runtime error (see the world's log), so the world (pid {}) is still running.",
			pid
		)),
		"stop said {}",
		printed
	);
	assert!(alive(pid), "the world ended");
	let printed = run(&["eval", "1 + 1"], 0);
	assert_eq!(printed.trim(), "2", "eval after the failed stop said {}", printed);
	std::fs::remove_file(dir.join("fail_del")).unwrap();
	std::fs::write(dir.join("refuse_del"), "yes").unwrap();
	println!("ok: stop reports a failing world/Del");
	let printed = run(&["stop", "--timeout", "2"], 2);
	assert!(
		printed.starts_with(&format!(
			"error: timed out after 2 seconds waiting for the world process (pid {}) to exit.",
			pid
		)),
		"stop said {}",
		printed
	);
	assert!(alive(pid), "the world ended");
	let printed = run(&["stop", "--timeout", "0", "--kill", "--kill-timeout", "10", "--json"], 0);
	let stopped: serde_json::Value =
		serde_json::from_str(printed.lines().next().unwrap_or_default()).unwrap_or_else(|_| panic!("stop --json said {}", printed));
	assert_eq!(stopped, json!({ "pid": pid, "stopped": true, "killed": true }), "stop said {}", printed);
	assert!(
		printed.contains("graceful shutdown timed out after 0 seconds; the process was terminated."),
		"stop said {}",
		printed
	);
	assert!(!alive(pid), "world process is still running");
	// (a process that's ended on Windows exits with the code it was ended with)
	let ended = if cfg!(windows) {
		"The world exited with code 1"
	} else {
		"The world was terminated"
	};
	wait_for_text("the background launcher's log", ended, || {
		std::fs::read_to_string(&log).unwrap_or_default()
	});
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	drop(started);
	println!("ok: stop timeout and forced termination");

	// (the directory can outlast this on Windows, while its files are open)
	let _ = std::fs::remove_file(dir.join("refuse_del"));
	let _ = std::fs::remove_dir_all(&dir);
}

/// Line breakpoints should explain missing debug info in non-DEBUG worlds.
fn without_debug_info(server: &str) {
	let (dir, dme) = attach_host_copy("nolines");
	let text = std::fs::read_to_string(&dme).unwrap().replace("#define DEBUG\n", "");
	std::fs::write(&dme, text).unwrap();
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");
	let dmb = dme.with_extension("dmb");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--no-hot-reload", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let launched: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap_or_default())
		.unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let pid = launched["pid"].as_u64().unwrap();
	started.pids.extend([pid, launched["launcher"].as_u64().unwrap()]);
	let pid_text = pid.to_string();
	let line = line_of(&dir.join("attach_host.dm"), "\ttick(1)").to_string();
	let place = format!("attach_host.dm:{}", line);
	let (got, printed) = launcher_command(&["--pid", &pid_text, "wait", "pause", "--break", &place, "--timeout", "5"]);
	assert_eq!(got, 1, "a line breakpoint without debug info gave {}: {}", got, printed);
	assert!(
		printed.contains("the world was compiled without debug information"),
		"a line breakpoint without debug info said {}",
		printed
	);
	let (got, printed) = launcher_command(&["--pid", &pid_text, "stop", "--timeout", "10", "--kill"]);
	assert_eq!(got, 0, "stop gave {}: {}", got, printed);
	drop(started);
	println!("ok: line breakpoints in a world without debug info");
	let _ = std::fs::remove_dir_all(&dir);
}

/// Preserve unread static locals and avoid rerunning their initializers.
fn unread_statics(server: &str) {
	let (dir, dme) = attach_host_copy("statics");
	let code = dir.join("attach_host.dm");
	let mut source = std::fs::read_to_string(&code).unwrap();
	source.push_str(
		"\nvar/inits = 0\n\n/proc/count_init(x)\n\tinits++\n\treturn x\n\n/proc/unread_static()\n\tvar/static/kept = count_init(1)\n\treturn \
		 1\n\n// __PROC__ needs BYOND 515.1644.\n#if DM_VERSION > 515 || DM_BUILD >= 1644\n#define STATIC_NAME __PROC__\n#else\n#define STATIC_NAME \
		 2\n#endif\n\n/proc/unread_static_proc()\n\tvar/static/named = count_init(STATIC_NAME)\n\treturn 1\n"
	);
	std::fs::write(&code, &source).unwrap();
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");
	let dmb = dme.with_extension("dmb");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let launched: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap_or_default())
		.unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let pid = launched["pid"].as_u64().unwrap();
	started.pids.extend([pid, launched["launcher"].as_u64().unwrap()]);
	let pid_text = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid_text.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	assert_eq!(run(&["eval", "inits"], 0).trim(), "2");

	for (n, edited) in ["1 + 1", "3"].iter().enumerate() {
		let before = if n == 0 { "1".to_owned() } else { "1 + 1".to_owned() };
		source = source.replace(
			&format!("count_init(1)\n\treturn {}\n", before),
			&format!("count_init(1)\n\treturn {}\n", edited)
		);
		source = source.replace(
			&format!("count_init(STATIC_NAME)\n\treturn {}\n", before),
			&format!("count_init(STATIC_NAME)\n\treturn {}\n", edited)
		);
		// Start before the edit so the wait can find a reload that already
		// finished.
		let since = journal::newest(pid).to_string();
		std::fs::write(&code, &source).unwrap();
		let printed = run(&["wait", "reload", "--since", &since, "--timeout", "60"], 0);
		assert!(printed.contains("hot reload applied"), "the reload said {}", printed);
		assert_eq!(run(&["eval", "inits"], 0).trim(), "2", "an initializer ran again");
	}
	assert_eq!(run(&["eval", "unread_static() + unread_static_proc()"], 0).trim(), "6");
	run(&["stop", "--timeout", "10", "--kill"], 0);
	drop(started);
	println!("ok: reloads keep static locals the code doesn't read");
	let _ = std::fs::remove_dir_all(&dir);
}
