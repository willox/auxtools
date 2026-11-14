//! Evaluating expressions: in the debug console, as watches, in any frame

use serde_json::{json, Value};

use crate::{ok, world::World};

/// What the launcher offers to go on with, typed as far as `text` in the
/// paused frame (or in no frame): each name with its kind
fn offered(world: &mut World, text: &str, frame: Option<&serde_json::Value>) -> Vec<(String, String)> {
	let targets = world.request(
		"completions",
		json!({ "frameId": frame, "text": text, "column": text.encode_utf16().count() + 1 })
	);
	targets["targets"]
		.as_array()
		.unwrap()
		.iter()
		.map(|t| (t["label"].as_str().unwrap().to_owned(), t["type"].as_str().unwrap().to_owned()))
		.collect()
}

/// Completions in the debug console, in the frame of a /datum/thing/sub's
/// inside(by), with a local local_list
fn completions(world: &mut World) {
	let frame = world.frame_id();
	let names = |world: &mut World, text: &str| -> Vec<String> { offered(world, text, Some(&frame)).into_iter().map(|(name, _)| name).collect() };
	let pair = |name: &str, kind: &str| (name.to_owned(), kind.to_owned());

	assert_eq!(offered(world, "by", Some(&frame)), [pair("by", "variable")]);
	assert_eq!(names(world, "loc"), ["local_list"]);
	// The frame's own, src's vars and procs, globals and global procs
	let all = offered(world, "", Some(&frame));
	for expected in [
		pair("by", "variable"),
		pair("local_list", "variable"),
		pair("src", "variable"),
		pair("usr", "variable"),
		pair("args", "variable"),
		pair("n", "variable"),
		pair("items", "variable"),
		pair("bump", "function"),
		pair("inside", "function"),
		pair("global_number", "variable"),
		pair("leaf", "function"),
		pair("middle", "function"),
		pair("world", "variable")
	] {
		assert!(all.contains(&expected), "{:?} isn't offered", expected);
	}
	assert!(!all.iter().any(|(name, _)| name.contains("auxtools") || name.contains('(')), "{:?}", all);
	ok("frame-scope completion");

	// After a dot: what that is, in the frame
	let members = offered(world, "src.", Some(&frame));
	for expected in [
		pair("n", "property"),
		pair("name", "property"),
		pair("other", "property"),
		pair("shared", "property"),
		pair("type", "property"),
		pair("bump", "method"),
		pair("inside", "method")
	] {
		assert!(members.contains(&expected), "{:?} isn't offered", expected);
	}
	assert!(
		!members.iter().any(|(name, _)| name == "by" || name == "leaf" || name == "hp"),
		"{:?}",
		members
	);
	assert_eq!(names(world, "src.o"), ["other"]);
	assert_eq!(names(world, "x + src.BU"), ["bump"]);
	assert_eq!(names(world, "world.max"), ["maxx", "maxy", "maxz"]);
	ok("case-insensitive member completion");

	let mob = names(world, "new /mob/player(null).");
	assert_eq!(mob, Vec::<String>::new(), "completion must not evaluate a call");
	assert_eq!(names(world, "src.other."), Vec::<String>::new());
	assert_eq!(names(world, "nowhere_at_all."), Vec::<String>::new());
	assert_eq!(names(world, "\"some te"), Vec::<String>::new());
	assert_eq!(names(world, "local_list[1]."), Vec::<String>::new());
	ok("completion rejects unsafe or invalid receivers");

	// In the middle of a line, and with letters that take two units
	let targets = world.request("completions", json!({ "frameId": frame, "text": "leaf(by) + n", "column": 8 }));
	assert_eq!(targets["targets"][0]["label"], "by");
	assert_eq!(
		(&targets["targets"][0]["start"], &targets["targets"][0]["length"]),
		(&json!(6), &json!(2))
	);
	let targets = world.request("completions", json!({ "frameId": frame, "text": "\"\u{1F600}\" + src.na", "column": 14 }));
	assert_eq!(targets["targets"][0]["label"], "name");
	// The face is two units: quote, face, quote, " + ", "src." are 11
	assert_eq!(
		(&targets["targets"][0]["start"], &targets["targets"][0]["length"]),
		(&json!(12), &json!(2))
	);
	ok("completion replacement range");

	assert_eq!(offered(world, "#d", Some(&frame)), [
		pair("dis", "keyword"),
		pair("disassemble", "keyword")
	]);
	assert_eq!(offered(world, "#", Some(&frame)).len(), 5);
	// Inspection commands are hidden from completion.
	assert_eq!(offered(world, "#s", Some(&frame)), Vec::<(String, String)>::new());
	ok("command completion");

	// No frame: the world's, and nothing of a frame's
	let global = offered(world, "glo", None);
	assert_eq!(global, [
		pair("global", "variable"),
		pair("global_list", "variable"),
		pair("global_number", "variable")
	]);
	assert_eq!(offered(world, "by", None), Vec::<(String, String)>::new());
	ok("global-scope completion");
}

/// Proc bodies in the paused frame of `inside`, and in none
fn bodies(world: &mut World) {
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (body, expected) in cases {
			assert_eq!(world.eval(body), *expected, "{}", body);
		}
	};
	gives(world, &[
		("var/n = 0; for(var/i in 1 to 3) n += i; n", "6"),
		("var/total = 0\nfor(var/x in local_list)\n\ttotal += x\ntotal", "61"),
		("return by * 2", "8"),
		("if(by > 3)\n\treturn \"big\"\nreturn \"small\"", "\"big\""),
		("var/list/L = list()\nfor(var/i in 1 to by)\n\tL += i\nL.Join(\",\")", "\"1,2,3,4\""),
		("var/i = 0\nwhile(i < 10)\n\ti++\n\tif(i == 5) break\ni", "5"),
		// Indented as a whole, as code copied from a proc is
		("\t\tvar/x = by\n\t\tx * 3", "12")
	]);
	ok("proc-body evaluation");

	gives(world, &[
		// src's var, the frame's argument, a global
		("var/x = n + by + global_number\nx", "10"),
		// a global proc, and src's
		("var/r = leaf(1)\nr", "4"),
		("var/got = bump(1)\nn = 1\ngot", "4"),
		// the frame's own vars come before src's
		("var/list/got = local_list\ngot.len", "3")
	]);
	ok("body name resolution");

	// (local_list is list(11, 20, 30) by now)
	// Changes to the frame's vars are written back; the body's own aren't
	// anywhere afterwards. Falling off the end gives `.`, the frame's.
	gives(world, &[("by = 9\nlocal_list[1] = 12\nvar/unused = 0", "null")]);
	assert_eq!(world.argument("by"), "9");
	assert_eq!(world.eval("local_list[1]"), "12");
	gives(world, &[("by = 4; local_list[1] = 11; by", "4"), ("var/q = 3", "null")]);
	assert_eq!(world.argument("by"), "4");
	let frame = world.frame_id();
	let why = world.refused("evaluate", json!({ "expression": "q", "frameId": frame, "context": "repl" }));
	assert_eq!(why, "undefined var: q");
	ok("body writes frame variables");

	let frame = world.frame_id();
	let mut fails = |world: &mut World, cases: &[(&str, &str)]| {
		for (body, expected) in cases {
			for context in ["repl", "watch"] {
				let why = world.refused("evaluate", json!({ "expression": body, "frameId": frame, "context": context }));
				assert!(why.starts_with(expected), "unexpected result for {} in {}: {:?}", body, context, why);
			}
		}
	};
	fails(world, &[
		("var/x = 1\nx = nowhere", "line 2: undefined var: nowhere"),
		("var/x = 1\nx +", "parser error: 2:"),
		("var/x = 0\nreturn by / x", "runtime error: Division by zero"),
		("var/x = 1\nreturn src.other.name", "runtime error: Cannot read null.name"),
		// A body is over when it's answered too
		("var/x = by\nsleep(1)\nreturn x", "evaluation aborted because the code slept")
	]);
	assert_eq!(world.eval("by"), "4");
	assert_eq!(world.frames(0).len(), 3);
	ok("body compile, runtime and sleep errors");

	// The same in a watch; a result that opens doesn't say how to reach
	// what's in it, which would run the body again
	let result = world.evaluate_in(&frame, "var/datum/thing/t = src\nt", "watch");
	assert_eq!(result["result"], "S");
	let inside = world.variables(&result["variablesReference"].clone());
	let n = inside.iter().find(|v| v["name"] == "n").unwrap();
	assert_eq!((&n["value"], &n["evaluateName"]), (&json!("1"), &Value::Null), "{}", n);
	// A hover reads one expression, and runs no body: this one would never end
	for body in ["again:\ngoto again", "var/x = n\nx", "return n"] {
		let why = world.refused("evaluate", json!({ "expression": body, "frameId": frame, "context": "hover" }));
		assert_eq!(why, "hover requires a single expression", "{}", body);
	}
	ok("body watch results and hover rejection");

	assert_eq!(world.eval_global("var/t = 0\nfor(var/i in 1 to 4) t += i\nt"), "10");
	assert_eq!(
		world.eval_global("global_number = 7\nvar/was = global_number\nglobal_number = 5\nwas"),
		"7"
	);
	let why = world.refused("evaluate", json!({ "expression": "var/x = by\nx", "context": "repl" }));
	assert_eq!(why, "line 1: undefined var: by");
	ok("body evaluation without a frame");
}

/// `*(&x)`, a type's var by `::`, and `initial()` of one
fn smaller_things(world: &mut World) {
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (expression, expected) in cases {
			assert_eq!(world.eval(expression), *expected, "{}", expression);
		}
	};
	gives(world, &[
		("*(&by)", "4"),
		("*(&by) = 6", "6"),
		("by", "6"),
		("var/p = &by\n*p = 4\n*(&by)", "4")
	]);
	assert_eq!(world.argument("by"), "4");
	ok("pointer reads and writes");

	// As DreamMaker has them: a type's initial value, a static var's value
	gives(world, &[
		("/datum/thing::name", "\"thing\""),
		("/datum/thing::n + 1", "2"),
		("/datum/thing/sub::name", "\"thing\""),
		("initial(/datum/thing::n)", "1"),
		("src::name", "\"thing\""),
		("initial(src::n)", "1"),
		("/datum/thing::shared", "9"),
		("initial(/datum/thing::shared)", "9"),
		("var/x = /datum/thing::n\nreturn x + initial(src::n)", "2")
	]);
	ok("type defaults in expressions and bodies");
}

pub fn run() {
	let mut world = World::start();
	world.run_to("inside", "/datum/thing/inside", "inside_stop");
	// Each expression and what it gives, in the stopped proc: src is a
	// /datum/thing/sub called "S" with n = 1, by is 4, local_list is
	// list(10, 20, 30)
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (expression, expected) in cases {
			assert_eq!(world.eval(expression), *expected, "{}", expression);
		}
	};

	gives(&mut world, &[
		("1 + 2 * 3", "7"),
		("7 % 4", "3"),
		("2 ** 10", "1024"),
		("\"a\" + \"b\"", "\"ab\""),
		("1 && 0", "0"),
		("!0", "1"),
		("by > 3 ? \"big\" : \"small\"", "\"big\""),
		("5 & 3", "1")
	]);
	ok("arithmetic and logical evaluation");

	gives(&mut world, &[
		("by", "4"),
		("by * 2", "8"),
		("local_list[2]", "20"),
		("length(local_list)", "3"),
		("src", "S"),
		("usr", "null")
	]);
	ok("frame variable evaluation");

	gives(&mut world, &[
		("src.n", "1"),
		("src.name", "\"S\""),
		("src.items[1]", "\"x\""),
		("src.other", "null"),
		("src.type", "/datum/thing/sub"),
		("src.vars[\"n\"]", "1"),
		("initial(src.n)", "1")
	]);
	ok("object, list and initial-value access");

	gives(&mut world, &[
		("global_number", "5"),
		("global.global_number", "5"),
		("global_list[2]", "\"two\""),
		("src.shared", "9"),
		("src.K", "4"),
		("world.maxx", "3")
	]);
	ok("global, static and const evaluation");

	gives(&mut world, &[
		("leaf(4)", "10"),
		("middle(1)", "14"),
		("recurse(3)", "5"),
		("text2num(\"12\") + 1", "13"),
		("istype(src, /datum/thing)", "1"),
		("locate(1, 1, 1)", "turf"),
		("\"[src.name]-[by]\"", "\"S-4\"")
	]);
	ok("proc calls and string interpolation");

	// A name the frame doesn't have is src's var, as it is in the proc's code
	gives(&mut world, &[
		("n", "1"),
		("name", "\"S\""),
		("items[1]", "\"x\""),
		("length(items)", "2"),
		("other", "null"),
		("type", "/datum/thing/sub"),
		("vars[\"n\"]", "1"),
		("n + by", "5"),
		("\"[name] has [n]\"", "\"S has 1\"")
	]);
	gives(&mut world, &[
		("n = 50", "50"),
		("src.n", "50"),
		("n++", "50"),
		("n", "51"),
		("n -= 50", "1")
	]);
	ok("bare src variable reads and writes");

	// Then a global, and a type's static and const vars are those
	gives(&mut world, &[
		("ticks > 0", "1"),
		("early_global", "5"),
		("global_number = 6", "6"),
		("global.global_number", "6"),
		("global_number -= 1", "5"),
		("shared", "9"),
		("K", "4")
	]);
	ok("bare global reads and writes");

	gives(&mut world, &[
		("bump(1)", "4"),
		("n", "3"),
		("n = 1", "1"),
		("inside(0)", "/list {len = 3}")
	]);
	ok("bare src proc calls");

	// The frame's arguments, not the expression's own
	gives(&mut world, &[("args[1]", "4"), ("length(args)", "1"), ("args", "/list {len = 1}")]);
	ok("args resolves to selected frame");

	// A sub's bump() doubles its argument for the parent's, and adds one
	gives(&mut world, &[("src.bump(1)", "4"), ("src.n", "3"), ("src.n = 1", "1")]);
	ok("object proc side effects");

	gives(&mut world, &[
		("new /datum/thing(\"E\")", "E"),
		("list(1, 2, 3)", "/list {len = 3}"),
		("/datum/thing", "/datum/thing"),
		("src?.other?.n", "null"),
		("isnull(src.other)", "1"),
		("null", "null")
	]);
	ok("constructors, paths and safe access");

	gives(&mut world, &[("by = 9", "9"), ("by", "9")]);
	assert_eq!(world.argument("by"), "9");
	gives(&mut world, &[("by++", "9"), ("by", "10"), ("by -= 6", "4")]);
	assert_eq!(world.argument("by"), "4");
	ok("argument assignment and compound operations");

	gives(&mut world, &[("local_list[1] = 11", "11"), ("local_list[1]", "11")]);
	let list = world.variable("Locals", "local_list");
	assert_eq!(world.children(&list)[0].1, "11");
	gives(&mut world, &[("src.n = 50", "50"), ("src.n", "50"), ("src.n = 1", "1")]);
	gives(&mut world, &[
		("global_number = 6", "6"),
		("global_number", "6"),
		("global_number = 5", "5")
	]);
	ok("list, object and global assignment");

	// In the debug console, a watch or a hover
	let frame = world.frame_id();
	for context in ["repl", "watch", "hover"] {
		let watched = world.evaluate_in(&frame, "src", context);
		assert_eq!(watched["result"], "S");
		let inside = crate::client::named(&world.variables(&watched["variablesReference"].clone()));
		assert_eq!(crate::client::value_of(&inside, "n"), "1", "{}", context);
		let watched = world.evaluate_in(&frame, "local_list", context);
		assert_eq!(world.variables(&watched["variablesReference"].clone()).len(), 3);
		let watched = world.evaluate_in(&frame, "by", context);
		assert_eq!((&watched["result"], &watched["variablesReference"]), (&json!("4"), &json!(0)));
	}
	ok("expandable evaluation results");

	let result = world.evaluate_in(&frame, "src", "watch");
	assert_eq!(result["type"], "/datum/thing/sub");
	let inside = world.variables(&result["variablesReference"].clone());
	let n = inside.iter().find(|v| v["name"] == "n").unwrap();
	assert_eq!(n["evaluateName"], "(src).n");
	assert_eq!(world.evaluate_in(&frame, "(src).n + 1", "watch")["result"], "2");
	let result = world.evaluate_in(&frame, "local_list", "watch");
	assert_eq!((&result["type"], &result["indexedVariables"]), (&json!("/list"), &json!(3)));
	assert_eq!(world.evaluate_in(&frame, "by", "watch")["type"], "num");
	assert_eq!(world.evaluate_in(&frame, "name", "watch")["type"], "text");
	ok("result type and access expressions");

	// The mouse on some code reads it, and that's all
	for (expression, shown) in [
		("n", "1"),
		("src.items[2]", "\"y\""),
		("by == 4 && n <= 1", "1"),
		("\"(a = b)\"", "\"(a = b)\"")
	] {
		assert_eq!(world.evaluate_in(&frame, expression, "hover")["result"], shown, "{}", expression);
	}
	for expression in ["bump(1)", "src.bump(1)", "n = 5", "n += 1", "n++", "leaf(n)", "\"[bump(1)]\""] {
		let why = world.refused("evaluate", json!({ "expression": expression, "frameId": frame, "context": "hover" }));
		assert_eq!(why, "hover expression may have side effects", "{}", expression);
	}
	assert_eq!(world.eval("n"), "1");
	ok("hover rejects side effects");

	completions(&mut world);

	// sc_inside called inside(), and has a local of its own
	let caller = world.frames(0)[1]["id"].clone();
	assert_eq!(world.evaluate_in(&caller, "in_caller", "watch")["result"], "77");
	assert_eq!(world.evaluate_in(&caller, "in_caller = 78", "repl")["result"], "78");
	assert_eq!(world.evaluate_in(&caller, "s.name", "watch")["result"], "\"S\"");
	// It's a global proc's frame: there's no src to have an n
	let why = world.refused("evaluate", json!({ "expression": "n", "frameId": caller, "context": "watch" }));
	assert_eq!(why, "undefined var: n");
	ok("caller-frame name resolution");

	assert_eq!(world.eval_global("global_number"), "5");
	assert_eq!(world.eval_global("leaf(1)"), "4");
	let why = world.refused("evaluate", json!({ "expression": "by", "context": "repl" }));
	assert_eq!(why, "undefined var: by");
	ok("global evaluation while paused");

	world.break_on(&["leaf_1"]);
	assert_eq!(world.eval("leaf(1)"), "4");
	assert_eq!(world.eval("middle(1)"), "14");
	assert!(!world.has_event("stopped"), "a breakpoint in an evaluated proc stopped the world");
	world.break_on(&[]);
	ok("no breakpoint stops during evaluation");

	bodies(&mut world);
	smaller_things(&mut world);

	// What can't be evaluated fails, and says why: a client shows that where
	// the value would be
	let mut fails = |world: &mut World, cases: &[(&str, &str)]| {
		let frame = world.frame_id();
		for (expression, expected) in cases {
			for context in ["repl", "watch"] {
				let why = world.refused("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }));
				assert!(
					why.starts_with(expected),
					"unexpected result for {} in {}: {:?}",
					expression,
					context,
					why
				);
			}
		}
	};
	fails(&mut world, &[
		("1 +", "parser error: "),
		("", "parser error: "),
		("by by", "expected end"),
		("5 = by", "expected l-value")
	]);
	ok("expression parse diagnostics");

	fails(&mut world, &[
		("not_a_name_anywhere", "undefined var: not_a_name_anywhere"),
		// A var of another type: src doesn't have it, and it's no global
		("hp", "undefined var: hp"),
		("hp = 3", "undefined var: hp"),
		("not_a_proc_anywhere(1)", "proc not found: /proc/not_a_proc_anywhere"),
		(
			"src.not_a_name_anywhere",
			"runtime error: undefined variable /datum/thing/sub/var/not_a_name_anywhere"
		),
		("/datum/not_a_type", "there's no type /datum/not_a_type"),
		// A statement-only builtin is compiled as a body and cannot sleep here.
		("sleep(1)", "evaluation aborted because the code slept"),
		("..()", "relative calls are not supported")
	]);
	ok("unresolved and unsupported expression diagnostics");

	fails(&mut world, &[
		("src.other.name", "runtime error: Cannot read null.name"),
		("1 / 0", "runtime error: Division by zero"),
		("local_list[9]", "runtime error: list index out of bounds"),
		("src.shared.len", "runtime error: Cannot read 9.len"),
		// (a statement, which is a body)
		("CRASH(\"no\")", "runtime error: no"),
		// An expression is over when it's answered: it can't wait
		("sleeper(1)", "evaluation aborted because the code slept")
	]);
	assert_eq!(world.eval("by"), "4");
	assert_eq!(world.frames(0).len(), 3);
	ok("runtime failure preserves pause state");

	let stale = world.try_request("evaluate", json!({ "expression": "by", "frameId": 99, "context": "watch" }));
	assert_eq!(stale["success"], false, "{}", stale);
	assert!(stale["message"].as_str().unwrap().starts_with("there's no frame 99"), "{}", stale);
	assert!(
		!world.output.contains("[auxtools]"),
		"the debug server had something to say in the console:\n{}",
		world.output
	);
	ok("stale frame errors stay in the response");

	let disassembly = world.eval("#dis");
	assert!(
		disassembly.starts_with("Dism for Proc(\"/datum/thing/proc/inside\")") && disassembly.contains("DbgLine"),
		"#dis said {}",
		disassembly
	);
	let disassembly = world.eval("#dis /proc/leaf");
	assert!(
		disassembly.contains("/proc/leaf") && disassembly.contains("DbgFile \"stepping.dm\""),
		"#dis said {}",
		disassembly
	);
	let help = world.eval("#help");
	assert!(
		help.contains("disassemble") && !help.contains("skin") && !help.contains("script"),
		"#help said {}",
		help
	);
	ok("console disassembly commands");

	// As a client sends its watches: all at once
	let requests: Vec<(&str, serde_json::Value)> = (0..100)
		.map(|i| {
			(
				"evaluate",
				json!({ "expression": format!("by + {}", i), "frameId": frame, "context": "watch" })
			)
		})
		.collect();
	let responses = world.pipeline(&requests);
	for (i, response) in responses.iter().enumerate() {
		assert_eq!(response["body"]["result"], (4 + i).to_string(), "{}", response);
	}
	ok("ordered responses to consecutive evaluations");
	world.resume_through("inside");

	assert_eq!(world.eval_global("global_number + 1"), "6");
	assert_eq!(world.eval_global("global_number = 8"), "8");
	assert_eq!(world.eval_global("global_number"), "8");
	assert_eq!(world.eval_global("leaf(2)"), "6");
	assert!(!world.has_event("stopped"));
	world.run_through("plain");
	ok("evaluation while running");
	world.end();
}
