//! Data breakpoints: the world pauses when what's at a place changes

use serde_json::{json, Value};

use crate::{client::named, ok, world::World};

/// Asks what watching a variable (`name` inside `container`) or an
/// expression in the paused frame would watch
fn info(world: &mut World, container: Option<&Value>, name: &str) -> Value {
	let frame = world.frame_id();
	match container {
		Some(container) => world.request("dataBreakpointInfo", json!({ "variablesReference": container, "name": name })),
		None => world.request("dataBreakpointInfo", json!({ "name": name, "frameId": frame }))
	}
}

/// Sets the client's data breakpoints, each `(dataId, condition, hit count)`
fn watch(world: &mut World, wanted: &[(&Value, &str, &str)]) -> Vec<Value> {
	let breakpoints: Vec<Value> = wanted
		.iter()
		.map(|(data, condition, hit)| {
			let mut breakpoint = json!({ "dataId": data, "accessType": "write" });
			if !condition.is_empty() {
				breakpoint["condition"] = json!(condition);
			}
			if !hit.is_empty() {
				breakpoint["hitCondition"] = json!(hit);
			}
			breakpoint
		})
		.collect();
	let set = world.request("setDataBreakpoints", json!({ "breakpoints": breakpoints }));
	set["breakpoints"].as_array().unwrap().clone()
}

/// Lets the world carry on from each stop until the scenario is over, and
/// returns what each stop said: why, the proc, its description, the ids
fn stops(world: &mut World, scenario: &str) -> Vec<(String, String, String, Vec<u64>)> {
	let mut seen = vec![];
	let over = format!("dap: finished {}\n", scenario);
	let finished = world.count(&over);
	world.resume();
	let deadline = std::time::Instant::now() + crate::client::TIMEOUT;
	loop {
		if let Some(event) = world.launcher.event_within("stopped", std::time::Duration::from_millis(20)) {
			let frame = world.top_frame();
			let ids = event["body"]["hitBreakpointIds"]
				.as_array()
				.map(|ids| ids.iter().filter_map(Value::as_u64).collect())
				.unwrap_or_default();
			seen.push((
				event["body"]["reason"].as_str().unwrap_or_default().to_owned(),
				format!("{}:{}", frame["name"].as_str().unwrap_or_default(), frame["line"]),
				event["body"]["description"].as_str().unwrap_or_default().to_owned(),
				ids
			));
			world.resume();
		} else if world.count(&over) > finished {
			return seen;
		}
		assert!(std::time::Instant::now() < deadline, "timed out waiting for {}", scenario);
	}
}

/// What the launcher says of a breakpoint whose state changed: its id,
/// whether it's set, and why not
fn changed(world: &mut World) -> (u64, bool, String) {
	let event = world.event("breakpoint");
	assert_eq!(event["body"]["reason"], "changed", "{}", event);
	let breakpoint = &event["body"]["breakpoint"];
	(
		breakpoint["id"].as_u64().unwrap(),
		breakpoint["verified"].as_bool().unwrap(),
		breakpoint["message"].as_str().unwrap_or_default().to_owned()
	)
}

fn id(breakpoint: &Value) -> u64 {
	assert_eq!(breakpoint["verified"], true, "it wasn't set: {}", breakpoint);
	breakpoint["id"].as_u64().expect("a breakpoint with no id")
}

/// What a pointer is shown as, and what it opens to
fn pointers(world: &mut World) {
	world.run_to("values", "/values", "values_stop");
	let pointer = world.variable("Locals", "pointer");
	assert_eq!((&pointer["value"], &pointer["type"]), (&json!("&whole"), &json!("pointer")));
	let inside = world.variables(&pointer["variablesReference"]);
	assert_eq!(inside.len(), 1, "{:?}", inside);
	assert_eq!(
		(&inside[0]["name"], &inside[0]["value"], &inside[0]["evaluateName"]),
		(&json!("*"), &json!("42"), &json!("*pointer"))
	);
	assert_eq!(world.eval("*pointer"), "42");
	assert_eq!(world.eval("*pointer + 1"), "43");
	ok("pointer expansion and evaluation");

	// A pointer to an object's var names it; one to a var of the
	// expression's own code names nothing once that has run
	assert_eq!(world.eval("&named.n"), "&A (/datum/thing).n");
	assert_eq!(world.eval("&whole"), "a pointer to a var that's gone");
	world.resume_through("values");
	ok("pointer lifetime after evaluation");
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();
	pointers(&mut world);

	world.run_to("watch", "/sc_watch", "watch_start");
	let frame = world.frame_id();
	let locals = world.scope(&frame, "Locals");
	let globals = world.scope(&frame, "Globals");
	let kept = world.variables(&locals).into_iter().find(|v| v["name"] == "kept").unwrap()["variablesReference"].clone();
	let asked = [
		info(&mut world, Some(&locals), "steps"),
		info(&mut world, Some(&globals), "watched_number"),
		info(&mut world, None, "watched_list[2]"),
		info(&mut world, None, "watched_pairs[\"two\"]"),
		info(&mut world, Some(&kept), "name"),
		info(&mut world, None, "kept.n")
	];
	let names: Vec<&str> = asked.iter().map(|a| a["description"].as_str().unwrap()).collect();
	assert_eq!(names, [
		"steps",
		"watched_number",
		"watched_list[2]",
		"watched_pairs[\"two\"]",
		"kept.name",
		"kept.n"
	]);
	for answer in &asked {
		assert!(answer["dataId"].is_string(), "{}", answer);
		assert_eq!(
			(&answer["accessTypes"], &answer["canPersist"]),
			(&json!(["write"]), &json!(false)),
			"{}",
			answer
		);
	}
	ok("supported watch targets");

	let constants = world.variables(&globals).into_iter().find(|v| v["name"] == "(BYOND's)").unwrap()["variablesReference"].clone();
	for (container, name, why) in [
		(None, "src", "select a variable of this value to watch"),
		(Some(&constants), "NORTH", "BYOND's constants don't change"),
		(None, "steps + 1", ""),
		(None, "nothing_of_the_kind", "")
	] {
		let answer = info(&mut world, container, name);
		assert!(answer["dataId"].is_null(), "{} can be watched: {}", name, answer);
		let said = answer["description"].as_str().unwrap();
		assert!(!said.is_empty() && (why.is_empty() || said == why), "{}: {}", name, said);
	}
	ok("invalid watch target diagnostics");

	let wanted: Vec<(&Value, &str, &str)> = asked.iter().map(|a| (&a["dataId"], "", "")).collect();
	let set = watch(&mut world, &wanted);
	let ids: Vec<u64> = set.iter().map(id).collect();
	let (steps, number, list, pair, name, n) = (ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]);

	// Each stop is on the instruction after the change: the next line,
	// in the proc that made it
	let seen = stops(&mut world, "watch");
	let stop = |proc: &str, marker: &str, what: &str, id: u64| {
		(
			"data breakpoint".to_owned(),
			format!("{}:{}", proc, at.line(marker)),
			what.to_owned(),
			vec![id]
		)
	};
	assert_eq!(seen, [
		stop("/sc_watch", "watch_call", "steps changed: null → 1", steps),
		stop("/change_number", "change_number_after", "watched_number changed: 1 → 3", number),
		stop("/sc_watch", "watch_pair", "watched_list[2] changed: \"b\" → \"B\"", list),
		stop("/sc_watch", "watch_object", "watched_pairs[\"two\"] changed: 2 → 22", pair),
		stop("/datum/thing/rename", "rename_after", "kept.name changed: \"kept\" → \"renamed\"", name),
		// (kept.n = kept.n changes nothing)
		stop("/sc_watch", "watch_sleep", "steps changed: 1 → 11", steps),
		stop("/change_number", "change_number_after", "watched_number changed: 3 → 6", number),
		stop("/sc_watch", "watch_del", "steps changed: 11 → 111", steps)
	]);
	ok("stop after a watched value changes");
	ok("same-value writes do not stop");
	ok("frame watch survives sleep");
	assert!(
		world.output.contains("watched_number changed from 1 to 3\n"),
		"the console didn't say what changed"
	);

	// The object was deleted, and the proc returned
	let mut over = vec![changed(&mut world), changed(&mut world), changed(&mut world)];
	over.sort();
	let mut expected = vec![
		(steps, false, "watched proc returned".to_owned()),
		(name, false, "watch target no longer exists".to_owned()),
		(n, false, "watch target no longer exists".to_owned()),
	];
	expected.sort();
	assert_eq!(over, expected);
	assert!(
		world.output.contains("No longer watching steps: watched proc returned"),
		"the console didn't say a watch was over"
	);
	ok("watch expiration notifications");

	// What's still watched stays as it is when another is added
	assert_eq!(world.eval_global("watched_number = 1"), "1");
	assert!(!world.has_event("stopped"), "a value the debugger set paused the world");
	world.run_to("watch", "/sc_watch", "watch_start");
	let again = info(&mut world, None, "steps");
	let set = watch(&mut world, &[(&asked[1]["dataId"], "", ""), (&again["dataId"], "", "== 2")]);
	assert_eq!(id(&set[0]), number, "the watch that was there was set again");
	let steps = id(&set[1]);
	assert_eq!(stops(&mut world, "watch"), [
		stop("/change_number", "change_number_after", "watched_number changed: 1 → 3", number),
		stop("/sc_watch", "watch_sleep", "steps changed: 1 → 11", steps),
		stop("/change_number", "change_number_after", "watched_number changed: 3 → 6", number)
	]);
	assert_eq!(changed(&mut world), (steps, false, "watched proc returned".to_owned()));
	ok("debugger writes, retained watches and hit counts");

	// An expression in no frame can be asked about while the world runs.
	// `value` and `old` in a condition are what's there now and what was.
	world.eval_global("watched_number = 1");
	let anywhere = world.request("dataBreakpointInfo", json!({ "name": "watched_number" }));
	assert_eq!(anywhere["description"], "watched_number");
	let set = watch(&mut world, &[(&anywhere["dataId"], "value > 4 && old == 3", "")]);
	let number = id(&set[0]);
	world.run("watch");
	let stop_now = world.stop();
	assert_eq!(
		(stop_now.reason.as_str(), stop_now.line, &stop_now.hit),
		("data breakpoint", at.line("change_number_after"), &vec![number])
	);
	assert_eq!(world.eval("watched_number"), "6");
	world.resume_through("watch");
	ok("global watch condition bindings");

	// A condition that fails with an error isn't a change to pause for, and
	// the console says so once: watched_number goes to 3, then 6
	world.eval_global("watched_number = 1");
	watch(&mut world, &[(&anywhere["dataId"], "value.len > 0", "")]);
	world.run_through("watch");
	let said = "[auxtools] The condition (value.len > 0) of the watch on watched_number failed; skipping affected changes (reported once): runtime \
	            error: Cannot read 3.len";
	assert_eq!(world.count(said), 1, "expected exactly one diagnostic matching {:?}", said);
	assert_eq!(world.count("(reported once)"), 1, "diagnostic was reported more than once");
	ok("watch condition errors reported once");

	// Stepping over the call that makes the change
	world.eval_global("watched_number = 1");
	watch(&mut world, &[(&anywhere["dataId"], "", "")]);
	world.run_to("watch", "/sc_watch", "watch_call");
	let stop_now = world.step("next");
	assert_eq!((stop_now.reason.as_str(), stop_now.proc.as_str()), ("data breakpoint", "/change_number"));
	watch(&mut world, &[]);
	world.resume_through("watch");
	world.run_through("watch");
	ok("watch interrupts step and can be removed");

	// What was asked about at one pause is something of that pause
	world.run_to("watch", "/sc_watch", "watch_start");
	let of_this_pause = info(&mut world, None, "steps");
	world.resume_through("watch");
	world.run_to("watch", "/sc_watch", "watch_start");
	let set = watch(&mut world, &[(&of_this_pause["dataId"], "", "")]);
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "watch target expired; select it again");
	watch(&mut world, &[]);
	world.resume_through("watch");
	ok("pause-local watch target expiration");

	// Thirty sleeps, with spawned copies of the proc's frame (which have
	// the var too, each its own) and other procs coming and going
	world.run_to("watch_long", "/sc_watch_long", "watch_long_start");
	let patient = info(&mut world, None, "patient");
	let set = watch(&mut world, &[(&patient["dataId"], "", "")]);
	let patient = id(&set[0]);
	let seen = stops(&mut world, "watch_long");
	assert_eq!(seen.iter().map(|(_, _, what, _)| what.as_str()).collect::<Vec<_>>(), [
		"patient changed: null → 1",
		"patient changed: 1 → 2"
	]);
	assert_eq!(changed(&mut world), (patient, false, "watched proc returned".to_owned()));
	ok("frame watches exclude spawned copies");

	// The next world: a global is watched in it too, but what was something
	// of a pause in the old one isn't found again
	world.run_to("watch", "/sc_watch", "watch_start");
	let frame = world.frame_id();
	let globals = world.scope(&frame, "Globals");
	let shown = info(&mut world, Some(&globals), "watched_list");
	let set = watch(&mut world, &[(&anywhere["dataId"], "", ""), (&shown["dataId"], "", "")]);
	let (number, shown) = (id(&set[0]), id(&set[1]));
	assert_eq!(stops(&mut world, "watch").len(), 2);
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	assert_eq!(
		changed(&mut world),
		(shown, false, "watch expired after reboot; select it again".to_owned())
	);
	world.wait_output("The world restarted");
	world.run("watch");
	let stop_now = world.stop();
	assert_eq!((stop_now.reason.as_str(), &stop_now.hit), ("data breakpoint", &vec![number]));
	watch(&mut world, &[]);
	world.resume_through("watch");
	assert!(!world.has_event("breakpoint"), "unexpected additional watch update");
	ok("restore only global watches after reboot");
	world.end();
}
