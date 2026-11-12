//! Scopes and variables: how each kind of value shows, and what's inside it

use serde_json::{json, Value};

use crate::{
	client::{named, value_of},
	ok,
	world::World
};

/// What kind of value each is, and the expression that reaches it
fn kinds_and_expressions(world: &mut World, locals: &[Value], version: u64) {
	let of = |name: &str| locals.iter().find(|v| v["name"] == name).unwrap().clone();
	for (name, kind) in [
		("whole", "num"),
		("text", "text"),
		("nothing", "null"),
		("path", "path"),
		("proc_path", "proc"),
		("file", "file"),
		("named", "/datum/thing"),
		("T", "/turf"),
		("M", "/mob/player"),
		("the_world", "/world"),
		("plain", "/list"),
		("mx", "/matrix"),
		("pointer", "pointer"),
		("deleted", "null")
	] {
		assert_eq!(of(name)["type"], kind, "{}", name);
	}
	if version >= 516 {
		assert_eq!(of("pairs")["type"], "/alist");
		assert_eq!(of("vec")["type"], "/vector");
	}
	ok("variable type labels");

	// Each local is reached by its name, and gives what it shows
	let frame = world.frame_id();
	for local in locals {
		let (name, value) = (local["name"].as_str().unwrap(), local["value"].as_str().unwrap());
		if name.starts_with("twin") {
			assert!(local["evaluateName"].is_null(), "two locals share a name, and {} has one to itself", name);
			continue;
		}
		assert_eq!(local["evaluateName"], name);
		assert_eq!(world.evaluate_in(&frame, name, "watch")["result"], value, "{}", name);
	}
	ok("local access expressions");

	// Down through objects and lists
	let inside = world.variables(&of("named")["variablesReference"].clone());
	for child in &inside {
		let expression = child["evaluateName"].as_str().unwrap();
		assert_eq!(expression, format!("named.{}", child["name"].as_str().unwrap()));
		assert_eq!(world.evaluate_in(&frame, expression, "watch")["result"], child["value"], "{}", expression);
	}
	let other = inside.iter().find(|v| v["name"] == "other").unwrap();
	let inside = world.variables(&other["variablesReference"].clone());
	let name = inside.iter().find(|v| v["name"] == "name").unwrap();
	assert_eq!(name["evaluateName"], "named.other.name");
	assert_eq!(world.evaluate_in(&frame, "named.other.name", "watch")["result"], "\"B\"");

	let inside = world.variables(&of("plain")["variablesReference"].clone());
	assert_eq!(inside[1]["evaluateName"], "plain[2]");
	assert_eq!(world.evaluate_in(&frame, "plain[2]", "watch")["result"], "\"b\"");
	let inside = world.variables(&of("assoc")["variablesReference"].clone());
	assert_eq!(inside[1]["evaluateName"], "assoc[\"b\"]");
	assert_eq!(world.evaluate_in(&frame, "assoc[\"b\"]", "watch")["result"], "A");
	let pair = world.variables(&inside[1]["variablesReference"].clone());
	assert!(pair[0]["evaluateName"].is_null());
	assert_eq!(pair[1]["evaluateName"], "assoc[\"b\"]");
	let inside = world.variables(&of("nested")["variablesReference"].clone());
	let inner = world.variables(&inside[1]["variablesReference"].clone());
	let innermost = world.variables(&inner[0]["variablesReference"].clone());
	let last = world.variables(&innermost[1]["variablesReference"].clone());
	assert_eq!(last[0]["evaluateName"], "nested[2][\"x\"][1]");
	assert_eq!(world.evaluate_in(&frame, "nested[2][\"x\"][1]", "watch")["result"], "3");
	ok("nested access expressions");

	// Reach entries with escaped literals, object refs or positional lookup.
	let inside = world.variables(&of("keyed")["variablesReference"].clone());
	let expressions: Vec<&str> = inside.iter().map(|v| v["evaluateName"].as_str().unwrap()).collect();
	assert_eq!(expressions[..3], ["keyed[\"a\\[b\\]\"]", "keyed[keyed[2]]", "keyed[keyed[3]]"]);
	assert!(expressions[3].starts_with("keyed[locate(\"\\[0x"), "{}", expressions[3]);
	for (entry, value) in inside.iter().zip(["1", "2", "3", "4"]) {
		let expression = entry["evaluateName"].as_str().unwrap();
		assert_eq!(world.evaluate_in(&frame, expression, "watch")["result"], value, "{}", expression);
		let pair = world.variables(&entry["variablesReference"].clone());
		assert_eq!(pair[1]["evaluateName"], expression);
	}
	if version >= 516 {
		let inside = world.variables(&of("keyed_pairs")["variablesReference"].clone());
		// Alist entries cannot be accessed by position.
		assert!(inside[0]["evaluateName"].is_null(), "{}", inside[0]);
		let expression = inside[1]["evaluateName"].as_str().unwrap();
		assert_eq!(world.evaluate_in(&frame, expression, "watch")["result"], "2", "{}", expression);
	}
	ok("access expressions for awkward keys");

	let arguments = world.scope(&frame, "Arguments");
	let arguments = world.variables(&arguments);
	let expressions: Vec<&str> = arguments.iter().map(|v| v["evaluateName"].as_str().unwrap()).collect();
	assert_eq!(expressions, ["src", "usr", "first", "second", "args[3]"]);
	assert_eq!(world.evaluate_in(&frame, "args[3]", "watch")["result"], "3");
	let globals = world.scope(&frame, "Globals");
	let globals = world.variables(&globals);
	let number = globals.iter().find(|v| v["name"] == "global_number").unwrap();
	assert_eq!(number["evaluateName"], "global.global_number");
	assert_eq!(world.evaluate_in(&frame, "global.global_number", "watch")["result"], "5");
	ok("argument and global access expressions");
}

/// A long list a piece at a time, and whole numbers in hex
fn pieces_and_hex(world: &mut World, big: &Value, locals: &Value) {
	assert_eq!(big["indexedVariables"], 3000);
	let piece = |world: &mut World, start: u64, count: u64| -> Vec<String> {
		let vars = world.request(
			"variables",
			json!({ "variablesReference": big["variablesReference"], "filter": "indexed", "start": start, "count": count })
		);
		vars["variables"]
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v["name"].as_str().unwrap().to_owned())
			.collect()
	};
	let first = piece(world, 0, 100);
	assert_eq!((first.len(), first[0].as_str(), first[99].as_str()), (100, "[1]", "[100]"));
	let last = piece(world, 2950, 100);
	assert_eq!((last.len(), last[0].as_str(), last[49].as_str()), (50, "[2951]", "[3000]"));
	assert_eq!(piece(world, 3000, 100).len(), 0);
	assert_eq!(piece(world, 5000, 100).len(), 0);
	assert_eq!(piece(world, 0, 0).len(), 3000);
	ok("list length and pagination");

	let vars = world.request("variables", json!({ "variablesReference": locals, "format": { "hex": true } }));
	let vars = named(vars["variables"].as_array().unwrap());
	assert_eq!(value_of(&vars, "whole"), "0x2A");
	assert_eq!(value_of(&vars, "past_24_bits"), "0x1000000");
	assert_eq!(value_of(&vars, "fraction"), "1.5");
	assert_eq!(value_of(&vars, "nothing"), "null");
	let frame = world.frame_id();
	let result = world.request(
		"evaluate",
		json!({ "expression": "whole + 213", "frameId": frame, "context": "watch", "format": { "hex": true } })
	);
	assert_eq!(result["result"], "0xFF");
	ok("hexadecimal number formatting");
}

/// Gives the variable `name`, among what `inside` opens to, the value of an
/// expression, and returns it as the launcher says it is then
fn set(world: &mut World, inside: &Value, name: &str, value: &str) -> Value {
	world.request("setVariable", json!({ "variablesReference": inside, "name": name, "value": value }))
}

/// Why that can't be done
fn cannot_set(world: &mut World, inside: &Value, name: &str, value: &str) -> String {
	world.refused("setVariable", json!({ "variablesReference": inside, "name": name, "value": value }))
}

/// Giving variables other values, in the paused frame of values()
fn setting(world: &mut World, version: u64) {
	let frame = world.frame_id();
	let locals = world.scope(&frame, "Locals");
	let arguments = world.scope(&frame, "Arguments");
	let inside = |world: &mut World, name: &str| -> Value { world.variable("Locals", name)["variablesReference"].clone() };

	let now = set(world, &locals, "whole", "50");
	assert_eq!((&now["value"], &now["type"]), (&json!("50"), &json!("num")));
	assert_eq!(world.local("whole"), "50");
	assert_eq!(world.eval("whole + 1"), "51");
	// The value is an expression, in the variable's frame
	assert_eq!(set(world, &locals, "fraction", "whole * 2 + length(args)")["value"], "103");
	assert_eq!(set(world, &locals, "text", "\"new [whole]\"")["value"], "\"new 50\"");
	assert_eq!(set(world, &locals, "nothing", "named")["value"], "A");
	assert_ne!(world.variable("Locals", "nothing")["variablesReference"], 0);
	assert_eq!(set(world, &locals, ".", "list(1, 2)")["indexedVariables"], 2);
	assert_eq!(world.eval("length(.)"), "2");
	ok("local and return-value assignment");

	assert_eq!(set(world, &arguments, "first", "9")["value"], "9");
	assert_eq!(set(world, &arguments, "undefined argument #1", "\"third\"")["value"], "\"third\"");
	assert_eq!(world.eval("\"[first] [args[3]]\""), "\"9 third\"");
	assert_eq!(set(world, &arguments, "usr", "M")["value"], "player");
	assert_eq!(world.eval("usr.hp"), "10");
	set(world, &arguments, "usr", "null");
	ok("argument and usr assignment");

	let named = inside(world, "named");
	assert_eq!(set(world, &named, "n", "7")["value"], "7");
	assert_eq!(set(world, &named, "name", "\"renamed\"")["value"], "\"renamed\"");
	assert_eq!(world.eval("\"[named.name] [named.n]\""), "\"renamed 7\"");
	assert_eq!(world.local("named"), "renamed");
	// Through the object to the one inside it
	let children = world.variables(&named);
	let other = children.iter().find(|v| v["name"] == "other").unwrap()["variablesReference"].clone();
	assert_eq!(set(world, &other, "n", "named.n + 1")["value"], "8");
	assert_eq!(world.eval("named.other.n"), "8");
	// A built-in var takes what BYOND makes of the value
	let mob = inside(world, "M");
	assert_eq!(set(world, &mob, "hp", "3")["value"], "3");
	assert_eq!(set(world, &mob, "name", "\"Bob\"")["value"], "\"Bob\"");
	assert_eq!(world.local("M"), "Bob");
	ok("object variable assignment");

	let plain = inside(world, "plain");
	assert_eq!(set(world, &plain, "[2]", "\"z\"")["value"], "\"z\"");
	assert_eq!(world.eval("plain[2]"), "\"z\"");
	let why = cannot_set(world, &plain, "[9]", "1");
	assert_eq!(why, "the list has no entry [9]");
	// An entry with a value for its key gets another value
	let assoc = inside(world, "assoc");
	assert_eq!(set(world, &assoc, "[1]", "5")["value"], "\"a\" = 5");
	assert_eq!(world.eval("assoc[\"a\"]"), "5");
	let entries = world.variables(&assoc);
	let entry = entries[1]["variablesReference"].clone();
	assert_eq!(set(world, &entry, "value", "6")["value"], "6");
	assert_eq!(world.eval("assoc[\"b\"]"), "6");
	let why = cannot_set(world, &entry, "key", "\"c\"");
	assert!(why.starts_with("list keys cannot be edited"), "{}", why);
	if version >= 516 {
		let pairs = inside(world, "pairs");
		let listed = world.variable("Locals", "pairs");
		let entries = world.children(&listed);
		let first = entries.iter().position(|(_, value)| value == "\"a\" = 1").unwrap();
		assert_eq!(set(world, &pairs, &format!("[{}]", first + 1), "11")["value"], "\"a\" = 11");
		assert_eq!(world.eval("pairs[\"a\"]"), "11");
	}
	ok("list entry assignment");

	let globals = world.scope(&frame, "Globals");
	assert_eq!(set(world, &globals, "global_number", "whole")["value"], "50");
	assert_eq!(world.eval("global_number"), "50");
	set(world, &globals, "global_number", "5");
	let constants = world.variable("Globals", "(BYOND's)")["variablesReference"].clone();
	assert_eq!(cannot_set(world, &constants, "NORTH", "2"), "BYOND's constants can't be changed");
	ok("global assignment and immutable builtins");

	for (inside, name, value, why) in [
		(&locals, "whole", "1 +", "parser error: "),
		(&locals, "whole", "nowhere_at_all", "undefined var: nowhere_at_all"),
		(&locals, "whole", "1 / 0", "runtime error: Division by zero"),
		(&locals, "not_a_local", "1", "the frame has no local called not_a_local"),
		(&locals, "twin", "1", "the proc has more than one local called twin"),
		(&arguments, "third", "1", "the frame has no argument called third"),
		(&named, "not_a_var", "1", "runtime error: ")
	] {
		let said = cannot_set(world, inside, name, value);
		assert!(said.starts_with(why), "unexpected assignment result for {} = {}: {:?}", name, value, said);
	}
	assert_eq!(world.local("whole"), "50");
	ok("assignment diagnostics");

	// An expression that names a place, as a watch is set
	let now = world.request(
		"setExpression",
		json!({ "expression": "named.n", "value": "whole + 1", "frameId": frame })
	);
	assert_eq!((&now["value"], &now["type"]), (&json!("51"), &json!("num")));
	assert_eq!(world.eval("named.n"), "51");
	let now = world.request(
		"setExpression",
		json!({ "expression": "plain[1]", "value": "list(\"x\")", "frameId": frame })
	);
	assert_eq!(now["indexedVariables"], 1);
	assert_eq!(world.eval("plain[1][1]"), "\"x\"");
	let why = world.refused("setExpression", json!({ "expression": "whole + 1", "value": "3", "frameId": frame }));
	assert_eq!(why, "expected l-value");
	ok("setExpression target validation");

	// What was listed before the world carried on is gone
	world.step("next");
	assert_eq!(
		cannot_set(world, &locals, "whole", "1"),
		"variable reference expired after execution resumed"
	);
	ok("stale assignment references");
}

/// The world's BYOND version
fn byond_version(world: &mut World) -> u64 {
	world.eval_global("world.byond_version").parse().unwrap()
}

pub fn run() {
	let mut world = World::start();
	let version = byond_version(&mut world);
	world.run_to("values", "/values", "values_stop");

	let frame = world.frame_id();
	let scopes = world.request("scopes", json!({ "frameId": frame }));
	let names: Vec<&str> = scopes["scopes"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
	assert_eq!(names, ["Locals", "Arguments", "Globals"]);
	ok("frame variable scopes");

	let locals_reference = world.scope(&frame, "Locals");
	let variables = world.variables(&locals_reference);
	let locals = named(&variables);
	let local = |name: &str| value_of(&locals, name).to_owned();
	let variable = |name: &str| -> Value { variables.iter().find(|v| v["name"] == name).unwrap().clone() };
	let opens = |name: &str| variable(name)["variablesReference"] != 0;

	assert_eq!(locals[0], (".".to_owned(), "\"dot\"".to_owned()));
	assert_eq!(locals[1].0, "whole");
	ok("local declaration order");

	assert_eq!(local("whole"), "42");
	assert_eq!(local("fraction"), "1.5");
	assert_eq!(local("negative"), "-0.25");
	// BYOND's numbers are 32-bit floats
	assert_eq!(local("past_24_bits"), "16777216");
	assert_eq!(local("huge"), "1000000000000000000000000000000");
	assert_eq!(local("infinite"), "inf");
	ok("numeric value formatting");

	assert_eq!(local("text"), r#""hello \"quoted\"\nline two""#);
	assert_eq!(local("empty_text"), r#""""#);
	ok("escaped string formatting");

	assert_eq!(local("nothing"), "null");
	assert_eq!(local("deleted"), "null");
	assert!(!opens("nothing") && !opens("whole") && !opens("text"));
	ok("null and deleted object values");

	assert_eq!(local("path"), "/datum/thing");
	assert_eq!(local("proc_path"), "/proc/leaf");
	assert_eq!(local("file"), "values.dm");
	assert!(!opens("path") && !opens("proc_path") && !opens("file"));
	ok("non-expandable paths and resources");

	// A datum shows as its name, if it has one
	assert_eq!(local("named"), "A");
	assert_eq!(local("unnamed"), "/datum/made");
	let inside = world.children(&variable("named"));
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing".to_owned()));
	let children = world.variables(&variable("named")["variablesReference"].clone());
	assert_eq!(children[0]["variablesReference"], 0, "an object's type opens");
	let names: Vec<String> = inside[1..].iter().map(|(name, _)| name.to_lowercase()).collect();
	let mut sorted = names.clone();
	sorted.sort();
	assert_eq!(names, sorted, "an object's vars aren't sorted");
	assert_eq!(value_of(&inside, "name"), "\"A\"");
	assert_eq!(value_of(&inside, "n"), "1");
	assert_eq!(value_of(&inside, "other"), "B");
	assert_eq!(value_of(&inside, "items"), "/list {len = 3}");
	assert_eq!(value_of(&inside, "K"), "4");
	ok("datum variables sorted by name");

	// Down through an object's var to another object
	let children = world.variables(&variable("named")["variablesReference"].clone());
	let other = children.iter().find(|v| v["name"] == "other").unwrap();
	let inside = world.children(other);
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing/sub".to_owned()));
	assert_eq!(value_of(&inside, "name"), "\"B\"");
	ok("nested object expansion");

	for (name, text, type_path) in [
		("T", "turf", "/turf"),
		("O", "obj", "/obj"),
		("M", "player", "/mob/player"),
		("A", "area", "/area")
	] {
		assert_eq!(local(name), text);
		let inside = world.children(&variable(name));
		assert_eq!(inside[0], ("type".to_owned(), type_path.to_owned()));
	}
	let mob = world.children(&variable("M"));
	assert_eq!(value_of(&mob, "hp"), "10");
	assert_eq!(value_of(&mob, "loc"), "turf");
	ok("atom built-in and user variables");

	assert_eq!(local("the_world"), "world");
	let inside = world.children(&variable("the_world"));
	assert_eq!(value_of(&inside, "maxx"), "3");
	assert_eq!(value_of(&inside, "byond_version"), version.to_string());
	ok("world variable expansion");

	assert_eq!(local("empty"), "/list {len = 0}");
	assert_eq!(world.children(&variable("empty")).len(), 0);
	assert_eq!(local("plain"), "/list {len = 4}");
	let plain: Vec<(&str, &str)> = vec![("[1]", "1"), ("[2]", "\"b\""), ("[3]", "null"), ("[4]", "A")];
	let inside = world.children(&variable("plain"));
	assert_eq!(inside.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect::<Vec<_>>(), plain);
	ok("one-based list entries");

	let children = world.variables(&variable("assoc")["variablesReference"].clone());
	assert_eq!(named(&children), [
		("[1]".to_owned(), "\"a\" = 1".to_owned()),
		("[2]".to_owned(), "\"b\" = A".to_owned()),
		// A key with no value is no different from an element
		("[3]".to_owned(), "\"c\"".to_owned())
	]);
	assert_eq!(world.children(&children[1]), [
		("key".to_owned(), "\"b\"".to_owned()),
		("value".to_owned(), "A".to_owned())
	]);
	ok("associative entry expansion");

	let children = world.variables(&variable("nested")["variablesReference"].clone());
	assert_eq!(world.children(&children[0]), [
		("[1]".to_owned(), "1".to_owned()),
		("[2]".to_owned(), "2".to_owned())
	]);
	let inner = world.variables(&children[1]["variablesReference"].clone());
	assert_eq!(named(&inner), [("[1]".to_owned(), "\"x\" = /list {len = 1}".to_owned())]);
	ok("nested lists");

	let inside = world.children(&variable("dupes"));
	assert_eq!(inside.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["\"a\"", "\"a\"", "1", "1"]);
	let inside = world.children(&variable("big"));
	assert_eq!(inside.len(), 3000);
	assert_eq!(inside[2999], ("[3000]".to_owned(), "null".to_owned()));
	ok("duplicate entries and large lists");

	let inside = world.children(&variable("var_list"));
	assert!(inside.iter().any(|(_, value)| value == "\"n\" = 1"), "{:?}", inside);
	assert!(inside.iter().any(|(_, value)| value == "\"name\" = \"A\""), "{:?}", inside);
	let inside = world.children(&variable("contents"));
	assert_eq!(inside.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["obj", "player"]);
	assert_eq!(local("overlays"), "/list {len = 0}");
	let inside = world.children(&variable("verbs"));
	assert_eq!(inside, [("[1]".to_owned(), "/mob/player/verb/shout".to_owned())]);
	assert_eq!(local("world_contents"), "/list {len = 12}");
	ok("BYOND built-in lists");

	let inside = world.children(&variable("mx"));
	assert_eq!(local("mx"), "/matrix");
	assert_eq!(["a", "b", "c", "d", "e", "f"].map(|name| value_of(&inside, name).to_owned()), [
		"1", "0", "0", "0", "1", "0"
	]);
	assert_eq!(local("re"), "a+");
	assert_eq!(value_of(&world.children(&variable("re")), "name"), "\"a+\"");
	assert_eq!(local("error"), "boom");
	let inside = world.children(&variable("error"));
	assert_eq!((value_of(&inside, "file"), value_of(&inside, "line")), ("\"file.dm\"", "3"));
	assert_eq!(local("img"), "/image");
	assert_eq!(value_of(&world.children(&variable("img")), "type"), "/image");
	assert_eq!(local("appearance"), "appearance");
	ok("matrix, regex and appearance variables");

	assert_eq!(local("save"), "dap_test.sav");
	let inside = world.children(&variable("save"));
	assert_eq!(inside[0], ("type".to_owned(), "/savefile".to_owned()));
	assert_eq!((value_of(&inside, "name"), value_of(&inside, "cd")), ("\"dap_test.sav\"", "\"/\""));
	assert_eq!(value_of(&inside, "dir"), "/list {len = 0}");
	assert!(!world.output.contains("BUG:"), "BYOND complained:\n{}", world.output);
	ok("savefile variables without reference errors");

	assert_eq!(local("pointer"), "&whole");
	ok("pointer display");

	if version >= 516 {
		assert_eq!(local("pairs"), "/alist {len = 2}");
		let mut inside: Vec<String> = world.children(&variable("pairs")).into_iter().map(|(_, value)| value).collect();
		inside.sort();
		assert_eq!(inside, ["\"a\" = 1", "7 = \"seven\""]);
		assert_eq!(local("vec"), "vector(1,2,3)");
		let inside = world.children(&variable("vec"));
		assert_eq!(
			inside[..4],
			[("x", "1"), ("y", "2"), ("z", "3"), ("len", "3")].map(|(n, v)| (n.to_owned(), v.to_owned()))
		);
		assert_eq!(local("pix"), "pixloc(1,1,1)");
		assert_eq!(value_of(&world.children(&variable("pix")), "loc"), "turf");
		let inside = world.children(&variable("this_proc"));
		assert_eq!(value_of(&inside, "proc"), "/proc/values");
		assert_eq!(value_of(&inside, "file"), "\"values.dm\"");
		assert_eq!(local("filters"), "/list {len = 0}");
		ok("516 value types");
	} else {
		println!("(BYOND {} has no alists, vectors, pixlocs or callees to show)", version);
	}

	assert_eq!(local("twin"), "1");
	assert_eq!(local("twin #1"), "2");
	ok("duplicate local names");

	assert_eq!(world.scope_values("Arguments"), [
		("src".to_owned(), "null".to_owned()),
		("usr".to_owned(), "null".to_owned()),
		("first".to_owned(), "1".to_owned()),
		("second".to_owned(), "\"two\"".to_owned()),
		// values() was called with one more than it declares
		("undefined argument #1".to_owned(), "3".to_owned())
	]);
	ok("src, usr and extra arguments");

	let globals = world.scope_values("Globals");
	assert_eq!(value_of(&globals, "global_number"), "5");
	assert_eq!(value_of(&globals, "global_list"), "/list {len = 3}");
	assert_eq!(value_of(&globals, "early_global"), "5");
	// A type's static and const vars are globals
	assert_eq!(value_of(&globals, "shared"), "9");
	assert_eq!(value_of(&globals, "K"), "4");
	ok("globals include statics and consts");

	// The hundred or so BYOND declares are apart, in the last row
	assert!(!globals.iter().any(|(name, _)| name == "NORTH" || name == "MOB_LAYER"), "{:?}", globals);
	let last = world.variable("Globals", "(BYOND's)");
	assert!(last["value"].as_str().unwrap().ends_with(" constants"), "{}", last);
	assert_eq!(globals.last().unwrap().0, "(BYOND's)");
	let constants = world.children(&last);
	assert_eq!(value_of(&constants, "NORTH"), "1");
	assert_eq!(value_of(&constants, "MOB_LAYER"), "4");
	assert!(!constants.iter().any(|(name, _)| name == "global_number"));
	let scopes = world.request("scopes", json!({ "frameId": frame }));
	let expensive: Vec<bool> = scopes["scopes"].as_array().unwrap().iter().map(|s| s["expensive"] == true).collect();
	assert_eq!(expensive, [false, false, true], "expected only the globals scope to be lazy");
	ok("lazy globals and separate builtins");

	kinds_and_expressions(&mut world, &variables, version);
	pieces_and_hex(&mut world, &variable("big"), &locals_reference);

	for reference in [-5, 0, 99_999] {
		let vars = world.request("variables", json!({ "variablesReference": reference }));
		assert_eq!(vars["variables"].as_array().unwrap().len(), 0);
	}
	assert_eq!(world.local("whole"), "42");
	ok("unknown variable references");

	// whole += length(args)
	world.step("next");
	assert_eq!(world.local("whole"), "45");
	assert_eq!(world.eval("whole = 7"), "7");
	assert_eq!(world.local("whole"), "7");
	ok("updated locals after stepping and evaluation");

	setting(&mut world, version);
	world.resume_through("values");

	world.run_to("inside", "/datum/thing/inside", "inside_stop");
	let arguments = world.scope_values("Arguments");
	assert_eq!((value_of(&arguments, "src"), value_of(&arguments, "by")), ("S", "4"));
	let src = world.variable("Arguments", "src");
	let inside = world.children(&src);
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing/sub".to_owned()));
	assert_eq!(value_of(&inside, "items"), "/list {len = 2}");
	ok("src object expansion");

	let list = world.variable("Locals", "local_list");
	assert_eq!(world.children(&list).iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), [
		"10", "20", "30"
	]);
	world.eval("local_list += 40");
	let list = world.variable("Locals", "local_list");
	assert_eq!(list["value"], "/list {len = 4}");
	assert_eq!(world.children(&list)[3], ("[4]".to_owned(), "40".to_owned()));
	world.resume_through("inside");
	ok("list mutation reflected in inspection");
	world.end();
}
