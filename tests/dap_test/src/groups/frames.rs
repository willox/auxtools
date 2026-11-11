//! Stack frames

use std::path::Path;

use serde_json::{json, Value};

use crate::{ok, world::World};

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	world.run_to("middle", "/leaf", "leaf_1");
	let frames = world.frames(0);
	let places: Vec<(&str, u64)> = frames
		.iter()
		.map(|f| (f["name"].as_str().unwrap(), f["line"].as_u64().unwrap()))
		.collect();
	assert_eq!(places[..3], [
		("/leaf", at.line("leaf_1")),
		("/middle", at.line("middle_1")),
		("/sc_middle", at.line("sc_middle_1"))
	]);
	assert_eq!(places[3].0, "/main_loop");
	assert_eq!(places.len(), 4);
	ok("frame order and source lines");

	for (frame, file) in frames.iter().zip(["stepping.dm", "stepping.dm", "stepping.dm", "main.dm"]) {
		assert_eq!(frame["source"]["name"], file, "{}", frame);
		assert_eq!(Path::new(frame["source"]["path"].as_str().unwrap()), at.file(file), "{}", frame);
		assert!(frame["presentationHint"].is_null(), "{}", frame);
	}
	ok("frame source paths");

	let reference = frames[0]["instructionPointerReference"].as_str().unwrap();
	// Its proc, which of its codes and the offset, in one hex number (see
	// the bytecode group)
	let hex = reference
		.strip_prefix("0x")
		.unwrap_or_else(|| panic!("the top frame's instruction is {}", reference));
	assert!(
		hex.len() == 16 && u64::from_str_radix(hex, 16).is_ok(),
		"the top frame's instruction is {}",
		reference
	);
	assert_eq!((&frames[0]["column"], &frames[1]["column"]), (&json!(1), &json!(0)));
	let ids: Vec<u64> = frames.iter().map(|f| f["id"].as_u64().unwrap()).collect();
	assert_eq!(ids, [0, 1, 2, 3]);
	ok("frame instruction addresses");

	let page = world.request("stackTrace", json!({ "threadId": 0, "startFrame": 1, "levels": 2 }));
	let names: Vec<&str> = page["stackFrames"]
		.as_array()
		.unwrap()
		.iter()
		.map(|f| f["name"].as_str().unwrap())
		.collect();
	assert_eq!(names, ["/middle", "/sc_middle"]);
	assert_eq!(page["totalFrames"], 4);
	let all = world.request("stackTrace", json!({ "threadId": 0, "levels": 0 }));
	assert_eq!(all["stackFrames"].as_array().unwrap().len(), 4);
	let past = world.request("stackTrace", json!({ "threadId": 0, "startFrame": 9, "levels": 2 }));
	assert_eq!(past["stackFrames"].as_array().unwrap().len(), 0);
	ok("stack pagination");

	// Its own locals, not the top frame's
	let caller = frames[1]["id"].clone();
	let locals = world.scope(&caller, "Locals");
	let locals = crate::client::named(&world.variables(&locals));
	assert_eq!(crate::client::value_of(&locals, "r"), "null");
	let arguments = world.scope(&caller, "Arguments");
	assert_eq!(crate::client::value_of(&crate::client::named(&world.variables(&arguments)), "a"), "3");
	ok("caller variables");

	// What a client still holds once the world carries on
	let stale_frame = frames[0]["id"].clone();
	let stale_scope = world.scope(&stale_frame, "Locals");
	world.resume_through("middle");
	let scopes = world.request("scopes", json!({ "frameId": stale_frame }));
	assert_eq!(scopes["scopes"].as_array().unwrap().len(), 0);
	assert_eq!(world.variables(&stale_scope).len(), 0);
	world.run_through("plain");
	ok("stale frame and scope references");

	world.run_to("deep", "/leaf", "leaf_1");
	let trace = world.request("stackTrace", json!({ "threadId": 0 }));
	// leaf, recurse(0) to recurse(120), the scenario and the world's loop
	assert_eq!(trace["totalFrames"], 124);
	assert_eq!(trace["stackFrames"].as_array().unwrap().len(), 124);
	let page = world.request("stackTrace", json!({ "threadId": 0, "startFrame": 120, "levels": 20 }));
	let names: Vec<&str> = page["stackFrames"]
		.as_array()
		.unwrap()
		.iter()
		.map(|f| f["name"].as_str().unwrap())
		.collect();
	assert_eq!(names, ["/recurse", "/recurse", "/sc_deep", "/main_loop"]);
	let deepest = page["stackFrames"][1]["id"].clone();
	let arguments = world.scope(&deepest, "Arguments");
	assert_eq!(
		crate::client::value_of(&crate::client::named(&world.variables(&arguments)), "depth"),
		"120"
	);
	world.resume_through("deep");
	ok("deep stack pagination");

	// Three procs asleep, each with its own src. Deleting one's src from an
	// expression in its frame doesn't end it (BYOND ends it when it wakes,
	// if at all) but it moves the sleeping procs about: the frames are still
	// the ones they were.
	world.run_to("dozers", "/leaf", "leaf_1");
	let threads = world.threads();
	let mut dozing: Vec<(Value, String)> = vec![];
	for (thread, name) in &threads {
		if name.starts_with("/datum/dozer/doze  ") {
			let frame = world.frames(*thread)[0]["id"].clone();
			let mine = world.evaluate_in(&frame, "mine", "watch")["result"].as_str().unwrap().to_owned();
			dozing.push((frame, mine));
		}
	}
	assert_eq!(dozing.len(), 3, "{:?}", threads);
	let (first, _) = &dozing[0];
	assert_eq!(world.evaluate_in(first, "end_dozer(src)", "repl")["result"], "null");
	assert_eq!(world.evaluate_in(first, "src", "watch")["result"], "null");
	for (frame, mine) in &dozing {
		assert_eq!(&world.evaluate_in(frame, "mine", "watch")["result"], mine.as_str());
		let locals = world.scope(frame, "Locals");
		assert_eq!(crate::client::value_of(&crate::client::named(&world.variables(&locals)), "mine"), mine);
	}
	world.resume_through("dozers");
	ok("stable sleeping frame IDs after evaluation");
	world.end();
}
