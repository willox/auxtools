//! Runs a tiny world with the debug server loaded and pokes it the way
//! dm-langserver would, to make sure the debugger can't take the game down.

mod client;

// test_runner already knows how to find BYOND, and the debug server's message
// types aren't exported from its cdylib. Both are pulled in as source, and
// each has bits this test never touches.
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{
	net::{Ipv4Addr, TcpListener},
	path::PathBuf,
	process::{Child, Command},
	time::Duration
};

use client::Client;
use paths::ByondCommand;
use server_types::{BreakpointReason, BreakpointSetResult, InstructionRef, ProcRef, Request, Response};

// The debug server names global procs without the /proc part
const BREAKPOINT_TARGET: &str = "/breakpoint_target";
const TICK_MARKER: &str = "/tick_marker";

struct World(Child);

impl Drop for World {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn find_debug_server() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libdebug_server.so");

	#[cfg(windows)]
	path.push("debug_server.dll");

	assert!(path.is_file(), "couldn't find debug_server");
	path
}

fn host_file(name: &str) -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/debug_test_host");
	path.push(name);
	path
}

fn free_port() -> u16 {
	TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

fn start_of(proc: &str) -> InstructionRef {
	InstructionRef {
		proc: ProcRef {
			path: proc.to_owned(),
			override_id: 0
		},
		offset: 0
	}
}

fn set_breakpoint(client: &mut Client, proc: &str, condition: Option<&str>) {
	let response = client.request(Request::BreakpointSet {
		instruction: start_of(proc),
		condition: condition.map(str::to_owned)
	});

	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"couldn't set a breakpoint on {}: {:?}",
		proc,
		response
	);
}

fn eval(client: &mut Client, frame_id: Option<u32>, command: &str) -> String {
	let response = client.request(Request::Eval {
		frame_id,
		command: command.to_owned(),
		context: None
	});

	match response {
		Response::Eval(eval) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// The proc the world is paused in.
fn paused_in(client: &mut Client) -> String {
	match client.request(Request::Stacks) {
		Response::Stacks { stacks } => stacks.into_iter().next().expect("paused with no stacks").name,
		other => panic!("expected Stacks, got {:?}", other)
	}
}

fn expect_breakpoint_in(client: &mut Client, proc: &str) {
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(client), proc);
}

fn pause(client: &mut Client) {
	let response = client.request(Request::Pause);
	assert!(matches!(response, Response::Ack), "expected Ack for Pause, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Pause);
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(host_file("debug_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");

	let port = free_port();

	let _world = World(
		Command::new(paths::find_dreamdaemon())
			.with_byond_paths()
			.env("AUXTOOLS_DEBUG_DLL", find_debug_server())
			.env("AUXTOOLS_DEBUG_PORT", port.to_string())
			.arg(host_file("debug_test_host.dmb"))
			.arg("-trusted")
			.arg("-close")
			.spawn()
			.unwrap()
	);

	let dis_command = format!("#dis {}", BREAKPOINT_TARGET);

	let mut client = Client::connect(port);
	let clean_disassembly = eval(&mut client, None, &dis_command);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: breakpoint hit");

	// VS Code re-runs every watch expression on every stop, so this is the
	// easy one to hit by accident
	assert_eq!(eval(&mut client, Some(0), "breakpoint_target()"), "2");
	println!("ok: evaluating a proc that has a breakpoint in it");

	// numbers are keys in an alist, so walking it by position shows nonsense
	let response = client.request(Request::Eval {
		frame_id: Some(0),
		command: "alist_sample()".to_owned(),
		context: None
	});
	let Response::Eval(alist) = response else {
		panic!("expected the result of alist_sample(), got {:?}", response);
	};
	assert_eq!(alist.value, "/alist {len = 3}");
	let vars = match client.request(Request::Variables {
		vars: alist.variables.expect("an alist should be expandable")
	}) {
		Response::Variables { vars } => vars,
		other => panic!("expected Variables, got {:?}", other)
	};
	let mut shown: Vec<_> = vars.into_iter().map(|var| var.value).collect();
	shown.sort();
	assert_eq!(shown, ["\"a\" = 1", "\"n\" = null", "7 = \"seven\""]);
	println!("ok: an alist shows its key/value pairs");

	assert_eq!(eval(&mut client, None, &dis_command), clean_disassembly);
	println!("ok: disassembly doesn't show the breakpoint");

	let response = client.request(Request::BreakpointUnset {
		instruction: start_of("/proc/does_not_exist")
	});
	assert!(
		matches!(response, Response::BreakpointUnset { success: false }),
		"unsetting a breakpoint on a missing proc got {:?}",
		response
	);
	println!("ok: unsetting a breakpoint on a missing proc");

	// The condition is false, and working that out means calling a proc that
	// has its own breakpoint. The second stop in tick_marker is the one that
	// shows breakpoint_target got skipped.
	set_breakpoint(&mut client, TICK_MARKER, None);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, Some("tick_marker() == 0"));
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	println!("ok: conditional breakpoint that calls a proc with a breakpoint");

	// dm-langserver re-sends every breakpoint without unsetting it first, so
	// this is how a condition gets removed
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	client.resume();
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: re-setting a breakpoint drops its old condition");

	eval(&mut client, None, "start_runtimes()");
	client.resume();
	drop(client);
	// start_runtimes() gives two seconds of runtime errors, wait them out
	std::thread::sleep(Duration::from_secs(3));

	let mut client = Client::connect(port);
	pause(&mut client);
	println!("ok: survived runtime errors with the debugger gone, and it can attach again");

	client.resume();
	client.send(Request::Disconnect);
	drop(client);

	let mut client = Client::connect(port);
	let response = client.request(Request::Stacks);
	assert!(matches!(response, Response::Stacks { .. }), "expected Stacks, got {:?}", response);
	println!("ok: attaching again after a clean detach");

	eval(&mut client, None, "request_reboot()");
	client.wait_closed();
	// The old world hangs up on us a moment before it stops listening. Don't
	// get picked up by its last accept().
	std::thread::sleep(Duration::from_millis(500));

	let mut client = Client::connect(port);
	pause(&mut client);
	println!("ok: attaching again after a world reboot");

	println!("Debug server tests succeeded");
}
