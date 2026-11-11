//! Breakpoints on lines

use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	before_the_world_runs();
	placing();
	locations();
	hitting();
}

/// The world waits for the client's breakpoints, so ones in the code that
/// runs as it loads are hit
fn before_the_world_runs() {
	let mut world = World::launch(json!({}));
	world.catch_runtimes(false);
	world.break_on(&["early_value", "tooled", "sub_tool", "world_new"]);
	world.request("configurationDone", json!({}));

	// One proc sets all the world's global vars, in an order of DreamMaker's
	// own: its code starts in one file that has one, and goes on in each of
	// the others
	let tool = world.marks.file("sub/tool.dm");
	let (tooled, early_value, sub_tool) = (world.marks.line("tooled"), world.marks.line("early_value"), world.marks.line("sub_tool"));
	let path = |frame: &serde_json::Value| std::path::PathBuf::from(frame["source"]["path"].as_str().unwrap_or_default());
	let mut seen = vec![];
	for _ in 0..3 {
		let stop = world.stop();
		assert_eq!(stop.reason, "breakpoint");
		let thread = world.thread;
		let frames = world.frames(thread);
		match stop.proc.as_str() {
			"/early_value" => assert_eq!(stop.line, early_value),
			"/sub_tool" => {
				assert_eq!(stop.line, sub_tool);
				assert_eq!(
					(frames[1]["line"].as_u64(), path(&frames[1])),
					(Some(tooled), tool.clone()),
					"{}",
					frames[1]
				);
			}
			_ => assert_eq!((stop.line, path(&frames[0])), (tooled, tool.clone()), "{}", frames[0])
		}
		seen.push(stop.proc);
		world.resume();
	}
	seen.sort();
	seen.dedup();
	assert_eq!(seen.len(), 3, "{:?}", seen);
	ok("breakpoint in global initializer");
	ok("breakpoint in later initializer source");
	ok("initializer frame source location");
	world.stop_on("breakpoint", "/world/New", "world_new");
	ok("breakpoint in world/New");
	world.resume();
	world.end();
}

fn placing() {
	let mut world = World::start();
	let at = world.marks.clone();
	let stepping = at.file("stepping.dm");
	let (leaf_1, leaf_2) = (at.line("leaf_1"), at.line("leaf_2"));

	let set = world.set_breakpoints(&stepping, json!([leaf_2]));
	assert_eq!((&set[0]["verified"], &set[0]["line"]), (&json!(true), &json!(leaf_2)), "{}", set[0]);
	assert!(set[0]["id"].is_number(), "a breakpoint with no id: {}", set[0]);
	ok("executable line verification");

	// The proc's declaration, the blank line before it and the comment that
	// starts the file have no code: the next line that has is leaf's first
	let declaration = at.line_starting("stepping.dm", "/proc/leaf(x)");
	let set = world.set_breakpoints(&stepping, json!([declaration, declaration - 1, 1]));
	for breakpoint in &set {
		assert_eq!(
			(&breakpoint["verified"], &breakpoint["line"]),
			(&json!(true), &json!(leaf_1)),
			"{}",
			breakpoint
		);
	}
	ok("non-executable line remapping");

	let set = world.set_breakpoints(&stepping, json!([99_999]));
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "no executable code at or after this line");
	let elsewhere = at.dir.join("not_in_the_world.dm");
	let set = world.set_breakpoints(&elsewhere, json!([3]));
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "no executable code at or after this line");
	ok("invalid source breakpoint diagnostics");

	let first = world.set_breakpoints(&stepping, json!([leaf_1, leaf_2]));
	let second = world.set_breakpoints(&at.file("errors.dm"), json!([at.line("fail_null")]));
	let mut ids: Vec<u64> = first.iter().chain(&second).map(|b| b["id"].as_u64().unwrap()).collect();
	ids.sort_unstable();
	ids.dedup();
	assert_eq!(ids.len(), 3, "breakpoints share ids");
	ok("unique breakpoint IDs");

	let why = world.refused(
		"setBreakpoints",
		json!({ "source": { "name": "stepping.dm" }, "breakpoints": [{ "line": leaf_1 }] })
	);
	assert_eq!(why, "breakpoint source is missing a path");
	ok("missing source path diagnostic");
	world.set_breakpoints(&stepping, json!([]));
	world.set_breakpoints(&at.file("errors.dm"), json!([]));

	// A `var` with no value is a line of the world's code with nothing on
	// it: its breakpoint is where the next line starts
	let tool = at.file("sub/tool.dm");
	let (var, first, second) = (at.line("declares_var"), at.line("declares_first"), at.line("declares_second"));
	let set = world.set_breakpoints(&tool, json!([var]));
	assert_eq!((&set[0]["verified"], &set[0]["line"]), (&json!(true), &json!(var)), "{}", set[0]);
	world.run("declares");
	world.stop_at("breakpoint", "/sub_declares", var);
	world.step_to("next", "/sub_declares", first);
	world.step_to("next", "/sub_declares", second);
	world.resume_through("declares");
	ok("uninitialized declaration breakpoint and stepping");

	let set = world.set_breakpoints(&tool, json!([var, first]));
	for (breakpoint, line) in set.iter().zip([var, first]) {
		assert_eq!(
			(&breakpoint["verified"], &breakpoint["line"]),
			(&json!(true), &json!(line)),
			"{}",
			breakpoint
		);
	}
	world.run("declares");
	world.stop_at("breakpoint", "/sub_declares", var);
	world.resume();
	world.stop_at("breakpoint", "/sub_declares", first);
	world.set_breakpoints(&tool, json!([]));
	world.resume_through("declares");
	ok("adjacent declaration breakpoint");
	world.end();
}

/// The lines `breakpointLocations` gives for a range of a file
fn locations_in(world: &mut World, file: &std::path::Path, line: u64, end_line: Option<u64>) -> Vec<u64> {
	let mut arguments = json!({ "source": { "path": file }, "line": line });
	if let Some(end_line) = end_line {
		arguments["endLine"] = json!(end_line);
	}
	let answer = world.request("breakpointLocations", arguments);
	answer["breakpoints"]
		.as_array()
		.unwrap()
		.iter()
		.map(|location| {
			assert!(location.get("column").is_none(), "a location with a column: {}", location);
			location["line"].as_u64().unwrap()
		})
		.collect()
}

fn locations() {
	let mut world = World::start();
	let at = world.marks.clone();
	let stepping = at.file("stepping.dm");

	// The comment that starts the file, blank lines and declarations have none
	let lines = locations_in(&mut world, &stepping, 1, Some(at.line("sc_middle_2")));
	let marked: Vec<u64> = ["leaf_1", "leaf_2", "middle_1", "middle_2", "middle_3", "sc_middle_1", "sc_middle_2"]
		.iter()
		.map(|name| at.line(name))
		.collect();
	assert_eq!(lines, marked);
	ok("executable locations in a range");

	let leaf_2 = at.line("leaf_2");
	assert_eq!(locations_in(&mut world, &stepping, leaf_2, None), [leaf_2]);
	assert_eq!(locations_in(&mut world, &stepping, leaf_2, Some(leaf_2)), [leaf_2]);
	let declaration = at.line_starting("stepping.dm", "/proc/leaf(x)");
	assert_eq!(locations_in(&mut world, &stepping, declaration, None), Vec::<u64>::new());
	ok("single-line executable locations");

	let elsewhere = at.dir.join("not_in_the_world.dm");
	assert_eq!(locations_in(&mut world, &elsewhere, 1, Some(100)), Vec::<u64>::new());
	assert_eq!(locations_in(&mut world, &stepping, 99_999, Some(100_000)), Vec::<u64>::new());
	ok("missing source locations");
	world.end();
}

fn hitting() {
	let mut world = World::start();

	// middle() calls leaf() twice, then returns
	world.break_on(&["leaf_1", "middle_3"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("breakpoints set while running");

	world.break_on(&["middle_3"]);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	ok("replacement removes omitted breakpoints");

	// Paused: one more, further on
	world.break_on(&["middle_3", "sc_middle_2"]);
	world.resume();
	world.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	world.resume_through("middle");
	ok("breakpoints set while paused");

	world.break_on(&[]);
	world.run_through("middle");
	ok("clearing all breakpoints");

	world.break_on(&["spawner_body"]);
	world.run("spawner");
	world.stop_on("breakpoint", "/spawner", "spawner_body");
	assert_eq!(world.local("count"), "0", "the spawned block has its own copy of the proc's locals");
	world.resume_through("spawner");
	ok("breakpoint in spawned code");

	world.break_on(&["shout"]);
	world.run("verb");
	world.stop_on("breakpoint", "/mob/player/shout", "shout");
	world.resume_through("verb");
	ok("breakpoint in a verb");

	world.break_on(&["sub_bump_1", "bump_1"]);
	world.run("parent");
	world.stop_on("breakpoint", "/datum/thing/sub/bump", "sub_bump_1");
	world.resume();
	world.stop_on("breakpoint", "/datum/thing/bump", "bump_1");
	world.resume_through("parent");
	ok("breakpoints across override calls");

	// The same line, once for each call deeper
	world.break_on(&["recurse_1"]);
	world.run("recurse");
	for depth in ["3", "2", "1", "0"] {
		world.stop_on("breakpoint", "/recurse", "recurse_1");
		assert_eq!(world.argument("depth"), depth);
		world.resume();
	}
	world.finish("recurse");
	ok("recursive breakpoint hits");
	world.end();
}
