//! Edits the source of a running world and hot reloads it the way a watcher
//! would: dmasm compiles what changed, and the debug server swaps it in.

// test_runner already knows how to find BYOND, debug_test has a client, and
// the debug server's message types aren't exported from its cdylib. All of
// them are pulled in as source.
#[allow(dead_code)]
#[path = "../../debug_test/src/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{
	collections::HashSet,
	path::{Path, PathBuf},
	process::Command,
	time::{Duration, Instant}
};

use client::{Client, World};
use dmasm::reload::{diff, Changes, ParseOptions, Snapshot};
use paths::ByondCommand;
use server_types::*;

/// The world's source, in a copy the test can edit, and how it was last
/// parsed.
struct Source {
	dir: PathBuf,
	options: ParseOptions,
	snapshot: Option<Snapshot>
}

impl Source {
	fn copy_host() -> Source {
		let dir = std::env::temp_dir().join(format!("hot_reload_test_host-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();

		let source = Source {
			dir,
			options: ParseOptions::default(),
			snapshot: None
		};
		source.copy_files();
		source
	}

	fn copy_files(&self) {
		let host = std::env::current_dir().unwrap().join("tests/hot_reload_test_host");
		for file in ["hot_reload_test_host.dme", "main.dm", "reload.dm"] {
			std::fs::copy(host.join(file), self.dir.join(file)).unwrap();
		}
	}

	/// Puts the source back as the world was built from it, which is what a
	/// rebooted world runs, and parses it
	fn restore(&mut self) {
		self.copy_files();
		self.snapshot = None;
		self.parse();
	}

	fn dme(&self) -> PathBuf {
		self.dir.join("hot_reload_test_host.dme")
	}

	fn text(&self, file: &str) -> String {
		std::fs::read_to_string(self.dir.join(file)).unwrap()
	}

	/// The line `needle` is on, counting from 1.
	fn line_of(&self, file: &str, needle: &str) -> u32 {
		let text = self.text(file);
		let at = text.find(needle).unwrap_or_else(|| panic!("{:?} isn't in {}", needle, file));
		text[..at].matches('\n').count() as u32 + 1
	}

	fn parse(&mut self) {
		self.snapshot = Some(Snapshot::parse(&self.dme(), &self.options).unwrap());
	}

	/// Replaces `from` (which has to be there exactly once) with `to`, and
	/// returns what changed since the last parse.
	fn edit(&mut self, file: &str, from: &str, to: &str) -> Changes {
		self.edit_many(file, &[(from, to)])
	}

	/// Makes each replacement, as for `edit`, and returns what they changed
	/// together.
	fn edit_many(&mut self, file: &str, edits: &[(&str, &str)]) -> Changes {
		let mut text = self.text(file);
		for (from, to) in edits {
			assert_eq!(text.matches(from).count(), 1, "{:?} should be in {} once", from, file);
			text = text.replace(from, to);
		}
		std::fs::write(self.dir.join(file), text).unwrap();

		let old = self.snapshot.take().expect("parsed before editing");
		self.parse();
		let edited: HashSet<PathBuf> = [Path::new(file).to_owned()].into_iter().collect();
		let changes = diff(&old, self.snapshot.as_ref().unwrap(), Some(&edited));
		assert!(changes.refused.is_empty(), "the edit was refused: {:?}", changes.refused);
		changes
	}
}

impl Drop for Source {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.dir);
	}
}

fn proc_ref(key: &dmasm::reload::ProcKey) -> ProcRef {
	ProcRef {
		path: key.path.clone(),
		override_id: key.override_id
	}
}

fn proc_code(r: &dmasm::reload::ProcReload) -> HotReloadProc {
	HotReloadProc {
		proc: proc_ref(&r.proc),
		nodes: r.nodes.clone(),
		locals: r.locals.clone(),
		params: r.params.clone(),
		settings: r.settings.clone(),
		statics: r.statics.clone()
	}
}

fn reload_request(changes: &Changes) -> Request {
	Request::HotReload(HotReloadRequest {
		new_types: changes.new_types.clone(),
		procs: changes.reloads.iter().map(proc_code).collect(),
		settings: changes.settings.clone(),
		new_procs: changes
			.new_procs
			.iter()
			.map(|p| HotReloadNewProc {
				type_path: p.type_path.clone(),
				name: p.name.clone(),
				declared: p.declared,
				verb: p.verb,
				code: proc_code(&p.code)
			})
			.collect(),
		removed_procs: changes.removed_procs.clone(),
		builtin_defaults: changes.builtin_defaults.clone(),
		new_vars: changes.new_vars.clone(),
		redeclared: changes.redeclared.clone(),
		init_procs: changes
			.init_procs
			.iter()
			.map(|i| HotReloadInitProc {
				type_path: i.type_path.clone(),
				code: i.code.as_ref().map(proc_code)
			})
			.collect(),
		var_defaults: changes
			.var_defaults
			.iter()
			.map(|d| HotReloadVarDefault {
				var: d.var.clone(),
				value: d.value.clone(),
				types: d.types.clone()
			})
			.collect(),
		line_moves: changes
			.line_moves
			.iter()
			.map(|m| HotReloadLines {
				proc: proc_ref(&m.proc),
				lines: m.lines.clone()
			})
			.collect()
	})
}

fn expect_reload_result(response: Response) -> HotReloadResult {
	match response {
		Response::HotReload(result) => result,
		other => panic!("expected the result of a hot reload, got {:?}", other)
	}
}

fn reload(client: &mut Client, changes: &Changes) -> HotReloadResult {
	let result = expect_reload_result(client.request(reload_request(changes)));
	assert!(result.applied, "reload rejected: {:?}", result.errors);
	result
}

fn eval(client: &mut Client, command: &str) -> String {
	match client.request(Request::Eval {
		frame_id: None,
		command: command.to_owned(),
		context: None
	}) {
		Response::Eval(Ok(eval)) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// Whether an expression goes to sleep, which is how one tells that a proc
/// it calls waits for another's sleep
fn sleeps(client: &mut Client, command: &str) -> bool {
	match client.request(Request::Eval {
		frame_id: None,
		command: command.to_owned(),
		context: None
	}) {
		Response::Eval(Ok(_)) => false,
		Response::Eval(Err(why)) if why.starts_with("evaluation aborted because the code slept") => true,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// Evaluates `command` until it gives `expected`.
fn wait_for(client: &mut Client, command: &str, expected: &str) {
	let start = Instant::now();
	loop {
		let value = eval(client, command);
		if value == expected {
			return;
		}
		assert!(
			start.elapsed() < Duration::from_secs(10),
			"{} is still {}, not {}",
			command,
			value,
			expected
		);
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// The line the top frame of the paused world is on.
fn paused_line(client: &mut Client) -> (String, Option<u32>) {
	match client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(0),
		count: Some(1)
	}) {
		Response::StackFrames { frames, .. } => {
			let frame = frames.into_iter().next().expect("paused with no frames");
			(frame.instruction.proc.path, frame.line)
		}
		other => panic!("expected StackFrames, got {:?}", other)
	}
}

fn offset_of(client: &mut Client, proc: &str, line: u32) -> u32 {
	match client.request(Request::Offset {
		proc: ProcRef {
			path: proc.to_owned(),
			override_id: 0
		},
		line
	}) {
		Response::Offset { offset: Some(offset) } => offset,
		other => panic!("no offset for line {} of {}: {:?}", line, proc, other)
	}
}

fn main() {
	let mut source = Source::copy_host();

	let res = Command::new(paths::find_dm()).with_byond_paths().arg(source.dme()).status().unwrap();
	assert!(res.success(), "dreammaker build failed");

	let world = World::start(&source.dir.join("hot_reload_test_host.dmb"), &[]);

	// A debugger and a watcher, attached at the same time. The watcher sends
	// the reloads, except where the debugger's own connection is tested.
	let mut client = world.connect();
	// Each client says whether it wants runtime errors
	client.send(Request::CatchRuntimes {
		uncaught: Some(Default::default()),
		caught: None
	});
	let mut watcher = world.connect_watcher();

	// Parsed as DreamMaker does for the BYOND version the world runs on
	let (version, build) = match watcher.request(Request::ByondVersion) {
		Response::ByondVersion { version, build } => (version, build),
		other => panic!("expected ByondVersion, got {:?}", other)
	};
	println!("ok: BYOND {}.{}", version, build);
	source.options.byond = Some((version, build));
	source.parse();

	// New code for a proc
	assert_eq!(eval(&mut client, "answer()"), "1");
	let changes = source.edit("reload.dm", "/proc/answer()\n\treturn 1", "/proc/answer()\n\treturn 2");
	assert_eq!(changes.reloads.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "answer()"), "2");
	println!("ok: proc bytecode replacement");

	// A call that was sleeping when its proc changed finishes on the code it
	// started with. New calls run the new code.
	assert_eq!(eval(&mut client, "sleeper_state"), "\"sleeping\"");
	let changes = source.edit(
		"reload.dm",
		"sleeper_state = \"old code finished\"",
		"sleeper_state = \"new code finished\""
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "release_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"old code finished\"");
	eval(&mut client, "start_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"new code finished\"");
	println!("ok: active calls retain old bytecode");

	// Recursion into the new code
	assert_eq!(eval(&mut client, "factorial(4)"), "24");
	let changes = source.edit("reload.dm", "return n * factorial(n - 1)", "return n * factorial(n - 1) + 1");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "factorial(4)"), "41");
	println!("ok: recursive proc reload");

	// More locals than the proc's frames had room for
	assert_eq!(eval(&mut client, "few_locals()"), "1");
	let changes = source.edit(
		"reload.dm",
		"/proc/few_locals()\n\tvar/a = 1\n\treturn a",
		"/proc/few_locals()\n\tvar/a = 1\n\tvar/b = 2\n\tvar/c = 3\n\tvar/d = 4\n\tvar/e = 5\n\tvar/f = 6\n\treturn a + b + c + d + e + f"
	);
	let few_locals = changes.reloads.iter().find(|r| r.proc.path == "/few_locals").unwrap();
	assert_eq!(few_locals.locals.len(), 6);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "few_locals()"), "21");
	println!("ok: expanded locals table");

	// A static local keeps its value
	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "2");
	let changes = source.edit("reload.dm", "\tcount++", "\tcount += 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), "12");
	println!("ok: static local value preservation");

	// A breakpoint stays on its line, and the watcher can reload while the
	// debugger has the world paused.: with new code it stays on the same line,
	// and when the proc moves it moves with it
	let line = source.line_of("reload.dm", "\tx += 1");
	let old_offset = offset_of(&mut client, "/breakpoint_target", line);
	let response = client.request(Request::BreakpointSet {
		instruction: InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset: old_offset
		},
		condition: None,
		hit: None,
		log: None
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));

	// Two more lines in answer() move everything below it, and the world is
	// paused while it happens
	let changes = source.edit(
		"reload.dm",
		"/proc/answer()\n\treturn 2",
		"/proc/answer()\n\tvar/unused = 1\n\tunused++\n\treturn 3"
	);
	assert!(changes.line_moves.iter().any(|m| m.proc.path == "/breakpoint_target"));
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	// The paused frame's code is as it was, two lines further down the file
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 2)));
	client.resume();

	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 2)));
	assert_eq!(source.line_of("reload.dm", "\tx += 1"), line + 2);
	client.resume();
	assert_eq!(eval(&mut client, "answer()"), "3");
	println!("ok: breakpoint line remapping");

	// The debug-client still knows it by the offset it set it at
	let response = client.request(Request::BreakpointUnset {
		instruction: InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset: old_offset
		}
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	println!("ok: remove remapped breakpoint by original offset");

	// Two debuggers have a breakpoint on one line, and new code above it in
	// its proc moves it. Each knows it by its old offset until it sets it
	// anew: one that has, and then sets and takes away a breakpoint at the
	// old offset, means the new code that's there.
	let breakpoint = |client: &mut Client, offset: u32, set: bool| {
		let instruction = InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset
		};
		match client.request(if set {
			Request::BreakpointSet {
				instruction,
				condition: None,
				hit: None,
				log: None
			}
		} else {
			Request::BreakpointUnset { instruction }
		}) {
			Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}
			| Response::BreakpointUnset { success: true, .. } => {}
			other => panic!(
				"couldn't {} the breakpoint at {}: {:?}",
				if set { "set" } else { "take away" },
				offset,
				other
			)
		}
	};
	// A line of new code in place of a blank one keeps the breakpoint's
	// line where it is, and puts that line's code further on: the new
	// line's code is where the breakpoint was.
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx += 1", "\tvar/x = 1\n\n\tx += 1");
	reload(&mut watcher, &changes);
	let mut other = world.connect();
	let line = source.line_of("reload.dm", "\tx += 1");
	let old_offset = offset_of(&mut client, "/breakpoint_target", line);
	breakpoint(&mut client, old_offset, true);
	breakpoint(&mut other, old_offset, true);
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\n\tx += 1", "\tvar/x = 1\n\tx *= 1\n\tx += 1");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	let new_offset = offset_of(&mut client, "/breakpoint_target", line);
	assert_eq!(
		offset_of(&mut client, "/breakpoint_target", line - 1),
		old_offset,
		"the new line isn't where the old one was"
	);
	assert_ne!(new_offset, old_offset);
	// As the launcher does after a reload, then one at the old offset
	breakpoint(&mut client, old_offset, false);
	breakpoint(&mut client, new_offset, true);
	breakpoint(&mut client, old_offset, true);
	breakpoint(&mut client, old_offset, false);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(other.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));
	client.resume();
	// The other one still knows it by its old offset
	breakpoint(&mut other, old_offset, false);
	other.leave();
	breakpoint(&mut client, new_offset, false);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx *= 1\n\tx += 1", "\tvar/x = 1\n\tx += 1");
	reload(&mut watcher, &changes);
	println!("ok: breakpoint aliases are client-specific");

	// A runtime in new code reports the new code's line
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n\treturn 1",
		"/proc/runtime_target()\n\tvar/list/nothing\n\treturn nothing.len"
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "arm_runtime()");
	match client.expect_pause() {
		BreakpointReason::Runtime { message, .. } => assert!(message.contains("null.len"), "unexpected runtime: {}", message),
		other => panic!("expected a runtime, got {:?}", other)
	}
	let line = source.line_of("reload.dm", "\treturn nothing.len");
	assert_eq!(paused_line(&mut client), ("/runtime_target".to_owned(), Some(line)));
	client.resume();
	println!("ok: runtime error in reloaded code");

	// Resolve resource paths with different filename case, matching DreamMaker.
	std::fs::write(source.dir.join("Case_Probe.txt"), "found").unwrap();
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n\tvar/list/nothing\n\treturn nothing.len",
		"/proc/runtime_target()\n\treturn file2text('case_probe.txt')"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "runtime_target()"), "\"found\"");
	println!("ok: resource named in another case");
	// Cache logpoint expressions, invalidating for src type and newly added
	// globals.
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n",
		"/datum/cache_probe/proc/probe()\n\treturn 1\n\n/datum/cache_probe/sub\n\tvar/extra = 5\n\n/proc/runtime_target()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "/datum/cache_probe/proc/probe()") + 1;
	let instruction = InstructionRef {
		proc: ProcRef {
			path: "/datum/cache_probe/probe".to_owned(),
			override_id: 0
		},
		offset: offset_of(&mut client, "/datum/cache_probe/probe", line)
	};
	let response = client.request(Request::BreakpointSet {
		instruction,
		condition: None,
		hit: None,
		log: Some(vec![
			LogPart::Text("extra ".to_owned()),
			LogPart::Expr("extra".to_owned()),
			LogPart::Text(", global ".to_owned()),
			LogPart::Expr("cache_global".to_owned()),
		])
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"{:?}",
		response
	);
	let mut logged = |client: &mut Client, object: &str| {
		eval(client, &format!("spawn(0) (new {}).probe(); 1", object));
		match client.recv() {
			Response::Log { message, .. } => message,
			other => panic!("expected a log message, got {:?}", other)
		}
	};
	let sub = logged(&mut client, "/datum/cache_probe/sub");
	assert!(sub.starts_with("extra 5, global {cache_global: "), "{}", sub);
	let base = logged(&mut client, "/datum/cache_probe");
	assert!(base.starts_with("extra {extra: "), "{}", base);
	let changes = source.edit(
		"reload.dm",
		"/datum/cache_probe/sub\n",
		"var/cache_global = 7\n\n/datum/cache_probe/sub\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(logged(&mut client, "/datum/cache_probe/sub"), "extra 5, global 7");
	println!("ok: logpoint expressions compiled once, for each type of src and reload");

	// A proc auxtools hooked never runs its bytecode
	let changes = source.edit(
		"main.dm",
		"/proc/auxtools_launched_world_del()\n\tCRASH(\"auxtools not loaded\")",
		"/proc/auxtools_launched_world_del()\n\tCRASH(\"auxtools still not loaded\")"
	);
	let result = reload(&mut watcher, &changes);
	assert!(
		result.warnings.iter().any(|w| w.contains("hooked")),
		"no warning about the hook: {:?}",
		result.warnings
	);
	println!("ok: hooked-proc warning");

	// The debugger's own connection takes reloads too. Requests sent back to
	// back, the way VS Code sends them.
	let changes = source.edit("reload.dm", "\treturn 3", "\treturn 4");
	client.send(reload_request(&changes));
	client.send(Request::Eval {
		frame_id: None,
		command: "answer()".to_owned(),
		context: None
	});
	assert!(expect_reload_result(client.recv()).applied);
	match client.recv() {
		Response::Eval(Ok(eval)) => assert_eq!(eval.value, "4"),
		other => panic!("expected the result of answer(), got {:?}", other)
	}
	println!("ok: ordered reload and evaluation");

	// A new global proc, called from a changed one
	let changes = source.edit(
		"reload.dm",
		"/proc/uses_helper()\n\treturn 0",
		"/proc/uses_helper()\n\treturn helper() + 1\n\n/proc/helper()\n\treturn 7"
	);
	assert_eq!(changes.new_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "uses_helper()"), "8");
	assert_eq!(eval(&mut client, "helper()"), "7");
	// With its lines, as the world was built with DEBUG
	let response = client.request(Request::Offset {
		proc: ProcRef {
			path: "/helper".to_owned(),
			override_id: 0
		},
		line: source.line_of("reload.dm", "\treturn 7")
	});
	assert!(matches!(response, Response::Offset { offset: Some(_) }), "{:?}", response);
	println!("ok: new global proc");

	// A new proc on a type: existing instances of it and its subtypes have it
	let changes = source.edit(
		"reload.dm",
		"/datum/animal/proc/speak()\n\treturn sound",
		"/datum/animal/proc/speak()\n\treturn sound\n\n/datum/animal/proc/describe()\n\treturn \"[sound] on [legs] legs\""
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.describe()"), "\"woof on 4 legs\"");
	println!("ok: new type proc");

	// A new override between a proc and a subtype's override of it: the
	// subtype's ..() goes through it now
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof?\"");
	let changes = source.edit(
		"reload.dm",
		"/datum/animal/dog\n\tsound = \"woof\"\n",
		"/datum/animal/dog\n\tsound = \"woof\"\n\n/datum/animal/dog/speak()\n\treturn \"[..()]!\"\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!?\"");
	println!("ok: new override and subtype parent calls");

	// A new var default reaches instances that never set the var, new ones,
	// and subtypes that inherit it, but not a subtype that sets its own
	eval(&mut client, "write_legs()");
	let changes = source.edit("reload.dm", "\tvar/legs = 4", "\tvar/legs = 3");
	assert_eq!(changes.var_defaults.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.legs"), "3");
	assert_eq!(eval(&mut client, "new_dog_legs()"), "3");
	assert_eq!(eval(&mut client, "legs_written.legs"), "9");
	assert_eq!(eval(&mut client, "new_bird_legs()"), "2");
	assert_eq!(eval(&mut client, "initial(old_dog.legs)"), "3");
	println!("ok: variable default update");

	// New vars: an instance var, overridden by a subtype, a static var, a tmp
	// var and a global, used by new procs. Existing instances have them.
	let changes = source.edit(
		"reload.dm",
		"\tvar/legs = 3\n",
		"\tvar/legs = 3\n\tvar/tail = 1\n\tvar/static/herd = 0\n\tvar/tmp/mood = \"calm\"\n\n/datum/animal/bird\n\ttail = 0\n\nvar/flock_size = \
		 12\n\n/datum/animal/proc/join_herd()\n\therd++\n\treturn \"[herd] [mood] [tail] \
		 [flock_size]\"\n\n/proc/new_bird_tail()\n\tvar/datum/animal/bird/b = new\n\treturn b.tail\n"
	);
	assert_eq!(changes.new_vars.len(), 4, "{:?}", changes.new_vars);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "1");
	assert_eq!(eval(&mut client, "initial(old_dog.tail)"), "1");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	assert_eq!(eval(&mut client, "old_dog.join_herd()"), "\"1 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_puppy.join_herd()"), "\"2 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_dog.herd"), "2");
	assert_eq!(eval(&mut client, "flock_size"), "12");
	assert_eq!(eval(&mut client, "(\"tail\" in old_dog.vars)"), "1");
	println!("ok: new variables");

	// The default of a var a reload added
	let changes = source.edit("reload.dm", "\tvar/tail = 1\n", "\tvar/tail = 2\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "2");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	println!("ok: new variable default");

	// A new parameter, by name too
	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n)\n\treturn \"hi [n]\"\n\n/proc/greet_named()\n\treturn \"unchanged\"",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet(\"al\")"), "\"hi al!\"");
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo?\"");
	println!("ok: new parameter");

	// A parameter whose name nothing in the world had
	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")",
		"/proc/greet(n, sound = \"!\", unheard_of = \"\")\n\treturn \"hi [n][sound][unheard_of]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", \
		 unheard_of = \"~\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo!~\"");
	println!("ok: new parameter name");

	// New static locals: one in a proc that has another, and one in a new proc
	let before: u32 = eval(&mut client, "counter()").parse().unwrap();
	let changes = source.edit(
		"reload.dm",
		"\tvar/static/count = 0\n\tcount += 10\n\treturn count",
		"\tvar/static/count = 0\n\tvar/static/bonus = 100\n\tcount += 10\n\treturn count + bonus\n\n/proc/visits()\n\tvar/static/v = \
		 10\n\tv++\n\treturn v"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), (before + 110).to_string());
	assert_eq!(eval(&mut client, "visits()"), "11");
	assert_eq!(eval(&mut client, "visits()"), "12");
	println!("ok: new static locals");

	// Reject duplicate var declarations on a type and its subtype.
	const CAT: &str = "/datum/animal/cat\n\tsound = \"meow\"\n";
	const DUPE: &str = "/datum/animal\n\tvar/dupe = 1\n\n/datum/animal/cat\n\tsound = \"meow\"\n\tvar/dupe = 2\n";
	let changes = source.edit("reload.dm", CAT, DUPE);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a duplicate var was applied");
	assert!(
		result
			.errors
			.iter()
			.any(|e| e.contains("/datum/animal/cat already has a var of that name")),
		"{:?}",
		result.errors
	);
	source.edit("reload.dm", DUPE, CAT);
	println!("ok: duplicate var definition refused by type");

	// More definitions of a proc on a type that has one: one more of the dog's
	// speak(), which the puppy's ..() goes through now, and two for the cat
	// in one reload, the second calling the first
	let changes = source.edit(
		"reload.dm",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n\n/datum/animal/dog/speak()\n\treturn \
		 \"[..()]!!\"\n\n/datum/animal/cat/speak()\n\treturn \"[..()]~\"\n\n/datum/animal/cat/speak()\n\treturn \
		 uppertext(..())\n\n/proc/cat_says()\n\tvar/datum/animal/cat/c = new\n\treturn c.speak()\n"
	);
	assert_eq!(
		changes.new_procs.len(),
		4,
		"{:?}",
		changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect::<Vec<_>>()
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!!!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!!!?\"");
	assert_eq!(eval(&mut client, "cat_says()"), "\"MEOW~\"");
	println!("ok: additional proc definitions");

	// Taking a definition away: calls and the puppy's ..() go to the one before
	let changes = source.edit("reload.dm", "/datum/animal/dog/speak()\n\treturn \"[..()]!!\"\n", "");
	assert_eq!(changes.removed_procs.len(), 1, "{:?}", changes.removed_procs);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!?\"");
	// and putting it back
	let changes = source.edit(
		"reload.dm",
		"/proc/cat_says()\n",
		"/datum/animal/dog/speak()\n\treturn \"[..()]!!\"\n\n/proc/cat_says()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!!!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!!!?\"");
	println!("ok: remove and restore definition");

	// Re-adding an identical proc must allocate a fresh ID.
	let without = "/proc/cat_says()\n";
	let with = "/proc/again()\n\treturn 7\n\n/proc/cat_says()\n";
	let mut removal = None;
	for round in 0..2 {
		let changes = source.edit("reload.dm", without, with);
		assert_eq!(changes.new_procs.len(), 1, "round {}", round);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "round {}: {:?}", round, result.warnings);
		assert_eq!(eval(&mut client, "again()"), "7");
		let changes = source.edit("reload.dm", with, without);
		assert_eq!(changes.removed_procs.len(), 1, "round {}", round);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "round {}: {:?}", round, result.warnings);
		removal = Some(changes);
	}
	// Repeated removal produces a warning without rejecting the reload.
	let result = reload(&mut watcher, &removal.unwrap());
	assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
	println!("ok: repeated proc addition and removal");

	// A new parameter next to `in` lists: one BYOND has built in, and an
	// expression it keeps
	let changes = source.edit(
		"reload.dm",
		"/proc/in_lists(mob/m in world, o in global.vars)\n\treturn 1",
		"/proc/in_lists(mob/m in world, o in global.vars, n = 5)\n\treturn n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "in_lists()"), "5");
	assert_eq!(eval(&mut client, "in_lists(n = 6)"), "6");
	println!("ok: parameter addition preserves input lists");

	// A default that isn't a constant: new instances of the type and the
	// subtype that inherits it get it, the subtype with its own doesn't, and
	// instances that exist keep theirs
	let changes = source.edit("reload.dm", "\tvar/list/items = list(\"a\")", "\tvar/list/items = list(\"a\", \"b\")");
	assert_eq!(
		changes.init_procs.len(),
		2,
		"{:?}",
		changes.init_procs.iter().map(|i| &i.type_path).collect::<Vec<_>>()
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_items(/datum/bag)"), "\"a,b\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/big)"), "\"a,b\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/fixed)"), "\"z\"");
	assert_eq!(eval(&mut client, "jointext(old_bag.items, \",\")"), "\"a\"");
	println!("ok: computed default update");

	// New vars whose defaults aren't constants, on a type with an init proc
	// and one without
	let changes = source.edit(
		"reload.dm",
		"\tvar/list/items = list(\"a\", \"b\")\n",
		"\tvar/list/items = list(\"a\", \"b\")\n\tvar/list/tags = list(1, 2)\n\n/datum/animal\n\tvar/list/toys = \
		 list(\"ball\")\n\n/proc/new_tags()\n\tvar/datum/bag/b = new\n\treturn length(b.tags)\n\n/proc/new_toys()\n\tvar/datum/animal/dog/d = \
		 new\n\treturn d.toys[1]\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_tags()"), "2");
	assert_eq!(eval(&mut client, "new_toys()"), "\"ball\"");
	assert_eq!(eval(&mut client, "isnull(old_bag.tags)"), "1");
	println!("ok: new computed variables");

	// And back to a constant
	let changes = source.edit("reload.dm", "\tvar/list/items = list(\"a\", \"b\")\n", "\tvar/list/items = null\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_items(/datum/bag)"), "\"none\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/fixed)"), "\"z\"");
	assert_eq!(eval(&mut client, "new_tags()"), "2");
	println!("ok: computed default becomes constant");

	// New types: a subtype of a type the world has, a subtype of that, and a
	// datum, with vars, overrides, built-in fields and procs of their own
	let changes = source.edit(
		"reload.dm",
		"/obj/crate/proc/describe_crate()\n",
		"/obj/crate/big_crate\n\tdesc = \"roomy\"\n\ticon_state = \"big\"\n\tdensity = 1\n\tcapacity = 8\n\tvar/list/stickers = \
		 list(\"fragile\")\n\n/obj/crate/big_crate/describe_crate()\n\treturn \"[..()] with [stickers[1]]\"\n\n/obj/crate/big_crate/huge\n\tname = \
		 \"huge crate\"\n\tcapacity = 16\n\n/datum/ledger\n\tvar/entries = 0\n\n/datum/ledger/proc/add()\n\tentries++\n\treturn \
		 entries\n\n/proc/ledger_test()\n\tvar/datum/ledger/l = new\n\tl.add()\n\treturn \"[l.add()] [istype(l, /datum/ledger)] [istype(l, /datum)] \
		 [l.type]\"\n\n/obj/crate/proc/describe_crate()\n"
	);
	assert_eq!(changes.new_types.len(), 3, "{:?}", changes.new_types);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "crate_report(/obj/crate)"), "\"crate|||0|4|crate holds 4|empty|c\"");
	assert_eq!(
		eval(&mut client, "crate_report(/obj/crate/big_crate)"),
		"\"crate|roomy|big|1|8|crate holds 8 with fragile|empty|c\""
	);
	assert_eq!(
		eval(&mut client, "crate_report(/obj/crate/big_crate/huge)"),
		"\"huge crate|roomy|big|1|16|huge crate holds 16 with fragile|empty|h\""
	);
	assert_eq!(eval(&mut client, "ledger_test()"), "\"2 1 1 /datum/ledger\"");
	assert_eq!(eval(&mut client, "text2path(\"/obj/crate/big_crate/huge\")"), "/obj/crate/big_crate/huge");
	assert_eq!(eval(&mut client, "length(typesof(/obj/crate))"), "3");
	assert_eq!(eval(&mut client, "istype(new /obj/crate/big_crate/huge, /obj/crate/big_crate)"), "1");
	assert_eq!(eval(&mut client, "istype(new /obj/crate, /obj/crate/big_crate)"), "0");
	// Types after it in the preorder still know their subtypes
	assert_eq!(eval(&mut client, "length(typesof(/datum/animal))"), "5");
	assert_eq!(eval(&mut client, "istype(old_puppy, /datum/animal/dog)"), "1");
	println!("ok: new types");

	// Globals and statics whose initial values aren't constants get them once,
	// as when the world starts
	let changes = source.edit(
		"reload.dm",
		"/proc/crate_report(type)\n",
		"var/list/once_global = list(\"g\", \"h\")\n\n/obj/crate\n\tvar/static/list/once_static = list(1, 2, \
		 3)\n\n/proc/once_local()\n\tvar/static/datum/ledger/l = new\n\treturn l.add()\n\n/proc/once_report()\n\tvar/obj/crate/c = new\n\treturn \
		 \"[length(once_global)] [length(c.once_static)]\"\n\n/proc/crate_report(type)\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "once_report()"), "\"2 3\"");
	assert_eq!(eval(&mut client, "once_local()"), "1");
	assert_eq!(eval(&mut client, "once_local()"), "2");
	println!("ok: computed global and static initializers");

	// set waitfor = 0: the caller no longer waits for the sleep, which an
	// expression can't sit through
	assert!(sleeps(&mut client, "caller_sees()"));
	let changes = source.edit("reload.dm", "/proc/slow()\n\tsleep(1)", "/proc/slow()\n\tset waitfor = 0\n\tsleep(1)");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "caller_sees()"), "1");
	println!("ok: waitfor setting");

	// An override's settings come from the declaration, so a change there
	// reaches it, though its code is the same
	assert_eq!(eval(&mut client, "nap_skipped(/datum/napper/cat)"), "1");
	let changes = source.edit(
		"reload.dm",
		"\tset waitfor = FALSE\n\tsleep(1)\n\treturn 1",
		"\tset waitfor = TRUE\n\tsleep(1)\n\treturn 1"
	);
	assert_eq!((changes.reloads.len(), changes.settings.len()), (1, 1));
	reload(&mut watcher, &changes);
	assert!(sleeps(&mut client, "nap_skipped(/datum/napper/cat)"));
	println!("ok: declaration settings inheritance");

	// Verbs: a new one and an override with `..()`, which things made from
	// then on get, settings from the declaration, and taking the override away
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Helpers\"");
	let changes = source.edit(
		"reload.dm",
		"/mob/helper/small\n",
		"/mob/helper/verb/wave()\n\tset name = \"Wave Hello\"\n\treturn \"wave\"\n\n/mob/helper/small\n\n/mob/helper/small/greet()\n\treturn \
		 \"[..()]!\"\n"
	);
	reload(&mut watcher, &changes);
	// As DreamMaker lists them: the type's own, newest first, then its parent's
	assert_eq!(
		eval(&mut client, "verb_report(new /mob/helper/small)"),
		"\"Greet:Helpers,Wave Hello:,Greet:Helpers\""
	);
	assert_eq!(eval(&mut client, "verb_report(new /mob/helper)"), "\"Wave Hello:,Greet:Helpers\"");
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/small)"), "\"hi!\"");
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Helpers\"");
	assert_eq!(eval(&mut client, "helper_greets(old_helper)"), "\"hi\"");
	assert_eq!(eval(&mut client, "call(old_helper, \"wave\")()"), "\"wave\"");

	let changes = source.edit("reload.dm", "\tset category = \"Helpers\"\n", "\tset category = \"Friends\"\n");
	assert_eq!((changes.reloads.len(), changes.settings.len()), (1, 1));
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Friends\"");
	assert_eq!(
		eval(&mut client, "verb_report(new /mob/helper/small)"),
		"\"Greet:Friends,Wave Hello:,Greet:Friends\""
	);

	let changes = source.edit("reload.dm", "/mob/helper/small/greet()\n\treturn \"[..()]!\"\n", "");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(new /mob/helper/small)"), "\"Wave Hello:,Greet:Friends\"");
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/small)"), "\"hi\"");
	println!("ok: verb changes");

	// Another definition of a global proc is a proc of its own, which calls
	// don't reach, as in a build
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n\treturn \"first\"\n",
		"/proc/first_def()\n\treturn \"first\"\n\n/first_def()\n\treturn \"second\"\n"
	);
	assert_eq!(changes.new_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "first_def()"), "\"first\"");
	println!("ok: additional global definition");

	// A new `in` list expression (which only a client's command would show)
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/pick_from(n in list(7, 8, 9))\n\treturn n\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "pick_from(8)"), "8");
	println!("ok: new parameter input lists");

	// New prefabs, one used twice
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/prefab_report()\n\tvar/obj/crate/a = new /obj/crate{capacity = 11; name = \"custom\"}()\n\tvar/obj/crate/b = new /obj/crate{capacity \
		 = 11; name = \"custom\"}()\n\tvar/obj/crate/c = new /obj/crate{capacity = 3}\n\treturn \"[a.name] [a.capacity] [b.capacity] [c.name] \
		 [c.capacity] [istype(c, /obj/crate)]\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "prefab_report()"), "\"custom 11 11 crate 3 1\"");
	println!("ok: new prefabs");

	// Built-in vars: on a new type (a field, flags and listed ones), and new
	// defaults on a type the world has, which subtypes that don't set them get
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/obj/crate/fancy\n\talpha = 100\n\tpixel_x = 4\n\tmaptext = \"hi\"\n\tmaptext_width = 64\n\tgender = \"female\"\n\tappearance_flags = \
		 2049\n\tmouse_opacity = 0\n\tcolor = \"#ff0000\"\n\n/proc/builtin_report(type)\n\tvar/obj/o = new type\n\treturn \"[o.name] [o.alpha] \
		 [o.pixel_x] [o.pixel_y] [o.maptext] [o.maptext_width] [o.gender] [o.appearance_flags] [o.mouse_opacity] [o.color]\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/fancy)"),
		"\"crate 100 4 0 hi 64 female 2049 0 #ff0000\""
	);
	let changes = source.edit(
		"reload.dm",
		"/obj/crate\n\tname = \"crate\"\n",
		"/obj/crate\n\tname = \"box\"\n\tpixel_y = 3\n"
	);
	// (text, the first letter of the name, with it)
	assert_eq!(changes.builtin_defaults.len(), 3, "{:?}", changes.builtin_defaults);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "builtin_report(/obj/crate)"), "\"box 255 0 3  32 neuter 0 1 \"");
	assert_eq!(eval(&mut client, "(new /obj/crate).text"), "\"b\"");
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/big_crate)"),
		"\"box 255 0 3  32 neuter 0 1 \""
	);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/fancy)"),
		"\"box 100 4 3 hi 64 female 2049 0 #ff0000\""
	);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/big_crate/huge)"),
		"\"huge crate 255 0 3  32 neuter 0 1 \""
	);
	println!("ok: built-in variable defaults");

	// A type's const: the code that uses it is compiled again
	assert_eq!(eval(&mut client, "kept_limits.max_doubled()"), "6");
	let changes = source.edit("main.dm", "\tvar/const/MAX = 3\n", "\tvar/const/MAX = 7\n");
	assert_eq!(changes.consts, ["MAX"]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.max_doubled()"), "14");
	// read alone it's its global, which has the new value
	assert_eq!(eval(&mut client, "kept_limits.MAX"), "7");
	println!("ok: type const update");

	// A new const on a type: a global, with its value
	let changes = source.edit("main.dm", "\tvar/const/MAX = 7\n", "\tvar/const/MAX = 7\n\tvar/const/MIN = 2\n");
	assert!(changes.refused.is_empty(), "{:?}", changes.refused);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.MIN"), "2");
	println!("ok: new type const");

	// A global const's new value, read alone and folded
	assert_eq!(eval(&mut client, "json_encode(limit_all())"), "\"[10,11]\"");
	let changes = source.edit("main.dm", "var/const/LIMIT_ALL = 10\n", "var/const/LIMIT_ALL = 20\n");
	assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "json_encode(limit_all())"), "\"[20,21]\"");
	println!("ok: global const update");

	// A var declared with another type: only the code that uses it cares
	let changes = source.edit("main.dm", "/datum/limits/var/list/things = null\n", "/datum/limits/var/things = null\n");
	assert_eq!(changes.retyped, ["things"]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.thing_count()"), "0");
	println!("ok: variable type change");

	// A proc's new name, which call() by name finds it by
	assert_eq!(eval(&mut client, "call(kept_limits, \"Label\")()"), "\"labelled\"");
	let changes = source.edit("main.dm", "\tset name = \"Label\"\n", "\tset name = \"Tag\"\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "call(kept_limits, \"Tag\")()"), "\"labelled\"");
	assert_eq!(eval(&mut client, "call(kept_limits, \"label\")()"), "\"labelled\"");
	println!("ok: proc rename");

	// A new mob type, with a mob var of its own and its parent's verbs, and a
	// new default for a mob var of a mob type the world has
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/mob/helper/tiny\n\tname = \"tiny\"\n\tsee_in_dark = 6\n\n/proc/mob_report(type)\n\tvar/mob/m = new type\n\treturn \"[m.type] [m.name] \
		 [m.sight] [m.see_in_dark] [istype(m, /mob/helper)] [ismob(m)] [verb_report(m)]\"\n\n/proc/first_def()\n"
	);
	assert_eq!(changes.new_types.len(), 1, "{:?}", changes.refused);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "mob_report(/mob/helper/tiny)"),
		"\"/mob/helper/tiny tiny 0 6 1 1 Wave Hello:,Greet:Friends\""
	);
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/tiny)"), "\"hi\"");
	assert_eq!(eval(&mut client, "text2path(\"/mob/helper/tiny\")"), "/mob/helper/tiny");
	assert_eq!(eval(&mut client, "length(typesof(/mob/helper))"), "3");
	let changes = source.edit("reload.dm", "/mob/helper/small\n", "/mob/helper/small\n\tsight = 4\n");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "mob_report(/mob/helper/small)"),
		"\"/mob/helper/small small 4 2 1 1 Wave Hello:,Greet:Friends\""
	);
	println!("ok: new mob types");

	// Something that can't be done leaves everything as it was
	let mut request = reload_request(&source.edit("reload.dm", "\treturn 4", "\treturn 5"));
	if let Request::HotReload(HotReloadRequest { procs, .. }) = &mut request {
		let mut bad = procs[0].clone();
		bad.proc.path = "/a_proc_the_world_doesnt_have".to_owned();
		procs.push(bad);
	}
	let result = expect_reload_result(client.request(request));
	assert!(!result.applied);
	assert!(
		result.errors.iter().any(|e| e.contains("a_proc_the_world_doesnt_have")),
		"{:?}",
		result.errors
	);
	assert_eq!(eval(&mut client, "answer()"), "4");
	println!("ok: rejected reload preserves existing code");

	// A definition among a type's others goes where the source has it in the
	// `..()` chain, and the world numbers them again as the source does
	let changes = source.edit(
		"main.dm",
		"/datum/chain/say()\n\treturn ..() + \"3\"",
		"/datum/chain/say()\n\treturn ..() + \"2\"\n\n/datum/chain/say()\n\treturn ..() + \"3\""
	);
	assert_eq!(changes.new_procs.len(), 1);
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"123\"");
	let definition_at = |client: &mut Client, override_id: u32, line: u32| {
		matches!(
			client.request(Request::Offset {
				proc: ProcRef {
					path: "/datum/chain/say".to_owned(),
					override_id
				},
				line
			}),
			Response::Offset { offset: Some(_) }
		)
	};
	assert!(definition_at(&mut client, 1, source.line_of("main.dm", "\treturn ..() + \"2\"")));
	assert!(definition_at(&mut client, 2, source.line_of("main.dm", "\treturn ..() + \"3\"")));
	// The one after it is known by its new number now
	let changes = source.edit("main.dm", "\treturn ..() + \"3\"", "\treturn ..() + \"4\"");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"124\"");
	// And one taken out of the middle
	let changes = source.edit("main.dm", "/datum/chain/say()\n\treturn ..() + \"2\"\n\n", "");
	assert_eq!(changes.removed_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"14\"");
	assert!(definition_at(&mut client, 1, source.line_of("main.dm", "\treturn ..() + \"4\"")));
	println!("ok: insert and remove type proc definitions");

	// A var made static: one value for all, which the code that uses it reads
	assert_eq!(eval(&mut client, "declared_one.read_count()"), "1");
	let changes = source.edit("main.dm", "\tvar/count = 1", "\tvar/static/count = 7");
	assert_eq!(changes.redeclared.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.read_count()"), "7");
	eval(&mut client, "declared_one.count = 8");
	assert_eq!(eval(&mut client, "(new /datum/declared/sub).read_count()"), "8");
	// Back again, with a subtype that sets it: each type has its own default
	let changes = source.edit_many("main.dm", &[
		("\tvar/static/count = 7", "\tvar/count = 2"),
		("/datum/declared/sub\n", "/datum/declared/sub\n\tcount = 3\n")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/declared).read_count()"), "2");
	assert_eq!(eval(&mut client, "(new /datum/declared/sub).read_count()"), "3");
	// A const, and a tmp
	assert_eq!(eval(&mut client, "issaved(declared_one.count)"), "1");
	let changes = source.edit_many("main.dm", &[
		("\tvar/shared = 10", "\tvar/const/shared = 11"),
		("\tvar/count = 2", "\tvar/tmp/count = 2")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.read_shared()"), "11");
	assert_eq!(eval(&mut client, "issaved(declared_one.count)"), "0");
	// A static whose value is worked out once
	let changes = source.edit("main.dm", "\tvar/many = null", "\tvar/static/list/many = list(1, 2, 3)");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.many_count()"), "3");
	println!("ok: static, const and tmp declaration transitions");

	// Builtins, a key worked out and __TYPE__ in a global proc
	let changes = source.edit(
		"main.dm",
		"/proc/compiled_late()\n\treturn 0",
		concat!(
			"/proc/compiled_late()\n",
			"\tvar/list/got = list(cmptextEx(\"a\", \"a\", \"a\"), cmptext(\"a\", \"A\", \"b\"),",
			" findlasttext_char(\"abcabc\", \"b\"), isnull(__TYPE__), keyed(lowertext(\"B\") = 3))\n",
			"\treturn jointext(got, \",\")"
		)
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "compiled_late()"), "\"1,0,5,1,3\"");
	println!("ok: additional compiler forms");

	// A verb's definitions, a world proc's and a global proc's come and go
	// among the others too
	let definition_at = |client: &mut Client, path: &str, override_id: u32, line: u32| {
		matches!(
			client.request(Request::Offset {
				proc: ProcRef {
					path: path.to_owned(),
					override_id
				},
				line
			}),
			Response::Offset { offset: Some(_) }
		)
	};
	assert_eq!(eval(&mut client, "verb_chain.speak()"), "\"ac\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xz\"");
	// (A mob made before keeps the verbs it had, and its verbs' `..()` goes
	// through them)
	let changes = source.edit_many("main.dm", &[
		(
			"/mob/chain_verbs/speak()\n\treturn ..() + \"c\"",
			"/mob/chain_verbs/speak()\n\treturn ..() + \"b\"\n\n/mob/chain_verbs/speak()\n\treturn ..() + \"c\""
		),
		(
			"/world/chant()\n\treturn ..() + \"z\"",
			"/world/chant()\n\treturn ..() + \"y\"\n\n/world/chant()\n\treturn ..() + \"z\""
		),
		(
			"/solo()\n\treturn \"another\"",
			"/solo()\n\treturn \"between\"\n\n/solo()\n\treturn \"another\""
		)
	]);
	assert_eq!(changes.new_procs.len(), 3);
	let result = reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /mob/chain_verbs).speak()"), "\"abc\"");
	// Listed as DreamMaker lists them, newest first
	assert_eq!(eval(&mut client, "verb_names(new /mob/chain_verbs)"), "\"speak,speak,other,speak\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xyz\"");
	assert_eq!(eval(&mut client, "solo()"), "\"declared\"");
	assert!(result.warnings.iter().all(|w| w.contains("things made before")), "{:?}", result.warnings);
	for (path, needle) in [
		("/mob/chain_verbs/speak", "\treturn ..() + \"c\""),
		("/world/chant", "\treturn ..() + \"z\""),
		("/solo", "\treturn \"another\"")
	] {
		let line = source.line_of("main.dm", needle);
		assert!(definition_at(&mut client, path, 2, line), "{} isn't numbered 2", path);
	}
	// And taken out of the middle again
	let changes = source.edit_many("main.dm", &[
		("/mob/chain_verbs/speak()\n\treturn ..() + \"b\"\n\n", ""),
		("/world/chant()\n\treturn ..() + \"y\"\n\n", ""),
		("/solo()\n\treturn \"between\"\n\n", "")
	]);
	assert_eq!(changes.removed_procs.len(), 3);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /mob/chain_verbs).speak()"), "\"ac\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xz\"");
	assert_eq!(eval(&mut client, "solo()"), "\"declared\"");
	for (path, needle) in [
		("/mob/chain_verbs/speak", "\treturn ..() + \"c\""),
		("/world/chant", "\treturn ..() + \"z\""),
		("/solo", "\treturn \"another\"")
	] {
		let line = source.line_of("main.dm", needle);
		assert!(definition_at(&mut client, path, 1, line), "{} isn't numbered 1", path);
	}
	println!("ok: verb, world and global definition edits");

	// A world/Del of the world's own: the source's numbers hold, with the
	// debugger's own world/Del set apart, and it stays beneath that one
	let changes = source.edit(
		"main.dm",
		"var/datum/limits/kept_limits = new\n",
		"var/datum/limits/kept_limits = new\n\n/world/Del()\n\tworld.log << \"world/Del says bye\"\n\t..()\n"
	);
	assert_eq!(changes.new_procs.len(), 1);
	assert_eq!(changes.new_procs[0].code.proc.override_id, 0);
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	offset_of(
		&mut client,
		"/world/Del",
		source.line_of("main.dm", "\tworld.log << \"world/Del says bye\"")
	);
	println!("ok: world/Del source override numbering");

	// Shutting auxtools down puts back what the world had, which BYOND frees
	// itself. The world then starts again from its .dmb.
	eval(&mut client, "request_reboot()");
	client.wait_closed();
	let mut client = world.connect();
	assert!(world.output().contains("world/Del says bye"), "world/Del override was not called");
	assert_eq!(eval(&mut client, "answer()"), "1");
	assert_eq!(eval(&mut client, "few_locals()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "old_dog.legs"), "4");
	println!("ok: reboot after reload");

	// The watcher doesn't need a debugger there. With the debugger gone, the
	// world says what it runs.
	client.send(Request::Disconnect);
	client.wait_closed();
	let mut watcher = world.connect_watcher();
	source.restore();
	let changes = source.edit("reload.dm", "/proc/tick()\n", "/proc/tick()\n\tworld.log << \"tick says [answer()]\"\n");
	reload(&mut watcher, &changes);
	let start = Instant::now();
	while !world.output().contains("tick says 1") {
		assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for reloaded tick()");
		std::thread::sleep(Duration::from_millis(100));
	}
	println!("ok: reload without a debugger");

	println!("Hot reload tests succeeded");
}
