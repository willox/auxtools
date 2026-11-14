//! DAP restart: reboot in place or relaunch after configuration changes.

use serde_json::{json, Value};

use crate::{client, ok, paths, world::World};

/// Find an unused listening port.
fn free_port() -> u16 {
	std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn process_id(event: &Value) -> u64 {
	event["body"]["systemProcessId"]
		.as_u64()
		.expect("process event is missing its process ID")
}

/// Check the world-ready event, including its `started` flag.
fn announced(world: &mut World, address: &str, started: bool) {
	let told = world.event("auxtoolsWorld")["body"].clone();
	assert_eq!(
		(&told["address"], &told["started"], &told["hosted"]),
		(&json!(address), &json!(started), &json!(false)),
		"{}",
		told
	);
}

/// Check that breakpoints survive restart by stopping in leaf_1 again.
fn breaks_at_leaf(world: &mut World) {
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
}

pub fn run() {
	let port = free_port();
	let mut world = World::launch(json!({ "port": port }));
	let pid = process_id(&world.event("process"));
	world.catch_runtimes(false);
	world.configure();
	assert_eq!(world.eval_global("world.port"), port.to_string());
	ok("configured listening port");
	let address = format!("byond://127.0.0.1:{}", port);
	announced(&mut world, &address, true);
	let joined = world.request("auxtoolsJoin", json!({ "open": false }));
	assert_eq!((&joined["address"], &joined["opened"]), (&json!(address), &json!(false)), "{}", joined);
	ok("world join address");
	world.break_on(&["leaf_1"]);

	world.request("restart", json!({}));
	world.wait_count("dap: world/New", 2);
	assert!(!world.has_event("process"), "reboot unexpectedly replaced the process");
	assert_eq!(client::listed_world("dap_test_host.dmb"), pid);
	assert_eq!(world.eval_global("world.port"), port.to_string());
	announced(&mut world, &address, false);
	breaks_at_leaf(&mut world);
	ok("restart preserves the process");

	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.request("restart", json!({}));
	world.wait_count("dap: world/New", 3);
	announced(&mut world, &address, false);
	breaks_at_leaf(&mut world);
	ok("restart while paused");

	world.request("auxtoolsReboot", json!({}));
	world.wait_count("dap: world/New", 4);
	announced(&mut world, &address, false);
	breaks_at_leaf(&mut world);
	ok("explicit reboot request");

	// Configuration changes require a replacement process.
	let changed = client::launch_arguments(
		&paths::find_byond(),
		&world.marks.file("dap_test_host.dme"),
		json!({ "port": port, "env": { "AUXTOOLS_RESTART_TEST": "1" } })
	);
	world.request("restart", json!({ "arguments": changed }));
	let again = process_id(&world.event("process"));
	assert_ne!(again, pid, "restart did not replace the process");
	world.wait_count("dap: world/New", 5);
	assert_eq!(world.eval_global("world.port"), port.to_string());
	assert_eq!(world.eval_global("world.GetConfig(\"env\", \"AUXTOOLS_RESTART_TEST\")"), "\"1\"");
	announced(&mut world, &address, true);
	breaks_at_leaf(&mut world);
	ok("relaunch after configuration changes");
	world.break_on(&[]);

	// Stop while the world refuses to shut down for a relaunch.
	assert_eq!(world.eval_global("on_shutdown = \"refuse\""), "\"refuse\"");
	let changed_again = client::launch_arguments(
		&paths::find_byond(),
		&world.marks.file("dap_test_host.dme"),
		json!({ "port": port, "env": { "AUXTOOLS_RESTART_TEST": "2" } })
	);
	world.request("restart", json!({ "arguments": changed_again }));
	world.request("terminate", json!({}));
	world.wait_output("Restart cancelled: the world is stopping");
	world.event("terminated");
	assert!(!world.has_event("process"), "the world started again after Stop");
	ok("stop cancels a pending relaunch");
	world.end();
}
