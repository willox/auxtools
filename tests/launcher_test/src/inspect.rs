//! The command line's commands for looking at a world: what `eval` says when
//! it fails, expressions from files, JSON, where `wait`'s breakpoints bound,
//! a proc's bytecode and whereabouts, and steps of one instruction

use std::{
	io::Write,
	path::Path,
	process::{Command, Stdio}
};

use serde_json::{json, Value};

use super::{client::launcher_path, line_of};

/// What a command did: its exit code, and what it printed on stdout and on
/// stderr, apart
struct Ran {
	code: i32,
	out: String,
	err: String
}

/// `auxtools --pid PID ARGS...`, with `input` on its stdin
fn auxtools(pid: &str, args: &[&str], input: Option<&str>) -> Ran {
	let mut child = Command::new(launcher_path())
		.args(["--pid", pid])
		.args(args)
		.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	if let Some(input) = input {
		child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
	}
	let out = child.wait_with_output().unwrap();
	Ran {
		code: out.status.code().unwrap_or(-1),
		out: String::from_utf8_lossy(&out.stdout).into_owned(),
		err: String::from_utf8_lossy(&out.stderr).into_owned()
	}
}

/// Runs it, and checks it exited with `code`
fn run(pid: &str, args: &[&str], code: i32) -> Ran {
	let ran = auxtools(pid, args, None);
	assert_eq!(
		ran.code,
		code,
		"`{}` gave {}, not {}: {}{}",
		args.join(" "),
		ran.code,
		code,
		ran.out,
		ran.err
	);
	ran
}

/// Run a failing command. Require exit code 1 and empty stdout;
/// return stderr without its `error: ` prefix.
fn fails(pid: &str, args: &[&str]) -> String {
	let ran = run(pid, args, 1);
	assert!(
		ran.out.is_empty(),
		"expected empty stdout from failed command `{}`: {}",
		args.join(" "),
		ran.out
	);
	let said = ran
		.err
		.strip_prefix("error: ")
		.unwrap_or_else(|| panic!("expected error prefix on stderr from `{}`: {:?}", args.join(" "), ran.err));
	said.trim_end().to_owned()
}

/// Schedule fail_in() after the next breakpoint is installed.
/// The world consumes the trigger file, possibly before installation.
fn call_soon(dir: &Path) {
	let call = dir.join("call");
	std::thread::spawn(move || {
		std::thread::sleep(std::time::Duration::from_millis(1500));
		std::fs::write(call, "yes").unwrap();
	});
}

pub fn command_line_inspect(pid: u64, dir: &Path, code: &Path) {
	let pid = pid.to_string();
	let pid = pid.as_str();
	eval_failures(pid, code);
	from_files(pid, dir);
	procs(pid, code);
	where_breakpoints_bound(pid, code);
	instruction_steps(pid, dir);
	as_json(pid, dir);
	on_an_object(pid);
	macros(pid, code);
	spawned(pid, code);
	steps_that_end_elsewhere(pid, dir);
}

/// `eval` fails on stderr with exit code 1, whatever went wrong, and a
/// runtime error says where it happened
fn eval_failures(pid: &str, code: &Path) {
	let said = fails(pid, &["eval", "fail_in(3)"]);
	let lines: Vec<&str> = said.lines().collect();
	assert!(
		lines.len() == 4 && lines[0].starts_with("runtime error: "),
		"unexpected eval output: {}",
		said
	);
	assert_eq!(lines[1], format!("  in /fail_deeper  attach_host.dm:{}", line_of(code, "\treturn L[n]")));
	assert_eq!(
		lines[2],
		format!("  called by /fail_in  attach_host.dm:{}", line_of(code, "\tvar/got = fail_deeper(n)"))
	);
	assert_eq!(lines[3], "  called by the expression");
	// In the expression itself, there's nowhere else to say
	assert_eq!(fails(pid, &["eval", "1/0"]), "runtime error: Division by zero");
	// One the proc catches is no failure
	assert_eq!(run(pid, &["eval", "caught_inside()"], 0).out.trim(), "\"caught\"");
	// /client in an expression is the value compiled code has for it, and a
	// proc can be named without its /proc/
	assert_eq!(run(pid, &["eval", "/client == client_type()"], 0).out.trim(), "1");
	assert_eq!(
		run(pid, &["eval", "/datum/twice/act"], 0).out,
		run(pid, &["eval", "/datum/twice/proc/act"], 0).out
	);
	println!("ok: eval runtime error call stack");

	// Long loops yield in the background, so synchronous eval loses the result.
	let looped = "var/x = 0; for(var/i in 1 to 2000000) x++; return x";
	let said = fails(pid, &["eval", looped]);
	assert!(
		said.starts_with("evaluation aborted") && said.contains("background") && said.contains("eval --spawn --wait"),
		"unexpected eval output: {}",
		said
	);
	// Spawned eval can wait for the result.
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", looped], 0).out, "2000000\n");
	println!("ok: background loop evaluation");

	// Match the text BYOND prints, without embedded macros.
	assert_eq!(run(pid, &["eval", "\"\\improper AI \\bold Chamber\""], 0).out, "\"AI Chamber\"\n");
	let named = json_of(pid, &["eval", "--json", "new /obj/macro_named"]);
	assert!(named["name"] == "AI Chamber" && named["vars"]["name"] == "AI Chamber", "{}", named);
	println!("ok: text macro removal");

	// Paused, the same
	run(pid, &["pause"], 0);
	let said = fails(pid, &["eval", "fail_in(3)"]);
	assert!(
		said.contains("\n  in /fail_deeper  ") && said.ends_with("\n  called by the expression"),
		"unexpected eval output: {}",
		said
	);
	for (args, said) in [
		(&["eval", "1 +"][..], "parser error: "),
		(&["eval", "nothing_called_this"][..], "undefined var: nothing_called_this"),
		(&["eval", "--frame", "9", "1"][..], "frame 9 does not exist: the paused stack has "),
		(&["eval"][..], "eval requires an expression: EXPRESSION, --file FILE, or - for stdin.")
	] {
		let got = fails(pid, args);
		assert!(got.starts_with(said), "unexpected output from `{}`: {}", args.join(" "), got);
	}
	run(pid, &["continue"], 0);
	println!("ok: consistent eval failure output");
}

/// Expressions and messages from files and stdin, which no shell gets to
/// change on the way
fn from_files(pid: &str, dir: &Path) {
	let file = |name: &str, text: &str| {
		let path = dir.join(name);
		std::fs::write(&path, text).unwrap();
		path.to_str().unwrap().to_owned()
	};
	let expression = file("expression.txt", "\"[\"a b\"] /datum/x\" + \"!\"\n");
	assert_eq!(run(pid, &["eval", "-f", &expression], 0).out.trim(), "\"a b /datum/x!\"");
	assert_eq!(run(pid, &["eval", "--file", &expression], 0).out.trim(), "\"a b /datum/x!\"");
	let ran = auxtools(pid, &["eval", "-"], Some("6 * 7\n"));
	assert_eq!((ran.code, ran.out.trim()), (0, "42"), "unexpected eval - output: {}{}", ran.out, ran.err);
	let said = fails(pid, &["eval", "-f", "no such file.txt"]);
	assert!(said.starts_with("cannot read no such file.txt: "), "unexpected eval -f output: {}", said);
	assert_eq!(
		fails(pid, &["eval", "1", "-f", &expression]),
		"EXPRESSION and --file cannot be used together."
	);
	println!("ok: file and stdin evaluation");

	let never = file("never.txt", "1 == 2\n");
	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/value",
			"--if",
			&format!("@{}", never),
			"--timeout",
			"2"
		],
		2
	);
	assert!(ran.err.contains("timed out"), "unexpected wait output: {}{}", ran.out, ran.err);
	let message = file("message.txt", "value() said \"{1 + 1}\"\n");
	let ran = run(pid, &["trace", "/proc/value", "--log", &format!("@{}", message), "--count", "1"], 0);
	assert!(ran.out.contains("value() said \"2\"\n"), "unexpected trace --log output: {}", ran.out);
	let watched = file("watched.txt", "rounds");
	let ran = run(pid, &["wait", "pause", "--change", &format!("@{}", watched), "--timeout", "60"], 0);
	assert!(
		ran.out.starts_with("watching rounds, now "),
		"unexpected wait --change output: {}",
		ran.out
	);
	run(pid, &["continue"], 0);
	// Two @s are one, and no file
	let said = fails(pid, &["wait", "pause", "--break", "/proc/value", "--if", &format!("@@{}", never)]);
	assert!(
		said.starts_with("cannot set a breakpoint at /proc/value: the condition doesn't compile"),
		"unexpected wait output: {}",
		said
	);
	let said = fails(pid, &["wait", "pause", "--break", "/proc/value", "--if", "@no such file.txt"]);
	assert!(said.starts_with("cannot read no such file.txt: "), "unexpected wait output: {}", said);
	println!("ok: file-backed wait arguments and @ escaping");
}

/// Where a proc is, and its bytecode
fn procs(pid: &str, code: &Path) {
	// value() was hot reloaded before this
	let printed = run(pid, &["proc", "/proc/value"], 0).out;
	assert_eq!(
		printed,
		format!(
			"/value  attach_host.dm:{}\ndefinitions: 1\nreloaded: code (new code)\n",
			line_of(code, "\treturn 2")
		)
	);
	let printed = run(pid, &["proc", "/fail_in"], 0).out;
	assert!(printed.ends_with("\nreloaded: no\n"), "unexpected proc output: {}", printed);
	let printed = run(pid, &["proc", "/datum/twice/act#1"], 0).out;
	assert_eq!(
		printed,
		format!(
			"/datum/twice/act#1  attach_host.dm:{}\ndefinitions: 2\nreloaded: no\n",
			line_of(code, "\t. = ..()")
		)
	);
	let printed = run(pid, &["proc", "/datum/twice/proc/act"], 0).out;
	assert!(
		printed.starts_with(&format!("/datum/twice/act  attach_host.dm:{}\n", line_of(code, "\treturn \"first\""))),
		"unexpected proc output: {}",
		printed
	);
	for (args, said) in [
		(
			&["proc", "/proc/nothing_called_this"][..],
			"the world has no proc /proc/nothing_called_this"
		),
		(&["proc", "/value#3"][..], "/value has one definition, #0: there's no #3"),
		(
			&["proc", "/datum/twice/act#2"][..],
			"/datum/twice/act has 2 definitions, #0 to #1: there's no #2"
		),
		(&["proc", "value"][..], "invalid proc path 'value': expected /PATH[#N]."),
		(
			&["proc", "/value#x"][..],
			"invalid definition index 'x' in '/value#x': expected a non-negative integer."
		),
		(
			&["proc"][..],
			"missing required argument: /PATH[#N]. Run 'auxtools proc --help' for usage."
		),
		(
			&["disassemble", "/proc/nothing_called_this"][..],
			"the world has no proc /proc/nothing_called_this"
		)
	] {
		assert_eq!(fails(pid, args), said, "`{}`", args.join(" "));
	}
	println!("ok: proc metadata and reload status");

	let printed = run(pid, &["disassemble", "/datum/twice/act#1"], 0).out;
	let (first, second) = (line_of(code, "\t. = ..()"), line_of(code, "\treturn \"second\""));
	let expected = format!(
		"/datum/twice/act#1  attach_host.dm\nline {first}:\n     0  DbgFile \"attach_host.dm\"\n     2  DbgLine {first}\n     4  CallParent\n     5  SetVar \
		 dot\nline {second}:\n     7  DbgLine {second}\n     9  PushVal \"second\"\n    12  Ret\n    13  End\n"
	);
	assert_eq!(printed, expected);
	println!("ok: bytecode offsets and source lines");
}

/// wait says where each of its breakpoints went, and fails before waiting
/// on a place the world doesn't have
fn where_breakpoints_bound(pid: &str, code: &Path) {
	let declared = line_of(code, "/proc/value()");
	let ran = run(
		pid,
		&["wait", "pause", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "60"],
		0
	);
	assert!(
		ran.out.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"unexpected wait output: {}",
		ran.out
	);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint set at /value  attach_host.dm:{} (line {} has no code; this is the next line that has)\n",
			declared + 1,
			declared
		)
	);
	run(pid, &["continue"], 0);
	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/fail_in",
			"--break",
			"/datum/twice/act#1",
			"--timeout",
			"1"
		],
		2
	);
	assert!(ran.out.is_empty(), "a wait that timed out printed {}", ran.out);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint set at /fail_in  attach_host.dm:{}\nbreakpoint set at /datum/twice/act#1  attach_host.dm:{}\nerror: timed out after 1 \
			 second waiting for the world to pause.\n",
			line_of(code, "\tvar/got = fail_deeper(n)"),
			line_of(code, "\t. = ..()")
		)
	);
	println!("ok: resolved wait breakpoint locations");

	let last = line_of(code, "\treturn \"second\"");
	let nowhere = [
		(
			"nowhere.dm:3",
			"cannot set a breakpoint at nowhere.dm:3: the world has no code from a file named nowhere.dm.".to_owned()
		),
		(
			"attach_host.dm:9999",
			format!(
				"cannot set a breakpoint at attach_host.dm:9999: attach_host.dm has no code after line {}.",
				last
			)
		),
		(
			"/proc/nothing_called_this",
			"cannot set a breakpoint at /proc/nothing_called_this: the world has no proc /proc/nothing_called_this".to_owned()
		),
		(
			"/value#2",
			"cannot set a breakpoint at /value#2: /value has one definition, #0: there's no #2".to_owned()
		)
	];
	// With no time limit: it would wait for ever if it didn't fail first
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "pause", "--break", place]), said);
	}
	run(pid, &["pause"], 0);
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "pause", "--break", place]), said);
	}
	run(pid, &["continue"], 0);
	println!("ok: invalid wait target diagnostics");
}

/// The instruction a pause after a step of one is on: its offset and text,
/// as `dis` has them for the frame's proc
fn instruction_at(printed: &str, proc: &str, pid: &str) -> u32 {
	let mut lines = printed.lines();
	assert_eq!(
		lines.next(),
		Some(format!("pid {}: paused after a step", pid).as_str()),
		"unexpected step output: {}",
		printed
	);
	let top = lines.next().unwrap_or_default();
	assert!(top.starts_with(&format!("#0 {}  ", proc)), "unexpected step output: {}", printed);
	let (offset, text) = lines
		.next()
		.and_then(|line| line.strip_prefix("   instruction "))
		.and_then(|line| line.split_once(": "))
		.unwrap_or_else(|| panic!("missing instruction in step output: {}", printed));
	let offset: u32 = offset.parse().unwrap();
	let listing = run(pid, &["disassemble", proc], 0).out;
	assert!(
		listing.contains(&format!("\n{:>6}  {}\n", offset, text)),
		"{} {} isn't in\n{}",
		offset,
		text,
		listing
	);
	offset
}

/// Steps of one instruction, over and into
fn instruction_steps(pid: &str, dir: &Path) {
	call_soon(dir);
	run(pid, &["wait", "pause", "--break", "/proc/fail_in", "--timeout", "60"], 0);
	let printed = run(pid, &["step-over", "--instruction"], 0).out;
	let mut last = instruction_at(&printed, "/fail_in", pid);
	// A breakpoint on a proc is on the instruction after its first DbgLine,
	// and this is the one after that
	let offsets: Vec<u32> = run(pid, &["disassemble", "/fail_in"], 0)
		.out
		.lines()
		.filter_map(|line| {
			line.trim_start()
				.split_once("  ")
				.and_then(|(offset, text)| Some((offset.parse().ok()?, text)))
		})
		.skip_while(|(_, text)| !text.starts_with("DbgLine"))
		.map(|(offset, _)| offset)
		.collect();
	assert_eq!(last, offsets[2], "step-over --instruction went to {} of {:?}", last, offsets);
	println!("ok: instruction Step Over");
	// One at a time, into fail_deeper() when its call comes
	let mut printed = run(pid, &["step-in", "--instruction"], 0).out;
	for _ in 0..10 {
		if printed.lines().nth(1).is_some_and(|line| line.starts_with("#0 /fail_deeper  ")) {
			break;
		}
		let offset = instruction_at(&printed, "/fail_in", pid);
		assert!(offset > last, "a step went from {} to {}", last, offset);
		last = offset;
		printed = run(pid, &["step-in", "--instruction"], 0).out;
	}
	instruction_at(&printed, "/fail_deeper", pid);
	// A step of a line still says nothing of instructions
	let printed = run(pid, &["step-over"], 0).out;
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /fail_deeper  ", pid)) && !printed.contains("instruction"),
		"step-over said {}",
		printed
	);
	run(pid, &["continue"], 0);
	println!("ok: instruction Step Into");
}

/// One JSON document on stdout, and nothing on stderr
fn json_of(pid: &str, args: &[&str]) -> Value {
	let ran = run(pid, args, 0);
	assert!(ran.err.is_empty(), "unexpected stderr from `{}`: {}", args.join(" "), ran.err);
	serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("`{}` printed {}: {}", args.join(" "), ran.out, e))
}

/// --json, on a running world and a paused one
fn as_json(pid: &str, dir: &Path) {
	let eval = |expression: &str, more: &[&str]| {
		let args: Vec<&str> = ["eval", "--json", expression].iter().chain(more).copied().collect();
		json_of(pid, &args)
	};
	let number: u64 = pid.parse().unwrap();
	let status = json_of(pid, &["status", "--json"]);
	for (key, value) in [
		("pid", json!(number)),
		("paused", json!(false)),
		("reason", json!(null)),
		("message", json!(null)),
		("caught", json!(null)),
		("changes", json!([])),
		("stack", json!([])),
		("instruction", json!(null))
	] {
		assert_eq!(status[key], value, "{} of {}", key, status);
	}
	assert!(
		status["uptime_ms"].as_u64().is_some_and(|up| up > 0) && status["world_time"].is_number() && status["loads"] == 1,
		"{}",
		status
	);
	assert_eq!(status["last_reload"]["kind"], "reload", "{}", status);
	assert_eq!(status.as_object().unwrap().len(), 12, "{}", status);
	assert_eq!(
		eval("list(1, \"two\", null, list(3, list(4)))", &[]),
		json!([1, "two", null, [3, { "list": true, "len": 1 }]])
	);
	assert_eq!(eval("list(\"a\" = 1, \"b\")", &[]), json!({ "a": 1, "b": null }));
	assert_eq!(eval("list(1, 2)", &["--depth", "0"]), json!({ "list": true, "len": 2 }));
	assert_eq!(eval("0.1", &[]).to_string(), "0.1");
	assert_eq!(eval("\"text\"", &[]), json!("text"));
	assert_eq!(eval("/datum/twice", &[]), json!({ "path": "/datum/twice" }));
	let made = eval("new /datum/twice", &[]);
	assert!(
		made["ref"].as_str().is_some_and(|r| r.starts_with("[0x")) && made["type"] == "/datum/twice" && made["name"].is_null(),
		"{}",
		made
	);
	assert!(
		made["vars"]["type"] == json!({ "path": "/datum/twice" }) && made["vars"].get("vars").is_none(),
		"{}",
		made
	);
	let shut = eval("new /datum/twice", &["--depth", "1"]);
	assert!(shut.get("vars").is_none(), "{}", shut);
	let in_list = eval("list(new /datum/twice)", &[]);
	assert!(in_list[0]["type"] == "/datum/twice" && in_list[0].get("vars").is_none(), "{}", in_list);
	assert_eq!(fails(pid, &["eval", "--json", "1/0"]), "runtime error: Division by zero");
	assert_eq!(fails(pid, &["eval", "--depth", "1", "1"]), "--depth requires --json.");
	let printed = json_of(pid, &["proc", "--json", "/datum/twice/act#1"]);
	assert!(
		printed["proc"] == "/datum/twice/act#1" && printed["definitions"] == 2 && printed["reloaded"].is_null(),
		"{}",
		printed
	);
	assert_eq!(json_of(pid, &["proc", "--json", "/value"])["reloaded"], "code");
	let printed = json_of(pid, &["disassemble", "--json", "/value"]);
	assert!(
		printed["instructions"][0]["text"].as_str().is_some_and(|t| t.starts_with("DbgFile")),
		"{}",
		printed
	);
	println!("ok: recursive JSON rendering");

	call_soon(dir);
	// (where the breakpoint went is on stderr as well as in the JSON)
	let ran = run(pid, &["wait", "pause", "--json", "--break", "/proc/fail_in", "--timeout", "60"], 0);
	assert!(
		ran.err.starts_with("breakpoint set at /fail_in  attach_host.dm:") && ran.err.lines().count() == 1,
		"unexpected wait output: {}",
		ran.err
	);
	let paused: Value = serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("wait pause --json printed {}: {}", ran.out, e));
	assert!(
		paused["paused"] == true && paused["reason"] == "breakpoint" && paused["already"] == false && paused["pid"] == number,
		"{}",
		paused
	);
	assert_eq!(paused.as_object().unwrap().len(), 11, "{}", paused);
	assert_eq!(paused["breakpoints"][0]["proc"], "/fail_in");
	assert_eq!(paused["breakpoints"][0]["place"], "/proc/fail_in");
	assert_eq!(paused["stack"][0]["proc"], "/fail_in");
	assert!(paused["instruction"].is_null() && paused["changes"] == json!([]), "{}", paused);
	let again = json_of(pid, &["wait", "pause", "--json"]);
	assert!(again["already"] == true && again["stack"] == paused["stack"], "{}", again);
	let status = json_of(pid, &["status", "--json"]);
	assert!(
		status["reason"] == "breakpoint" && status["stack"] == paused["stack"] && status["uptime_ms"].is_number(),
		"{}",
		status
	);
	let held = json_of(pid, &["pause", "--json"]);
	assert!(
		held["paused"] == true && held["reason"] == "breakpoint" && held.as_object().unwrap().len() == 8,
		"{}",
		held
	);
	let stack = json_of(pid, &["stack", "--json"]);
	assert!(
		stack[0]["proc"] == "/fail_in" && stack[0]["line"].is_number() && stack[0]["offset"].is_number(),
		"{}",
		stack
	);
	let all = json_of(pid, &["stack", "--json", "--all"]);
	assert!(all[0]["frames"] == stack && all[0]["sleeping"] == false, "{}", all);
	let vars = json_of(pid, &["vars", "--json", "--globals"]);
	assert!(
		vars["arguments"]["n"] == 1 && vars["locals"]["got"].is_null() && vars["globals"]["rounds"].is_number(),
		"{}",
		vars
	);
	assert_eq!(eval("n + 1", &[]), json!(2));
	let stepped = json_of(pid, &["step-over", "--instruction", "--json"]);
	assert!(
		stepped["reason"] == "step" && stepped["instruction"]["text"].is_string() && stepped["pid"] == number,
		"{}",
		stepped
	);
	run(pid, &["continue"], 0);
	println!("ok: JSON results while paused");
}

/// eval --src: an object's vars and procs by their bare names, with no frame
fn on_an_object(pid: &str) {
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert!(reference.starts_with("[0x") && reference.ends_with(']'), "ref(held) gave {}", reference);
	let bare = reference.trim_start_matches('[').trim_end_matches(']').to_owned();
	for object in [&reference, &bare] {
		assert_eq!(run(pid, &["eval", "--src", object, "count + double(2) + src.count"], 0).out, "10\n");
	}
	assert_eq!(
		json_of(pid, &["eval", "--src", &reference, "--json", "list(count, type)"]),
		json!([3, { "path": "/datum/held" }])
	);
	// Paused, the same, and not in the paused frame
	run(pid, &["pause"], 0);
	assert_eq!(run(pid, &["eval", "--src", &bare, "double(count)"], 0).out, "6\n");
	run(pid, &["continue"], 0);
	println!("ok: object-scoped evaluation");

	// One that's gone: the datum is deleted once nothing holds it
	let gone = run(pid, &["eval", "ref(new /datum/held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(fails(pid, &["eval", "--src", &gone, "count"]), format!("object not found: {}", gone));
	assert_eq!(
		fails(pid, &["eval", "--src", "held", "count"]),
		"invalid object reference held; expected a ref() value such as [0x2000001]"
	);
	assert_eq!(
		fails(pid, &["eval", "--src", &bare, "--frame", "0", "count"]),
		"--frame and --src cannot be used together."
	);
	assert!(fails(pid, &["eval", "--src", &bare, "nothing_called_this"]).starts_with("undefined var: nothing_called_this"));
	println!("ok: invalid object reference diagnostics");
}

/// What `eval` gives, once it gives `wanted`: the world is given the code's
/// macros a moment after a reload
fn eval_becomes(pid: &str, expression: &str, wanted: &str) {
	let mut got = String::new();
	for _ in 0..100 {
		got = auxtools(pid, &["eval", expression], None).out;
		if got.trim() == wanted {
			return;
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	panic!("eval {} gave {}, not {}", expression, got, wanted);
}

/// The code's #defines, in every kind of expression
fn macros(pid: &str, code: &Path) {
	assert_eq!(run(pid, &["eval", "DOUBLE_IT(ANSWER) + 1"], 0).out, "85\n");
	assert_eq!(run(pid, &["eval", "\"[ANSWER]!\""], 0).out, "\"42!\"\n");
	assert!(fails(pid, &["eval", "GONE"]).starts_with("undefined var: GONE"));
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(run(pid, &["eval", "--src", &reference, "DOUBLE_IT(count)"], 0).out, "6\n");
	println!("ok: source macro expansion and undefinition");

	// In a breakpoint's condition and its message, and in a paused frame
	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/tick",
			"--if",
			"n == DOUBLE_IT(1) && ANSWER == 42",
			"--timeout",
			"60"
		],
		0
	);
	assert!(
		ran.out.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /tick", pid)),
		"unexpected wait output: {}",
		ran.out
	);
	assert_eq!(run(pid, &["eval", "DOUBLE_IT(n)"], 0).out, "4\n");
	run(pid, &["continue"], 0);
	let ran = run(
		pid,
		&[
			"trace",
			"/proc/tick",
			"--if",
			"n == 1",
			"--log",
			"doubled: {DOUBLE_IT(n)}",
			"--count",
			"1"
		],
		0
	);
	assert!(ran.out.contains("doubled: 2\n"), "unexpected trace output: {}", ran.out);
	println!("ok: macros in conditions, logpoints and frame evaluation");

	// A reload that changes them gives the world the new ones
	let source = std::fs::read_to_string(code).unwrap();
	std::fs::write(code, source.replace("#define ANSWER 42", "#define ANSWER 43\n#define GONE 2")).unwrap();
	eval_becomes(pid, "ANSWER + GONE", "45");
	std::fs::write(code, &source).unwrap();
	eval_becomes(pid, "ANSWER", "42");
	assert!(fails(pid, &["eval", "GONE"]).starts_with("undefined var: GONE"));
	println!("ok: macro updates after reload");
}

/// eval --spawn: the world's own code, on its next tick
fn spawned(pid: &str, code: &Path) {
	let number = run(pid, &["eval", "--spawn", "slow_double(4)"], 0).out;
	let number: u64 = number
		.trim()
		.strip_prefix("spawn ")
		.and_then(|number| number.parse().ok())
		.unwrap_or_else(|| panic!("eval --spawn printed {}", number));
	assert_eq!(json_of(pid, &["eval", "--spawn", "--json", "1"]), json!({ "spawn": number + 1 }));
	let mut entry = Value::Null;
	for _ in 0..100 {
		let printed = run(pid, &["log", "--spawned", "--last", "2", "--json"], 0).out;
		entry = printed
			.lines()
			.filter_map(|line| serde_json::from_str::<Value>(line).ok())
			.find(|entry| entry["spawn"] == number)
			.unwrap_or_default();
		if entry["spawn"] == number {
			break;
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	assert!(
		entry["spawn"] == number && entry["value"] == 8 && entry["expression"] == "slow_double(4)",
		"{}",
		entry
	);
	let printed = run(pid, &["log", "--spawned", "--last", "2"], 0).out;
	assert!(
		printed.contains(&format!(" spawn {}: slow_double(4) returned 8\n", number)),
		"log said {}",
		printed
	);
	println!("ok: spawn ID and journal result");

	assert_eq!(run(pid, &["eval", "--spawn", "--wait", "slow_double(5)"], 0).out, "10\n");
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(
		run(
			pid,
			&["eval", "--spawn", "--wait", "--src", &reference, "double(count) + DOUBLE_IT(1)"],
			0
		)
		.out,
		"8\n"
	);
	let both = json_of(pid, &["eval", "--spawn", "--wait", "--json", "list(slow_double(1), held)"]);
	assert!(
		both[0] == 2 && both[1]["type"] == "/datum/held" && both[1].get("vars").is_none(),
		"{}",
		both
	);
	println!("ok: wait for spawned evaluation result");

	// A proc's body, with vars of its own, which can sleep here
	let body = "var/n = slow_double(2)\nsleep(1)\nvar/list/both = list(n, n + 1)\nboth[2]";
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", body], 0).out, "5\n");
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", "slow_double(5)"], 0).out, "10\n");
	println!("ok: spawned body variables survive sleep");

	// A runtime error, before a sleep and after one
	let said = fails(pid, &["eval", "--spawn", "--wait", "fail_in(3)"]);
	assert!(
		said.starts_with("runtime error: ") && said.contains("\n  in /fail_deeper  ") && said.ends_with("\n  called by the expression"),
		"unexpected eval output: {}",
		said
	);
	let said = fails(pid, &["eval", "--spawn", "--wait", "slow_fail()"]);
	let lines: Vec<&str> = said.lines().collect();
	assert!(
		lines.len() == 4 && lines[0].starts_with("runtime error: "),
		"unexpected eval output: {}",
		said
	);
	assert_eq!(lines[1], format!("  in /fail_deeper  attach_host.dm:{}", line_of(code, "\treturn L[n]")));
	assert_eq!(
		lines[2],
		format!("  called by /slow_fail  attach_host.dm:{}", line_of(code, "\treturn fail_deeper(5)"))
	);
	let printed = run(pid, &["log", "--spawned", "--last", "1"], 0).out;
	assert!(
		printed.contains(": slow_fail() failed: runtime error: ") && printed.contains("\n  in /fail_deeper  "),
		"log said {}",
		printed
	);
	println!("ok: spawned runtime error stacks");

	// A breakpoint in what it calls stops the world
	let waiting = crate::journal::Background::start(pid.parse().unwrap(), &[
		"wait",
		"pause",
		"--break",
		"/proc/slow_double",
		"--timeout",
		"60"
	]);
	// (it says where the breakpoint went on stderr, which is read once it's
	// done)
	std::thread::sleep(std::time::Duration::from_secs(2));
	run(pid, &["eval", "--spawn", "slow_double(6)"], 0);
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.contains(&format!("pid {}: paused at a breakpoint\n#0 /slow_double  ", pid)),
		"wait gave {}: {}",
		exit,
		printed
	);
	let stack = run(pid, &["stack"], 0).out;
	assert!(
		stack.lines().nth(1).is_some_and(|line| line.starts_with("#1 /auxtools_spawn_stub")),
		"stack said {}",
		stack
	);
	assert_eq!(run(pid, &["eval", "n"], 0).out, "6\n");
	run(pid, &["continue"], 0);
	println!("ok: breakpoints in spawned calls");

	let ran = run(pid, &["eval", "--spawn", "--wait", "--timeout", "0.5", "slow_double(1, 50)"], 2);
	assert!(
		ran.out.is_empty()
			&& ran.err.starts_with("error: timed out after 0.5 seconds waiting for spawn ")
			&& ran.err.trim_end().ends_with(" to finish. It keeps running."),
		"unexpected eval output: {}{}",
		ran.out,
		ran.err
	);
	assert!(fails(pid, &["eval", "--spawn", "1 +"]).starts_with("parser error: "));
	assert!(fails(pid, &["eval", "--spawn", "--src", "[0x2100ffff]", "1"]).starts_with("object not found: [0x2100ffff]"));
	assert_eq!(fails(pid, &["eval", "--wait", "1"]), "--wait requires --spawn.");
	assert_eq!(fails(pid, &["eval", "--spawn", "--timeout", "1", "1"]), "--timeout requires --wait.");
	assert_eq!(
		fails(pid, &["eval", "--spawn", "--frame", "0", "1"]),
		"--frame and --spawn cannot be used together."
	);
	println!("ok: spawn timeout and rejection diagnostics");
}

/// A step that ends with nothing to pause in is done, with the world running;
/// one that isn't done by its --timeout says so, and what the world is doing
fn steps_that_end_elsewhere(pid: &str, dir: &Path) {
	let number: u64 = pid.parse().unwrap();
	// fail_in() is called from a spawn() with nothing after it: a step from
	// there leaves the code that was running
	call_soon(dir);
	run(pid, &["wait", "pause", "--break", "/proc/fail_in", "--timeout", "60"], 0);
	let out = run(pid, &["step-out", "--timeout", "30"], 0).out;
	assert!(
		out.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped  ", pid)),
		"step-out said {}",
		out
	);
	let ran = run(pid, &["step-over", "--timeout", "30", "--json"], 0);
	let state: Value = serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("step-over --json printed {}: {}", ran.out, e));
	assert!(
		state["pid"] == number && state["paused"] == false && state["reason"].is_null() && state["stack"] == json!([]),
		"{}",
		state
	);
	assert_eq!(ran.err, "step completed with the world running: no frame remained to pause in.\n");
	assert_eq!(json_of(pid, &["status", "--json"])["paused"], false);
	println!("ok: step exhaustion succeeds with world running");

	// A step over a sleep isn't over until the proc wakes
	let waiting = crate::journal::Background::start(number, &["wait", "pause", "--break", "/proc/slow_double", "--timeout", "60"]);
	std::thread::sleep(std::time::Duration::from_secs(2));
	run(pid, &["eval", "--spawn", "slow_double(6, 40)"], 0);
	let (exit, printed) = waiting.finish();
	assert_eq!(exit, 0, "unexpected wait output: {}", printed);
	// (the breakpoint is before the line that sleeps: a step gets there)
	let on_sleep = run(pid, &["step-over", "--timeout", "30", "--json"], 0).out;
	assert!(
		on_sleep.contains("\"reason\":\"step\"") && on_sleep.contains("\"proc\":\"/slow_double\""),
		"step-over said {}",
		on_sleep
	);
	let started = std::time::Instant::now();
	let ran = run(pid, &["step-over", "--timeout", "1", "--json"], 2);
	assert!(
		started.elapsed() < std::time::Duration::from_secs(4),
		"a step with --timeout 1 took {:?}",
		started.elapsed()
	);
	let state: Value = serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("step-over --json printed {}: {}", ran.out, e));
	assert!(state["pid"] == number && state["paused"] == false, "{}", state);
	assert!(
		ran.err.starts_with("error: timed out after 1 second waiting for the step to complete."),
		"step-over said {}",
		ran.err
	);
	// In text, the same
	let waiting = crate::journal::Background::start(number, &["wait", "pause", "--break", "/proc/slow_double", "--timeout", "60"]);
	std::thread::sleep(std::time::Duration::from_secs(2));
	run(pid, &["eval", "--spawn", "slow_double(7, 40)"], 0);
	let (exit, printed) = waiting.finish();
	assert_eq!(exit, 0, "unexpected wait output: {}", printed);
	run(pid, &["step-over", "--timeout", "30"], 0);
	let ran = run(pid, &["step-over", "--timeout", "0.5"], 2);
	assert_eq!(ran.out, format!("pid {}: running\n", pid));
	assert!(
		ran.err
			.starts_with("error: timed out after 0.5 seconds waiting for the step to complete."),
		"step-over said {}",
		ran.err
	);
	// The steps left behind don't pause the world when their procs wake
	std::thread::sleep(std::time::Duration::from_secs(5));
	assert_eq!(json_of(pid, &["status", "--json"])["paused"], false);
	println!("ok: step timeout preserves execution state");
}
