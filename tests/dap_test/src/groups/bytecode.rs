//! Bytecode: the disassembly of the code a frame runs, old code included

use serde_json::{json, Value};

use crate::{
	groups::reloading::{copy_of_the_world, edit},
	ok,
	world::World
};

/// An address as the launcher writes it
fn address(text: &Value) -> u64 {
	let text = text.as_str().unwrap_or_else(|| panic!("{} isn't an address", text));
	u64::from_str_radix(text.strip_prefix("0x").unwrap(), 16).unwrap()
}

fn hex(address: u64) -> String {
	format!("0x{:016X}", address)
}

/// The proc and which of its codes an address is in, as an address
const CODE: u64 = !0xFFFF;

/// `count` instructions from `shift` on from the one at `reference`
fn disassemble(world: &mut World, reference: u64, shift: i64, count: i64) -> Vec<Value> {
	let answer = world.request(
		"disassemble",
		json!({ "memoryReference": hex(reference), "instructionOffset": shift, "instructionCount": count, "resolveSymbols": true })
	);
	answer["instructions"].as_array().unwrap().clone()
}

/// Whether it's one of the code's, not a place before or after it
fn is_code(instruction: &Value) -> bool {
	instruction["presentationHint"] != "invalid"
}

/// The whole of the code an address is in, from its start
fn listing(world: &mut World, reference: u64) -> Vec<Value> {
	disassemble(world, reference & CODE, 0, 1000).into_iter().filter(is_code).collect()
}

fn texts(listing: &[Value]) -> Vec<String> {
	listing.iter().map(|i| i["instruction"].as_str().unwrap().to_owned()).collect()
}

/// Makes these the client's instruction breakpoints, and returns them as
/// the launcher answered
fn break_at(world: &mut World, addresses: &[u64]) -> Vec<Value> {
	let breakpoints: Vec<Value> = addresses.iter().map(|&a| json!({ "instructionReference": hex(a) })).collect();
	let set = world.request("setInstructionBreakpoints", json!({ "breakpoints": breakpoints }));
	set["breakpoints"].as_array().unwrap().clone()
}

/// The address of the first instruction of a listing with this text
fn find(listing: &[Value], text: &str) -> u64 {
	let found = listing.iter().find(|i| i["instruction"] == text);
	address(&found.unwrap_or_else(|| panic!("no {} in {:?}", text, texts(listing)))["address"])
}

/// A step of `granularity`, and where it stopped: the proc and the address
fn step_by(world: &mut World, command: &str, granularity: &str) -> (String, u64) {
	let thread = world.thread;
	world.request(command, json!({ "threadId": thread, "granularity": granularity }));
	let stop = world.stop();
	assert_eq!(stop.reason, "step");
	(stop.proc, frame_address(world))
}

/// The paused frame's instruction
fn frame_address(world: &mut World) -> u64 {
	let frame = world.top_frame();
	address(&frame["instructionPointerReference"])
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	world.run_to("middle", "/leaf", "leaf_1");
	let frame = world.top_frame();
	let here = address(&frame["instructionPointerReference"]);
	let one = disassemble(&mut world, here, 0, 1);
	assert_eq!(one.len(), 1, "{:?}", one);
	assert_eq!(address(&one[0]["address"]), here);
	assert_eq!(one[0]["line"], at.line("leaf_1"));
	assert_eq!(one[0]["location"]["path"], frame["source"]["path"]);
	assert_eq!(one[0]["symbol"], "/leaf");
	ok("frame instruction disassembly and source location");

	// Far more than leaf() has, on both sides
	let window = disassemble(&mut world, here, -100, 300);
	assert_eq!(window.len(), 300);
	let addresses: Vec<u64> = window.iter().map(|i| address(&i["address"])).collect();
	assert!(
		addresses.windows(2).all(|pair| pair[0] < pair[1]),
		"addresses out of order: {:?}",
		addresses
	);
	assert!(addresses.contains(&here));
	let code: Vec<usize> = (0..window.len()).filter(|&i| is_code(&window[i])).collect();
	let (first, last) = (code[0], *code.last().unwrap());
	assert!(first > 0 && last < window.len() - 1, "no room before and after leaf()");
	assert_eq!(code.len(), last - first + 1, "something that isn't code in the middle of leaf()");
	assert_eq!(address(&window[first]["address"]), here & CODE, "leaf()'s code doesn't start at offset 0");
	assert!(window[..first]
		.iter()
		.chain(&window[last + 1..])
		.all(|i| i["instruction"] == "" && i["instructionBytes"].is_null()));
	ok("padded disassembly window");

	// Each instruction's words take up the addresses to the next one
	for pair in window[first..=last + 1].windows(2) {
		let words = pair[0]["instructionBytes"].as_str().unwrap().split(' ').count() as u64;
		assert_eq!(
			address(&pair[0]["address"]) + words,
			address(&pair[1]["address"]),
			"{} and {}",
			pair[0],
			pair[1]
		);
	}
	ok("word-based instruction addresses");

	// What's around an instruction is the same, whichever of them a client
	// counts from
	let from_start = disassemble(&mut world, here & CODE, 0, 5);
	let from_third = disassemble(&mut world, address(&from_start[2]["address"]), -2, 5);
	assert_eq!(from_start, from_third);
	// A reference in the middle of an instruction is that instruction
	let wide = window[first..=last]
		.iter()
		.find(|i| i["instructionBytes"].as_str().unwrap().contains(' '))
		.unwrap();
	let inside = disassemble(&mut world, address(&wide["address"]) + 1, 0, 1);
	assert_eq!(&inside[0], wide);
	ok("instruction reference normalization");

	// leaf(x): `var/y = x + 1` and `return y * 2`. The opcodes' numbers
	// differ between BYOND versions; their names don't.
	let leaf = listing(&mut world, here);
	let texts = texts(&leaf);
	let lines: Vec<u64> = leaf.iter().filter_map(|i| i["line"].as_u64()).collect();
	assert!(texts[0].starts_with("DbgFile"), "{:?}", texts);
	assert!(texts.iter().any(|t| *t == format!("DbgLine {}", at.line("leaf_1"))), "{:?}", texts);
	assert!(texts.iter().any(|t| *t == format!("DbgLine {}", at.line("leaf_2"))), "{:?}", texts);
	assert!(texts.iter().any(|t| t.starts_with("Ret")), "{:?}", texts);
	assert_eq!(texts.last().unwrap(), "End", "{:?}", texts);
	assert!(
		lines.windows(2).all(|pair| pair[0] <= pair[1]) && lines.len() == leaf.len(),
		"{:?}",
		lines
	);
	ok("proc disassembly and debug lines");
	world.resume_through("middle");

	instruction_breakpoints(&mut world, &leaf);
	instruction_steps(&mut world);
	world.end();
	old_code();
}

fn instruction_breakpoints(world: &mut World, leaf: &[Value]) {
	let at = world.marks.clone();
	// `return y * 2`: the multiplication, which no line starts at
	let mul = find(leaf, "Mul");
	let set = break_at(world, &[mul]);
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	assert_eq!(address(&set[0]["instructionReference"]), mul);
	assert_eq!(set[0]["line"], at.line("leaf_2"));
	let id = set[0]["id"].as_u64().unwrap();
	world.run("middle");
	let stop = world.stop();
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("instruction breakpoint", "/leaf", at.line("leaf_2"))
	);
	assert_eq!(stop.hit, [id]);
	assert_eq!(frame_address(world), mul);
	assert_eq!(world.eval("y"), "4");
	ok("instruction breakpoint stop and ID");
	world.resume();
	world.stopped("instruction breakpoint");
	break_at(world, &[]);
	world.resume_through("middle");
	ok("instruction breakpoint removal");

	// Inside an instruction of more than one word: in its words, a
	// breakpoint would turn the rest into instructions of their own
	let wide = leaf.iter().find(|i| i["instructionBytes"].as_str().unwrap().contains(' ')).unwrap();
	let wide = address(&wide["address"]);
	let past = break_at(world, &[(mul & CODE) + 0xFFF0]);
	assert_eq!(past[0]["verified"], false, "{}", past[0]);
	let set = break_at(world, &[wide + 1, wide]);
	assert_eq!(set[0]["verified"], false, "{}", set[0]);
	let why = set[0]["message"].as_str().unwrap();
	assert!(why.contains("in the middle of an instruction") && why.contains(&hex(wide)), "{}", why);
	assert_eq!(set[1]["verified"], true, "{}", set[1]);
	world.run("middle");
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), stop.hit.len()), ("/leaf", 1));
	world.resume();
	world.stopped("instruction breakpoint");
	break_at(world, &[]);
	world.resume_through("middle");
	ok("invalid instruction boundary rejection");

	// The instruction a line's breakpoint is on, with one of each: each is
	// its own
	let line_2 = leaf
		.iter()
		.position(|i| i["instruction"] == format!("DbgLine {}", at.line("leaf_2")))
		.unwrap();
	let after = address(&leaf[line_2 + 1]["address"]);
	world.break_on(&["leaf_2"]);
	let by_address = break_at(world, &[after])[0]["id"].as_u64().unwrap();
	world.run("middle");
	let stop = world.stop();
	assert_eq!(stop.reason, "breakpoint");
	assert_eq!(stop.hit.len(), 2, "{:?}", stop.hit);
	assert!(stop.hit.contains(&by_address));
	assert_eq!(frame_address(world), after);
	break_at(world, &[]);
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("independent line and instruction breakpoints");

	// A condition and a hit count, and a reboot: the proc is the same proc
	// in the world that starts again
	let set = world.request(
		"setInstructionBreakpoints",
		json!({ "breakpoints": [{ "instructionReference": hex(mul), "condition": "x == 8", "hitCondition": "2" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], true, "{}", set);
	let worlds = world.count("dap: world/New");
	world.run("reboot");
	world.wait_count("dap: world/New", worlds + 1);
	world.wait_output("The world restarted");
	// Each run calls leaf(3) and leaf(8): the second x == 8 stops
	world.run("middle");
	world.finish("middle");
	assert!(!world.has_event("stopped"), "it stopped at the first hit");
	world.run("middle");
	world.stopped("instruction breakpoint");
	assert_eq!((world.eval("x"), frame_address(world)), ("8".to_owned(), mul));
	break_at(world, &[]);
	world.resume_through("middle");
	ok("instruction conditions, counts and reboot restoration");
}

/// A frame on code a hot reload replaced disassembles to that code
fn old_code() {
	let dir = copy_of_the_world("bytecode");
	let mut world = World::launch_in(dir, json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");

	world.run_to("middle", "/leaf", "leaf_1");
	let paused = frame_address(&mut world);
	let before = listing(&mut world, paused);
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n"
	);
	// Its last line is two further down in the new code
	let extra = format!("DbgLine {}", world.marks.line("leaf_2"));

	let frame = world.top_frame();
	assert_eq!(frame["name"], "/leaf (old code)");
	let here = address(&frame["instructionPointerReference"]);
	assert_ne!(here & 0xFFFF_0000, 0, "an old code's address is the proc's");
	let old = listing(&mut world, here);
	assert_eq!(
		old,
		before
			.iter()
			.map(|i| {
				let mut i = i.clone();
				i["address"] = hex(address(&i["address"]) - (address(&before[0]["address"]) & CODE) + (here & CODE)).into();
				i["symbol"] = "/leaf (old code)".into();
				i["location"] = Value::Null;
				i
			})
			.collect::<Vec<_>>()
	);
	assert!(!texts(&old).contains(&extra));
	let one = disassemble(&mut world, here, 0, 1);
	assert_eq!(one[0]["line"], frame["line"]);
	let now = listing(&mut world, here & 0xFFFF_FFFF_0000_0000);
	assert!(texts(&now).contains(&extra), "{:?}", texts(&now));
	ok("current and pre-reload disassembly");

	world.resume_through("middle");
	world.end();
}

fn instruction_steps(world: &mut World) {
	let at = world.marks.clone();

	world.run_to("middle", "/leaf", "leaf_1");
	let mut here = frame_address(world);
	let leaf = listing(world, here);
	for _ in 0..3 {
		let next = leaf.iter().position(|i| address(&i["address"]) == here).unwrap() + 1;
		let (proc, now) = step_by(world, "next", "instruction");
		assert_eq!((proc.as_str(), now), ("/leaf", address(&leaf[next]["address"])), "{:?}", texts(&leaf));
		here = now;
	}
	ok("instruction Step Over");

	world.step_on("next", "/leaf", "leaf_2");
	ok("line stepping after instruction stepping");
	world.resume_through("middle");

	// `var/r = leaf(a)`: each step is the next of middle()'s instructions,
	// the call to leaf() among them, until the next line
	world.run_to("middle", "/middle", "middle_1");
	let mut here = frame_address(world);
	let middle = listing(world, here);
	let mut steps = 0;
	loop {
		let next = middle.iter().position(|i| address(&i["address"]) == here).unwrap() + 1;
		let (proc, now) = step_by(world, "next", "instruction");
		assert_eq!(
			(proc.as_str(), now),
			("/middle", address(&middle[next]["address"])),
			"{:?}",
			texts(&middle)
		);
		here = now;
		steps += 1;
		if middle[next]["instruction"] == format!("DbgLine {}", at.line("middle_2")) {
			break;
		}
	}
	assert!(steps > 2);
	assert_eq!(world.local("r"), "8");
	ok("instruction Step Over across a call");
	world.resume_through("middle");

	// The same again, into leaf() at the call
	world.run_to("middle", "/middle", "middle_1");
	let mut last = frame_address(world);
	let entered = loop {
		let (proc, now) = step_by(world, "stepIn", "instruction");
		if proc == "/leaf" {
			break now;
		}
		assert_eq!(proc, "/middle");
		assert!(now > last);
		last = now;
	};
	assert_eq!(entered & 0xFFFF, 0, "leaf() wasn't entered at its first instruction");
	assert_eq!(world.argument("x"), "3");
	ok("instruction Step Into enters callee");

	let first = listing(world, entered)[0]["instructionBytes"].as_str().unwrap().split(' ').count() as u64;
	let (proc, now) = step_by(world, "stepIn", "instruction");
	assert_eq!((proc.as_str(), now), ("/leaf", entered + first));
	world.step_on("stepOut", "/middle", "middle_1");
	world.step_on("next", "/middle", "middle_2");
	// Steps by statement are steps by line
	let thread = world.thread;
	world.request("next", json!({ "threadId": thread, "granularity": "statement" }));
	let stop = world.stop();
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("step", "/middle", at.line("middle_3"))
	);
	world.resume_through("middle");
	ok("instruction stepping and Step Out");
}
