//! Breakpoints on procs by name

use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	// Each way of writing a proc's path. A proc's own line is its first with
	// code.
	let names = [
		("/proc/leaf", "leaf_1"),
		("/leaf", "leaf_1"),
		("/datum/thing/proc/bump", "bump_1"),
		("/datum/thing/bump", "bump_1"),
		("/datum/thing/sub/bump", "sub_bump_1"),
		("/mob/player/verb/shout", "shout"),
		("/mob/player/shout", "shout"),
		("/datum/thing/twice/bump", "twice_first"),
		("/datum/thing/twice/bump#1", "twice_second")
	];
	let set = world.function_breakpoints(&names.map(|(name, _)| name));
	for (breakpoint, (name, marker)) in set.iter().zip(names) {
		assert_eq!(
			(&breakpoint["verified"], &breakpoint["line"]),
			(&json!(true), &json!(at.line(marker))),
			"{}: {}",
			name,
			breakpoint
		);
	}
	ok("function path forms");
	ok("function override indexes");

	let set = world.function_breakpoints(&["/world/New", "/proc/not_there", "leaf", "/datum/thing/sub/bump#3"]);
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	for (breakpoint, name) in set[1..].iter().zip(["/proc/not_there", "leaf", "/datum/thing/sub/bump#3"]) {
		assert_eq!(breakpoint["verified"], false, "{}", breakpoint);
		assert_eq!(breakpoint["message"], format!("there's no {}", name));
	}
	ok("world proc lookup and missing-proc diagnostics");

	world.function_breakpoints(&["/proc/leaf"]);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_1");
		assert_eq!(world.local("y"), "null");
		world.resume();
	}
	world.finish("middle");
	ok("function breakpoint entry position");

	// A breakpoint on that line as well: two in one place, each its own
	let id = |breakpoint: &serde_json::Value| breakpoint["id"].as_u64().expect("a breakpoint with no id");
	let on_procs = world.function_breakpoints(&["/proc/middle", "/proc/leaf"]);
	let on_line = world.try_break_if(&[("leaf_1", "x == 999")]);
	assert_ne!(id(&on_procs[1]), id(&on_line[0]));
	world.run("middle");
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), &stop.hit), ("/middle", &vec![id(&on_procs[0])]));
	world.resume();
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), &stop.hit), ("/leaf", &vec![id(&on_procs[1]), id(&on_line[0])]));
	ok("function and line breakpoint hit IDs");

	// The line's goes, with its condition that never held; the proc's stays
	world.break_on(&[]);
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	// And the other way round
	world.break_on(&["leaf_1"]);
	world.function_breakpoints(&[]);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_1");
		world.resume();
	}
	world.finish("middle");
	world.break_on(&[]);
	world.function_breakpoints(&["/proc/leaf"]);
	ok("independent function and line removal");

	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.step_on("next", "/leaf", "leaf_2");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.step_on("stepOut", "/middle", "middle_2");
	world.resume_through("middle");
	ok("stepping from function entry");

	world.function_breakpoints(&["/datum/thing/twice/bump#1", "/datum/thing/twice/bump"]);
	world.run("twice");
	// The last definition runs first, and calls the one before with ..()
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), stop.line), ("/datum/thing/twice/bump", at.line("twice_second")));
	world.resume();
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), stop.line), ("/datum/thing/twice/bump", at.line("twice_first")));
	world.resume_through("twice");
	ok("breakpoints in multiple overrides");

	world.function_breakpoints(&["/proc/middle"]);
	world.run("middle");
	assert_eq!(world.stop().proc, "/middle");
	world.resume_through("middle");
	ok("function breakpoint replacement");

	world.function_breakpoints(&[]);
	world.run_through("middle");
	ok("clearing function breakpoints");

	// leaf(3), then leaf(8)
	let set = world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [{ "name": "/proc/leaf", "condition": "x == 8" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], true);
	world.run("middle");
	assert_eq!(world.stop().proc, "/leaf");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	ok("function breakpoint condition");
	world.end();
}
