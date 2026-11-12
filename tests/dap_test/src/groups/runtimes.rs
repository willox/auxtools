//! Runtime errors

use serde_json::json;

use crate::{
	ok,
	world::{host_file, World}
};

/// Waits for the world to stop on a runtime error that nothing catches, and
/// checks its message and where it is
fn stopped_on(world: &mut World, message: &str, proc: &str, mark: &str) {
	stopped_on_a(world, false, message, proc, mark);
}

/// The same, for one that's caught or not
fn stopped_on_a(world: &mut World, caught: bool, message: &str, proc: &str, mark: &str) {
	let kind = if caught { "caught runtime error" } else { "runtime error" };
	let event = world.event("stopped");
	assert_eq!(event["body"]["reason"], "exception", "{}", event);
	assert_eq!(event["body"]["description"], message, "{}", event);
	assert_eq!(event["body"]["text"], kind, "{}", event);
	let info = world.request("exceptionInfo", json!({ "threadId": 0 }));
	assert_eq!(info["exceptionId"], kind, "{}", info);
	assert_eq!(info["description"], message, "{}", info);
	assert_eq!(info["breakMode"], if caught { "always" } else { "unhandled" }, "{}", info);
	assert_eq!(info["details"]["message"], message, "{}", info);
	let frame = world.top_frame();
	assert_eq!(
		(frame["name"].as_str().unwrap(), frame["line"].as_u64().unwrap()),
		(proc, world.marks.line(mark)),
		"the runtime error's frame"
	);
}

pub fn run() {
	while_loading();

	let mut world = World::launch(json!({}));
	world.catch_runtimes(true);
	world.configure();

	world.run("fail_null");
	stopped_on(&mut world, "cannot read from list", "/fail_null", "fail_null");
	ok("runtime stop message and location");

	let names: Vec<String> = world.frames(0).iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
	assert_eq!(names[..3], ["/fail_null", "/fail_deep", "/sc_fail_null"]);
	assert_eq!(world.local("L"), "null");
	assert_eq!(world.eval("isnull(L)"), "1");
	ok("inspection at runtime stops");

	let info = world.request("exceptionInfo", json!({ "threadId": 0 }));
	let trace: Vec<&str> = info["details"]["stackTrace"].as_str().unwrap().lines().collect();
	assert_eq!(trace[0], format!("/fail_null (errors.dm:{})", world.marks.line("fail_null")));
	assert_eq!(trace[1], format!("/fail_deep (errors.dm:{})", world.marks.line("fail_deep")));
	assert!(trace[2].starts_with("/sc_fail_null (errors.dm:"), "{:?}", trace);
	ok("runtime exception call stack");

	// The proc that failed is over, and so is its caller's line; the scenario
	// carries on
	let after = world.count("dap: after the failure");
	world.resume_through("fail_null");
	assert_eq!(world.count("dap: after the failure"), after + 1);
	assert!(
		world.output.contains("runtime error: cannot read from list"),
		"the world didn't log the error"
	);
	ok("resume and log after runtime error");

	for (scenario, message, proc, mark) in [
		("fail_index", "list index out of bounds", "/fail_index", "fail_index"),
		("fail_divide", "Division by zero", "/fail_divide", "fail_divide"),
		("fail_crash", "crashed on purpose", "/fail_crash", "fail_crash")
	] {
		world.run(scenario);
		stopped_on(&mut world, message, proc, mark);
		world.resume_through(scenario);
	}
	ok("index, division and CRASH errors");

	world.run("fail_spawned");
	stopped_on(&mut world, "cannot read from list", "/fail_null", "fail_null");
	let names: Vec<String> = world.frames(0).iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
	assert_eq!(names, ["/fail_null", "/sc_fail_spawned"]);
	world.resume_through("fail_spawned");
	ok("runtime error in spawned code");

	// The scenario divides by zero; the block it spawned fails after
	world.run("fail_twice");
	stopped_on(&mut world, "Division by zero", "/fail_divide", "fail_divide");
	world.resume();
	stopped_on(&mut world, "list index out of bounds", "/fail_index", "fail_index");
	world.resume_through("fail_twice");
	ok("consecutive runtime errors");

	// Callee errors end only the failing proc. Evaluation continues and reports
	// the error.
	world.run_to("middle", "/leaf", "leaf_1");
	let frame = world.frame_id();
	let got = world.request("evaluate", json!({ "expression": "fail_null()", "frameId": frame, "context": "watch" }));
	assert_eq!(got["result"], "null", "evaluate gave {}", got);
	let line = world.marks.line("fail_null");
	world.wait_output(&format!(
		"runtime error in a called proc (the evaluation carried on): cannot read from list\n  in /fail_null  errors.dm:{}\n  called by the \
		 expression",
		line
	));
	// An error in the expression frame fails evaluation.
	let why = world.refused("evaluate", json!({ "expression": "list()[1]", "frameId": frame, "context": "watch" }));
	assert_eq!(why, "runtime error: list index out of bounds");
	assert!(!world.has_event("stopped"), "an expression's runtime error stopped the world again");
	assert_eq!(world.eval("x"), "3");
	world.resume_through("middle");
	ok("evaluation errors do not stop recursively");

	world.catch_runtimes(false);
	let logged = world.count("runtime error: cannot read from list");
	world.run_through("fail_null");
	assert_eq!(world.count("runtime error: cannot read from list"), logged + 1);
	ok("disabled runtime filter still logs errors");

	world.catch_runtimes(true);
	world.run("fail_divide");
	stopped_on(&mut world, "Division by zero", "/fail_divide", "fail_divide");
	world.resume_through("fail_divide");
	ok("re-enable runtime filter");

	caught_and_thrown(&mut world);
	conditions(&mut world);
	world.end();
}

/// Runtime errors inside a `try`, and exceptions thrown
fn caught_and_thrown(world: &mut World) {
	// The world carries on through what its own code deals with
	let said = world.count("dap: caught cannot read from list");
	world.run_through("fail_caught");
	world.run_through("fail_caught_here");
	world.run_through("throw_caught");
	assert_eq!(world.count("dap: caught cannot read from list"), said + 1);
	ok("caught errors ignored by default");

	// throw isn't a runtime error to BYOND until nothing has caught it: the
	// stop is at the throw, before it happens
	world.run("fail_throw");
	stopped_on(world, "thrown on purpose", "/fail_throw", "fail_throw");
	let logged = world.count("runtime error: thrown on purpose");
	world.resume_through("fail_throw");
	assert_eq!(world.count("runtime error: thrown on purpose"), logged + 1);
	world.run("throw_text");
	stopped_on(world, "just text", "/sc_throw_text", "throw_text");
	world.resume_through("throw_text");
	ok("uncaught throw stops with its value");

	// Both filters ticked
	world.exception_filters(&["runtimes", "caught"], &[]);
	world.run("fail_caught");
	stopped_on_a(world, true, "cannot read from list", "/fail_null", "fail_null");
	let names: Vec<String> = world.frames(0).iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
	assert_eq!(names[..2], ["/fail_null", "/fail_caught"]);
	world.resume_through("fail_caught");
	world.run("fail_caught_here");
	stopped_on_a(world, true, "cannot read from list", "/fail_caught_here", "fail_caught_here");
	world.resume_through("fail_caught_here");
	world.run("throw_caught");
	stopped_on_a(world, true, "thrown and caught", "/throw_caught", "throw_caught");
	world.resume_through("throw_caught");
	world.run("fail_null");
	stopped_on(world, "cannot read from list", "/fail_null", "fail_null");
	world.resume_through("fail_null");
	ok("caught errors and throws filter");

	// Only the caught ones
	world.exception_filters(&["caught"], &[]);
	world.run_through("fail_null");
	world.run_through("fail_throw");
	world.run("fail_caught");
	stopped_on_a(world, true, "cannot read from list", "/fail_null", "fail_null");
	world.resume_through("fail_caught");
	ok("caught-only filter");
	world.exception_filters(&["runtimes"], &[]);
}

/// A filter's condition: text the error's message has to have, or mustn't
fn conditions(world: &mut World) {
	world.exception_filters(&[], &[("runtimes", "!division, !thrown")]);
	world.run_through("fail_divide");
	world.run_through("fail_throw");
	world.run("fail_index");
	stopped_on(world, "list index out of bounds", "/fail_index", "fail_index");
	world.resume_through("fail_index");
	ok("case-insensitive error exclusions");

	world.exception_filters(&[], &[("runtimes", "bounds, Division")]);
	world.run_through("fail_null");
	world.run("fail_index");
	stopped_on(world, "list index out of bounds", "/fail_index", "fail_index");
	world.resume_through("fail_index");
	world.run("fail_divide");
	stopped_on(world, "Division by zero", "/fail_divide", "fail_divide");
	world.resume_through("fail_divide");
	ok("error inclusion alternatives");

	// Each filter has its own
	world.exception_filters(&["runtimes"], &[("caught", "thrown")]);
	world.run_through("fail_caught");
	world.run("throw_caught");
	stopped_on_a(world, true, "thrown and caught", "/throw_caught", "throw_caught");
	world.resume_through("throw_caught");
	world.run("fail_null");
	stopped_on(world, "cannot read from list", "/fail_null", "fail_null");
	world.resume_through("fail_null");
	ok("independent error filter conditions");
	world.exception_filters(&["runtimes"], &[]);
}

/// A global var's initial value fails while the world loads
fn while_loading() {
	let flag = host_file("fail_early");
	std::fs::write(&flag, "yes").unwrap();
	// The client says nothing about runtime errors: the filter is on unless
	// it's turned off
	let mut world = World::launch(json!({}));
	world.request("configurationDone", json!({}));
	stopped_on(&mut world, "cannot read from list", "/early_value", "early_failure");
	std::fs::remove_file(&flag).unwrap();
	world.resume();
	world.wait_output("dap: world/New");
	world.run_through("plain");
	ok("runtime filter enabled by default at load");
	world.end();
}
