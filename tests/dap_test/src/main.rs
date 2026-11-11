//! Debugs a small world through the launcher, in the Debug Adapter Protocol,
//! and checks what a client sees of each thing the debugger does: breakpoints,
//! stepping, pausing, stack frames, variables, expressions, runtime errors,
//! several sessions at once, reboots, and all of it while the code is hot
//! reloaded.
//!
//! Build the launcher and the debug library first:
//! `cargo build -p auxtools-cli -p debug_server -p dap_test`, then run this
//! from the repository's root with `BYOND_PATH` set. Arguments name the groups
//! to run (see `GROUPS`); none runs them all. `LAUNCHER_TEST_SERVER` and
//! `LAUNCHER_TEST_ARGS` are `launcher_test`'s.

#[allow(dead_code)]
mod client;
mod groups;
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
mod world;

use std::{
	process::Command,
	sync::atomic::{AtomicUsize, Ordering}
};

use paths::ByondCommand;

static CHECKS: AtomicUsize = AtomicUsize::new(0);

/// One thing checked
pub fn ok(what: &str) {
	CHECKS.fetch_add(1, Ordering::Relaxed);
	println!("ok: {}", what);
}

const GROUPS: &[(&str, fn())] = &[
	("basics", groups::basics::run),
	("breakpoints", groups::breakpoints::run),
	("functions", groups::functions::run),
	("conditions", groups::conditions::run),
	("logpoints", groups::logpoints::run),
	("stepping", groups::stepping::run),
	("pausing", groups::pausing::run),
	("frames", groups::frames::run),
	("variables", groups::variables::run),
	("watches", groups::watches::run),
	("expressions", groups::expressions::run),
	("runtimes", groups::runtimes::run),
	("sessions", groups::sessions::run),
	("reboots", groups::reboots::run),
	("reloading", groups::reloading::run),
	("bytecode", groups::bytecode::run)
];

fn main() {
	let wanted: Vec<String> = std::env::args().skip(1).collect();
	for name in &wanted {
		assert!(
			GROUPS.iter().any(|(group, _)| group == name),
			"there's no group {}; the groups are {:?}",
			name,
			GROUPS.iter().map(|(group, _)| *group).collect::<Vec<_>>()
		);
	}

	// Left behind by a run that failed
	let _ = std::fs::remove_file(world::host_file("fail_early"));
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(world::host_file("dap_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");

	for (name, run) in GROUPS {
		if wanted.is_empty() || wanted.iter().any(|w| w == name) {
			println!("--- {}", name);
			run();
		}
	}
	println!("all ok: {} checks", CHECKS.load(Ordering::Relaxed));
}
