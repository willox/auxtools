//! Stepping: over, into and out of procs, across `sleep` and `spawn`

use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	world.run_to("plain", "/sc_plain", "plain_1");
	world.step_on("next", "/sc_plain", "plain_2");
	world.step_on("next", "/sc_plain", "plain_3");
	world.resume_through("plain");
	ok("line Step Over");

	world.run_to("plain", "/sc_plain", "plain_1");
	world.step_on("stepIn", "/sc_plain", "plain_2");
	world.resume_through("plain");
	ok("Step Into without a call");

	world.run_to("middle", "/middle", "middle_1");
	world.step_on("next", "/middle", "middle_2");
	assert_eq!(world.local("r"), "8");
	ok("Step Over a call");

	world.step_on("stepIn", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	ok("Step Into a proc");

	world.step_on("next", "/leaf", "leaf_2");
	// Back in the caller, on the line of the call, which isn't over
	world.step_on("next", "/middle", "middle_2");
	world.step_on("next", "/middle", "middle_3");
	assert_eq!(world.local("r"), "26");
	ok("Step Over return to caller");

	world.step_on("next", "/sc_middle", "sc_middle_1");
	world.step_on("next", "/sc_middle", "sc_middle_2");
	assert_eq!(world.local("got"), "26");
	world.resume_through("middle");
	ok("returned value after Step Over");

	world.run_to("middle", "/leaf", "leaf_2");
	world.step_on("stepOut", "/middle", "middle_1");
	assert_eq!(world.local("r"), "null", "the call's line isn't over yet");
	world.step_on("next", "/middle", "middle_2");
	assert_eq!(world.local("r"), "8");
	world.resume_through("middle");
	ok("Step Out to caller");

	world.run_to("parent", "/sc_parent", "sc_parent_1");
	world.step_on("stepIn", "/datum/thing/sub/bump", "sub_bump_1");
	ok("Step Into a type proc");
	world.step_on("stepIn", "/datum/thing/bump", "bump_1");
	assert_eq!(world.argument("by"), "4");
	ok("Step Into parent call");
	world.step_on("stepOut", "/datum/thing/sub/bump", "sub_bump_1");
	world.step_on("stepOut", "/sc_parent", "sc_parent_1");
	world.resume_through("parent");
	ok("nested Step Out");

	// /datum/made has an (init) proc that sets its list, which isn't the
	// user's code to step through
	world.run_to("new", "/sc_new", "sc_new_1");
	world.step_on("stepIn", "/datum/made/New", "made_new");
	world.step_on("stepOut", "/sc_new", "sc_new_1");
	world.step_on("next", "/sc_new", "sc_new_2");
	world.resume_through("new");
	ok("Step Into New skips initializer");

	world.run_to("call", "/sc_call", "sc_call_1");
	world.step_on("stepIn", "/leaf", "leaf_1");
	world.resume_through("call");
	ok("Step Into dynamic call");

	world.run_to("sleeper", "/sleeper", "sleeper_1");
	let thread = world.thread;
	world.step_on("next", "/sleeper", "sleeper_2");
	world.step_on("next", "/sleeper", "sleeper_3");
	assert_eq!(world.local("before"), "7");
	world.step_on("next", "/sleeper", "sleeper_4");
	// BYOND moves a proc's frames the first time it sleeps
	assert_eq!(world.thread, thread, "the proc is another thread since it slept");
	world.resume_through("sleeper");
	ok("Step Over sleep preserves thread");

	// The spawned block runs later, in a copy of the proc
	world.run_to("spawner", "/spawner", "spawner_1");
	world.step_on("next", "/spawner", "spawner_2");
	world.step_on("next", "/spawner", "spawner_3");
	world.step_on("next", "/spawner", "spawner_4");
	assert_eq!(world.local("count"), "1");
	world.break_on(&["spawner_body"]);
	world.resume();
	world.stop_on("breakpoint", "/spawner", "spawner_body");
	world.break_on(&[]);
	world.resume_through("spawner");
	ok("spawn stepping and breakpoint");

	world.run_to("middle", "/middle", "middle_1");
	world.break_on(&["leaf_2"]);
	let stop = world.step("next");
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("breakpoint", "/leaf", at.line("leaf_2")),
		"a breakpoint in the call stepped over"
	);
	world.break_on(&[]);
	world.resume_through("middle");
	ok("breakpoint interrupts Step Over");

	// recurse(3) calls recurse(2), ... to recurse(0)
	world.break_if(&[("recurse_2", "depth == 2")]);
	world.run("recurse");
	world.stop_on("breakpoint", "/recurse", "recurse_2");
	world.break_on(&[]);
	// recurse(2), recurse(3), the scenario and the world's loop
	assert_eq!(world.frames(0).len(), 4);
	world.step_on("next", "/recurse", "recurse_3");
	assert_eq!(world.argument("depth"), "2");
	assert_eq!(world.frames(0).len(), 4);
	ok("Step Over recursive call");
	world.step_on("stepOut", "/recurse", "recurse_2");
	assert_eq!(world.argument("depth"), "3");
	assert_eq!(world.frames(0).len(), 3);
	world.resume_through("recurse");
	ok("Step Out one recursive level");

	world.run_to("flow", "/sc_flow", "flow_1");
	let expected = [
		"flow_for",
		"flow_body",
		"flow_body",
		"flow_if",
		"flow_else",
		"flow_switch",
		"flow_case",
		"flow_try_start",
		"flow_try",
		// quiet_failure() fails, which nobody pauses for
		"flow_catch",
		"flow_return"
	];
	let stepped: Vec<u64> = expected.iter().map(|_| world.step("next").line).collect();
	assert_eq!(stepped, expected.map(|name| at.line(name)), "the lines stepped through");
	world.resume_through("flow");
	ok("stepping through control flow");

	// leaf(3) meets a breakpoint whose condition is false
	world.run_to("middle", "/middle", "middle_1");
	world.break_if(&[("leaf_1", "x == 999")]);
	world.step_on("next", "/middle", "middle_2");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("step skips false breakpoint condition");

	// A scenario's proc is called from a block the world's loop spawned, and
	// nothing called that
	world.run_to("plain", "/sc_plain", "plain_3");
	assert_eq!(world.step("stepOut").proc, "/main_loop");
	world.request("stepOut", json!({ "threadId": 0 }));
	world.finish("plain");
	assert!(!world.has_event("stopped"), "stepOut from the last proc stopped somewhere");
	ok("Step Out without caller resumes");

	// Off the end of that block, whatever runs next (the world's loop, the
	// debugger's own) is no part of the step
	for command in ["next", "stepIn"] {
		world.run_to("plain", "/sc_plain", "plain_3");
		assert_eq!(world.step(command).proc, "/main_loop");
		// The block's last line says the scenario is over
		assert_eq!(world.step(command).proc, "/main_loop");
		world.request(command, json!({ "threadId": 0 }));
		world.finish("plain");
		// Something of the world's has run since
		world.run_through("plain");
	}
	ok("step exhaustion resumes world");

	world.catch_runtimes(true);
	world.run_to("fail_null", "/fail_deep", "fail_deep");
	let stop = world.step("next");
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("exception", "/fail_null", at.line("fail_null")),
		"a runtime error in the call stepped over"
	);
	world.catch_runtimes(false);
	world.resume_through("fail_null");
	ok("runtime error interrupts Step Over");

	// VS Code takes the response to a step to mean the world is running: the
	// stop the step makes has to come after it
	world.run_to("deep", "/recurse", "recurse_1");
	for _ in 0..100 {
		let thread = world.thread;
		world.request("stepIn", json!({ "threadId": thread }));
		assert!(
			!world.events.iter().any(|e| e["event"] == "stopped"),
			"a step's stop came before its response"
		);
		world.stopped("step");
	}
	world.resume_through("deep");
	ok("step response precedes stop event");
	world.end();
}
