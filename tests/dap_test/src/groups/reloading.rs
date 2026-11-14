//! Debugging a world while its code is hot reloaded

use std::{path::PathBuf, process::Command};

use serde_json::{json, Value};

use crate::{
	client::value_of,
	ok,
	paths::ByondCommand,
	world::{host_file, World}
};

/// A copy of the world's code, to edit, in a directory of its own
pub fn copy_of_the_world(name: &str) -> PathBuf {
	fn copy(from: &std::path::Path, to: &std::path::Path) {
		std::fs::create_dir_all(to).unwrap();
		for entry in std::fs::read_dir(from).unwrap() {
			let path = entry.unwrap().path();
			let copy_to = to.join(path.file_name().unwrap());
			if path.is_dir() {
				copy(&path, &copy_to);
			} else if path
				.extension()
				.is_some_and(|e| ["dm", "dme", "dmb", "rsc", "dmi"].contains(&e.to_str().unwrap_or_default()))
			{
				std::fs::copy(&path, copy_to).unwrap();
			}
		}
	}
	let dir = std::env::temp_dir().join(format!("dap_test_{}-{}", name, std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	copy(&host_file(""), &dir);
	dir
}

/// The directory's long path. Windows can give the temporary directory by
/// an 8.3 name (`RUNNER~1`). DreamMaker expands the path of an included
/// `.dmf` or `.dms`, so under such a directory it rejects the file as
/// outside the world's ("invalid file") and builds the world without it.
#[cfg(windows)]
pub fn long_name(dir: PathBuf) -> PathBuf {
	use std::os::windows::ffi::{OsStrExt, OsStringExt};
	#[link(name = "kernel32")]
	extern "system" {
		fn GetLongPathNameW(short: *const u16, long: *mut u16, len: u32) -> u32;
	}
	let short: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
	let mut long = vec![0u16; 1024];
	// SAFETY: `short` ends in a NUL, and `long` has room for `len` units
	let len = unsafe { GetLongPathNameW(short.as_ptr(), long.as_mut_ptr(), long.len() as u32) } as usize;
	if len == 0 || len > long.len() {
		return dir;
	}
	PathBuf::from(std::ffi::OsString::from_wide(&long[..len]))
}

#[cfg(not(windows))]
pub fn long_name(dir: PathBuf) -> PathBuf {
	dir
}

/// How many reloads the launcher has said were done
fn reloads(output: &str) -> usize {
	output
		.lines()
		.filter(|line| line.starts_with("Hot reload: ") && line.ends_with(" ms"))
		.count()
}

/// Changes a file, and waits for the world to have the change
pub fn edit(world: &mut World, file: &str, old: &str, new: &str) {
	let before = reloads(&world.output);
	world.marks.edit(file, old, new);
	world.wait_until("the hot reload", |output| reloads(output) > before);
}

/// Waits for the launcher to say where a breakpoint is now, and returns its
/// line and whether it's set
fn moved(world: &mut World) -> (u64, bool) {
	let event = world.event("breakpoint");
	assert_eq!(event["body"]["reason"], "changed", "{}", event);
	let breakpoint = &event["body"]["breakpoint"];
	(breakpoint["line"].as_u64().unwrap(), breakpoint["verified"].as_bool().unwrap())
}

/// Reboot and wait for the `count`th world/New and restored reloads.
fn reboot(world: &mut World, count: usize) {
	let restarts = world.output.matches("The world restarted").count();
	world.run("reboot");
	world.wait_count("dap: world/New", count);
	world.wait_until("the reloads again", |output| {
		output.matches("The world restarted").count() > restarts
			&& output.rsplit("The world restarted").next().is_some_and(|after| reloads(after) > 0)
	});
}

/// Launch a separate world with hot reload and source watching.
fn watched_world(name: &str) -> (World, PathBuf) {
	let dir = copy_of_the_world(name);
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	(world, dir)
}

/// Removing only a built-in override must still produce a reload.
fn built_in_taken_away() {
	let (mut world, dir) = watched_world("built_in_taken_away");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/obj/layered\n\tlayer = 7\n\n/proc/icon_built()\n"
	);
	assert_eq!(world.eval_global("(new /obj/layered).layer"), "7");
	// Keep source lines unchanged so the override is the only difference.
	edit(&mut world, "main.dm", "\tlayer = 7\n", "\t// layer = 7\n");
	assert_eq!(world.eval_global("(new /obj/layered).layer"), "3");
	ok("built-in-only override removal");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Edits the client has no part in: an agent's, another editor's, a checkout
fn behind_the_clients_back() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	let at = world.marks.clone();
	assert_eq!(world.eval_global("TWICE(21)"), "42");

	// Three lines more above the breakpoint, which the client still has on
	// the old line
	world.break_on(&["leaf_2"]);
	let before = at.line("leaf_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n"
	);
	assert_eq!(moved(&mut world), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	assert_eq!(world.eval("TWICE(y)"), "8");
	ok("source macros in frame and global evaluation");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("breakpoint remapping after external edits");

	// A line more inside the proc, which is compiled again
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\ty += 0\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("breakpoint remapping within a proc");

	// Its own line is written anew, as two: it's on the first of them
	edit(
		&mut world,
		"stepping.dm",
		"\treturn y * 2 // @leaf_2\n",
		"\tvar/doubled = y * 2 // @leaf_doubled\n\treturn doubled // @leaf_2\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_doubled"), true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_doubled");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_doubled");
	world.resume_through("middle");
	ok("breakpoint on replacement text");

	// Its line is taken away: the next line left is the proc's last
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/doubled = y * 2 // @leaf_doubled\n\treturn doubled // @leaf_2\n",
		"\treturn y * 2 // @leaf_2\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.break_on(&[]);

	// VS Code moves its breakpoints with the text while it's edited, and
	// once it's saved sends them again with `sourceModified`: lines of the
	// file as saved. Here they come before the world has the change, and
	// one of them was taken away meanwhile.
	let stepping = at.file("stepping.dm");
	world.break_on(&["leaf_1", "middle_2"]);
	let middle_2 = at.line("middle_2");
	world.drop_events("breakpoint");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let before = reloads(&world.output);
	at.edit("stepping.dm", "/proc/middle(a)\n", "/proc/middle(a)\n\tvar/first = 1\n\tfirst++\n");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_2 + 2 }] })
	);
	assert_eq!(reloads(&world.output), before, "the reload came before the client's breakpoints");
	// The world's code is still the old one, where that line is another
	// statement: a client told where that is would take it for its own
	assert_eq!(
		(
			&set["breakpoints"][0]["verified"],
			&set["breakpoints"][0]["line"],
			&set["breakpoints"][0]["message"]
		),
		(&json!(false), &json!(middle_2 + 2), &json!("pending hot reload")),
		"{}",
		set
	);
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (middle_2 + 2, true));
	assert_eq!(at.line("middle_2"), middle_2 + 2);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("defer sourceModified breakpoints until reload");

	// The same with the world having the change first: the launcher has
	// moved them already. The client hasn't said it's saved yet.
	world.break_on(&["middle_3"]);
	let middle_3 = at.line("middle_3");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	edit(&mut world, "stepping.dm", "\tfirst++\n", "\tfirst++\n\tfirst++\n");
	assert_eq!(moved(&mut world), (middle_3 + 1, true));
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_3 + 1 }] })
	);
	assert_eq!(
		(set["breakpoints"][0]["verified"].clone(), set["breakpoints"][0]["line"].clone()),
		(json!(true), json!(middle_3 + 1)),
		"{}",
		set
	);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert!(!world.has_event("breakpoint"), "the breakpoints were set again on being saved");
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("sourceModified after reload");

	// Lines added above, not saved, and then a breakpoint added below them:
	// its line is the editor's, which the file doesn't have yet
	world.break_on(&[]);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_2") + 3;
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], false, "{}", set);
	assert_eq!(set["breakpoints"][0]["message"], "pending save and hot reload");
	ok("unsaved breakpoint waits for save");

	// Another file is reloaded meanwhile
	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/unrelated = 1\n");
	world.run_through("middle");
	assert!(
		!world.has_event("breakpoint"),
		"a reload of another file set the unsaved file's breakpoints"
	);
	ok("unrelated reload preserves pending breakpoint");

	// Saved: the client says so before the world has the change
	let before = reloads(&world.output);
	at.edit(
		"stepping.dm",
		"\tvar/r = leaf(a) // @middle_1\n",
		"\tvar/r = leaf(a) // @middle_1\n\tr += 0\n\tr += 0\n\tr += 0\n"
	);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(reloads(&world.output), before, "reload occurred before the save notification");
	// It still waits, for the world now, on the line it was given
	let waiting = world.event("breakpoint");
	assert_eq!(
		(&waiting["body"]["breakpoint"]["verified"], &waiting["body"]["breakpoint"]["line"]),
		(&json!(false), &json!(buffer_line)),
		"{}",
		waiting
	);
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_2"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("save and reload do not remap twice");

	// Again with the world having the change before it's said to be saved
	world.set_breakpoints(&stepping, json!([]));
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_3") + 1;
	world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] })
	);
	edit(
		&mut world,
		"stepping.dm",
		"\tr += 0\n\tr += 0\n\tr += 0\n",
		"\tr += 0\n\tr += 0\n\tr += 0\n\tr += 0\n"
	);
	world.run_through("middle");
	assert!(!world.has_event("breakpoint"), "a reload set the unsaved file's breakpoints");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_3"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	world.set_breakpoints(&stepping, json!([]));
	ok("reload before save notification");

	// Another launcher, attached, with a breakpoint of its own: it doesn't
	// reload anything, and is told the code changed
	let pid = crate::client::listed_world("dap_test_host.dmb");
	let mut other = World::attach_in(dir.clone(), pid);
	other.catch_runtimes(false);
	other.break_on(&["sc_middle_2"]);
	let before = at.line("sc_middle_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/before_it()\n\treturn 1\n\n/proc/sc_middle()\n"
	);
	assert_eq!(moved(&mut other), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	other.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	assert_eq!(other.local("got"), "26");
	other.resume();
	world.finish("middle");
	other.end();
	ok("attached-session breakpoint remapping");

	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Without hot reloading the world's code stays as it was built
fn saved_without_reloading() {
	let mut world = World::launch(json!({ "hotReload": false }));
	world.catch_runtimes(false);
	world.configure();
	let (file, line) = world.marks.place("leaf_1");
	for _ in 0..2 {
		let set = world.request(
			"setBreakpoints",
			json!({ "source": { "path": file }, "sourceModified": true, "breakpoints": [{ "line": line }] })
		);
		assert_eq!(set["breakpoints"][0]["verified"], true, "{}", set);
	}
	assert_eq!(
		world.count("has changed since the world was built"),
		1,
		"expected exactly one changed-source warning"
	);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.set_breakpoints(&file, json!([]));
	world.resume_through("middle");
	ok("changed-source warning without hot reload");
	// Its code isn't read, so it hasn't its macros
	let said = world.refused("evaluate", json!({ "expression": "TWICE(21)", "context": "repl" }));
	assert!(said.contains("source macros unavailable"), "{}", said);
	ok("missing-macro diagnostic without hot reload");
	world.end();
}

/// With no global init proc or map, startup still precedes world/New
/// at an internal call. Reloads introducing resources must apply at once
/// after world/New has loaded the world data.
fn starting_at_world_new() {
	let dir = std::env::temp_dir().join(format!("dap_test_world_new-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("dap_test_host.dme"), "#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/icon_new()\n\treturn null\n"
	)
	.unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("reload_new.dmi")).unwrap();
	let built = Command::new(crate::paths::find_dm())
		.with_byond_paths()
		.arg(dir.join("dap_test_host.dme"))
		.status()
		.unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload: watching");
	edit(&mut world, "bare.dm", "\treturn null\n", "\treturn 'reload_new.dmi'\n");
	assert_eq!(world.eval_global("jointext(icon_states(icon_new()), \",\")"), "\"new1\"");
	assert!(
		!world.output.contains("applies as world/New starts"),
		"the reload waited for world/New:\n{}",
		world.output
	);
	ok("resource reload after world/New startup");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// A parent-only change must still produce a reload.
fn parent_changed_alone() {
	let (mut world, dir) = watched_world("parent_changed_alone");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/datum/pa\n\tvar/x = 1\n\n/datum/pb\n\tvar/x = 2\n\n/datum/pt\n\tparent_type = /datum/pa\n\n/proc/icon_built()\n"
	);
	assert_eq!(world.eval_global("(new /datum/pt).x"), "1");
	edit(&mut world, "main.dm", "\tparent_type = /datum/pa\n", "\tparent_type = /datum/pb\n");
	assert_eq!(world.eval_global("(new /datum/pt).x"), "2");
	assert_eq!(world.eval_global("istype(new /datum/pt, /datum/pb)"), "1");
	ok("parent-only reload");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Restore added types and a parent change after reboot. Wait until
/// InitVarRecorder calls release their type entries, then restore before
/// global initialization and map creation. Restoring too early crashed
/// or made a new ancestor's New() recurse through ..().
fn types_changed_before_reboot() {
	let (mut world, dir) = watched_world("types_changed_before_reboot");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/datum/rb_base\n\tvar/list/held = list(3)\n\tvar/x = 5\n\n/datum/rb_base/kid\n\n/datum/rb_base/New()\n\tx += \
		 1\n\t..()\n\n/proc/icon_built()\n"
	);
	edit(
		&mut world,
		"stepping.dm",
		"/datum/made\n",
		"/datum/made\n\tparent_type = /datum/rb_base\n"
	);
	// New() calls the override added on its ancestor.
	edit(
		&mut world,
		"stepping.dm",
		"\tsize = n // @made_new\n",
		"\tsize = n // @made_new\n\t..()\n"
	);
	let check = |world: &mut World| {
		assert_eq!(world.eval_global("var/datum/rb_base/kid/k = new\n\"[k.x] [length(k.held)]\""), "\"6 1\"");
		assert_eq!(
			world.eval_global("var/datum/made/m = new(2)\n\"[istype(m, /datum/rb_base)] [m:x] [m.size]\""),
			"\"1 6 2\""
		);
	};
	check(&mut world);
	for count in 2..=4 {
		reboot(&mut world, count);
		check(&mut world);
	}
	ok("type and parent restoration after reboot");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Restore an override before map creation. BYOND has not built proc-name
/// maps yet; inserting it there as well caused duplicate entries and
/// recursive ..() calls when BYOND later built the map.
fn override_restored_before_map() {
	let dir = copy_of_the_world("override_restored_before_map");
	std::fs::write(
		dir.join("placed.dm"),
		"var/placed_news = 0\n\n/obj/placed\n\n/obj/placed/New()\n\tplaced_news++\n\t..()\n\n/datum/rb_over\n"
	)
	.unwrap();
	std::fs::write(
		dir.join("placed.dmm"),
		"\"a\" = (/turf,/area)\n\"b\" = (/obj/placed,/turf,/area)\n\n(1,1,1) = {\"\naab\n\"}\n"
	)
	.unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"placed.dm\"\n#include \"placed.dmm\"\n";
	std::fs::write(&dme, text).unwrap();
	let built = Command::new(crate::paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	edit(
		&mut world,
		"placed.dm",
		"/datum/rb_over\n",
		"/datum/rb_over\n\n/datum/rb_over/New()\n\tplaced_news += 10\n\t..()\n"
	);
	let made = |world: &mut World| world.eval_global("var/before = placed_news\nnew /datum/rb_over\nplaced_news - before");
	assert_eq!(made(&mut world), "10");
	for count in 2..=3 {
		reboot(&mut world, count);
		assert_eq!(world.eval_global("placed_news"), "1", "map atom was not constructed exactly once");
		assert_eq!(made(&mut world), "10");
	}
	ok("override restoration before map creation");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// New FILE_DIR references use paths relative to the world directory,
/// unlike built resource names. Watch and reload later saves too.
fn file_named_by_a_reload() {
	let dir = copy_of_the_world("file_named_by_a_reload");
	std::fs::create_dir_all(dir.join("fzdir")).unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("fzdir/named.dmi")).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = "#define FILE_DIR fzdir\n".to_owned() + &std::fs::read_to_string(&dme).unwrap();
	std::fs::write(&dme, text).unwrap();
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/proc/icon_named()\n\treturn 'named.dmi'\n\n/proc/icon_built()\n"
	);
	let shown = |world: &mut World| world.eval_global("\"[icon_named()] [jointext(icon_states(icon_named()), \",\")]\"");
	assert_eq!(shown(&mut world), "\"fzdir/named.dmi new1\"");
	ok("new resource lookup through FILE_DIR");

	let before = world.count("Hot reload: reloaded fzdir/named.dmi");
	// (bytes the world has no resource for)
	std::fs::copy(host_file("lamp_edited.dmi"), dir.join("fzdir/named.dmi")).unwrap();
	let saved = std::fs::File::options().write(true).open(dir.join("fzdir/named.dmi")).unwrap();
	saved.set_modified(std::time::SystemTime::now()).unwrap();
	world.wait_until("the resource reload", |output| {
		output.matches("Hot reload: reloaded fzdir/named.dmi").count() > before
	});
	assert_eq!(shown(&mut world), "\"fzdir/named.dmi on,off,new\"");
	ok("watching newly referenced resources");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Wait for a resource reload report.
fn resource_saved(world: &mut World, dir: &std::path::Path, version: &str) {
	let before = world.count("Hot reload: reloaded lamp.dmi");
	std::fs::copy(dir.join(version), dir.join("lamp.dmi")).unwrap();
	// As an editor saves it: Windows keeps the copied file's time
	let saved = std::fs::File::options().write(true).open(dir.join("lamp.dmi")).unwrap();
	saved.set_modified(std::time::SystemTime::now()).unwrap();
	world.wait_until("the resource reload", |output| {
		output.matches("Hot reload: reloaded lamp.dmi").count() > before
	});
}

/// An icon saved while the world runs, then after a reboot
fn resource_files() {
	let dir = copy_of_the_world("resources");
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload: watching");
	assert_eq!(world.eval_global("make_lamps()"), "\"made\"");
	assert_eq!(world.eval_global("lamp_states()"), "\"on,off\"");

	// Existing appearances, resource literals and newly created instances.
	resource_saved(&mut world, &dir, "lamp_edited.dmi");
	let new = "\"on,off,new\"";
	assert_eq!(world.eval_global("lamp_states()"), new);
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), new);
	assert_eq!(world.eval_global("states_of(placed_lamp.icon)"), new);
	assert_eq!(world.eval_global("overlay_states()"), new);
	assert_eq!(world.eval_global("var/obj/lamp/L = new\nstates_of(L.icon)"), new);
	ok("icon reload on existing and new instances");

	// Restore the icon after reboot, including appearances made while loading.
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.wait_output("Hot reload: restoring resource files changed since the build");
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), new);
	assert_eq!(world.eval_global("lamp_states()"), new);
	ok("saved icon restored after reboot");

	// A later save must replace the restored resource ID too.
	resource_saved(&mut world, &dir, "lamp_third.dmi");
	let third = "\"on,off,third\"";
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), third);
	assert_eq!(world.eval_global("lamp_states()"), third);
	ok("icon saved again after reboot");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Breakpoint after `var/text = "[n]"`: DreamMaker records that line before
/// assigning the var, so `text` is still unset there.
fn read_ahead_stop(world: &mut World, file: &std::path::Path, line: u64) -> (u64, String) {
	world.set_breakpoints(file, json!([line]));
	world.run("nothing_to_run");
	let stop = world.stop();
	let text = world.local("text");
	world.resume_through("nothing_to_run");
	world.set_breakpoints(file, json!([]));
	(stop.line, text)
}

/// Breakpoints must retain their positions after reload, including empty
/// macro expansions, uninitialized vars, set statements and lookahead lines.
fn nothing_to_run() {
	let (mut world, dir) = watched_world("nothing_to_run");
	let at = world.marks.clone();
	let file = at.file("stepping.dm");
	let names = ["nothing_macro", "nothing_var", "nothing_set", "nothing_code"];
	let lines: Vec<u64> = names.iter().map(|name| at.line(name)).collect();
	let placed = |world: &mut World| -> Vec<Value> {
		let set = world.set_breakpoints(&file, json!(lines));
		set.iter().map(|b| b["line"].clone()).collect()
	};
	let expected: Vec<Value> = lines.iter().map(|l| json!(l)).collect();
	assert_eq!(placed(&mut world), expected, "breakpoints moved off lines DreamMaker's code has");
	world.set_breakpoints(&file, json!([lines[0]]));
	world.run("nothing_to_run");
	world.stop_on("breakpoint", "/nothing_to_run", "nothing_macro");
	world.resume_through("nothing_to_run");
	let read_ahead = at.line("read_ahead_next");
	let before = read_ahead_stop(&mut world, &file, read_ahead);

	// Change the proc so its bytecode is recompiled.
	edit(&mut world, "stepping.dm", "\tn = 1 // @nothing_code\n", "\tn = 2 // @nothing_code\n");
	assert_eq!(world.eval_global("nothing_to_run()"), "2");
	assert_eq!(placed(&mut world), expected, "breakpoints moved off lines after the reload");
	world.set_breakpoints(&file, json!([lines[0]]));
	world.run("nothing_to_run");
	world.stop_on("breakpoint", "/nothing_to_run", "nothing_macro");
	world.resume_through("nothing_to_run");
	ok("breakpoints on nonexecutable lines after reload");
	let after = read_ahead_stop(&mut world, &file, read_ahead);
	assert_eq!(after, before, "a breakpoint DreamMaker records ahead stopped elsewhere after the reload");
	ok("breakpoint on a lookahead line after reload");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

pub fn run() {
	saved_without_reloading();
	built_in_taken_away();
	behind_the_clients_back();
	starting_at_world_new();
	parent_changed_alone();
	types_changed_before_reboot();
	override_restored_before_map();
	file_named_by_a_reload();
	resource_files();
	nothing_to_run();

	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_as(json!({ "supportsInvalidatedEvent": true }), dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");

	// Three lines more above the breakpoint: the client moves its mark with
	// the text, and says where its breakpoints are now
	world.break_on(&["leaf_2"]);
	let before = world.marks.line("leaf_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n"
	);
	assert_eq!(world.marks.line("leaf_2"), before + 3);
	world.break_on(&["leaf_2"]);
	assert!(
		!world.has_event("invalidated"),
		"a reload of a running world left something to ask for again"
	);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	ok("client-remapped breakpoint stays on its statement");

	// What the debug server has to say comes once, as the launcher's words
	assert_eq!(
		world.count("[auxtools] Hot reload: new code for"),
		1,
		"expected exactly one reload notification"
	);
	assert!(
		!world.output.contains("Debug Server:"),
		"the debug server's own output was passed on as well"
	);
	assert!(!world.output.contains("Pausing execution"), "a stop was announced in the console");
	ok("single delivery of server notifications");

	// Paused in leaf(), whose code changes under it
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n"
	);
	// The client is shown frames it asked for before the reload
	assert_eq!(world.event("invalidated")["body"]["areas"], json!(["stacks"]));
	let frames = world.frames(0);
	assert_eq!(frames[0]["name"], "/leaf (old code)");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [".", "y"]);
	ok("paused reload preserves old code and locals");

	// Its caller's code is as it was, two lines further down
	assert_eq!(
		(frames[1]["name"].as_str().unwrap(), frames[1]["line"].as_u64().unwrap()),
		("/middle", world.marks.line("middle_1"))
	);
	ok("line-only reload updates frame locations");

	// middle()'s r is what the old leaf() gives for 3
	world.break_on(&[]);
	world.step_on("stepOut", "/middle", "middle_1");
	world.step_on("next", "/middle", "middle_2");
	assert_eq!(world.local("r"), "8");
	ok("active frame finishes old code");

	world.step_on("stepIn", "/leaf", "leaf_1");
	world.step_on("next", "/leaf", "leaf_extra");
	world.step("next");
	world.step_on("next", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!((value_of(&locals, "y"), value_of(&locals, "extra")), ("14", "5"));
	world.resume_through("middle");
	ok("new calls use reloaded code and locals");

	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/sc_fresh()\n\tvar/a = 1 // @fresh_1\n\tvar/b = a + 1 // @fresh_2\n\treturn b // @fresh_3\n\n/proc/sc_middle()\n"
	);
	let stepping = world.marks.file("stepping.dm");
	let (fresh_1, fresh_3) = (world.marks.line("fresh_1"), world.marks.line("fresh_3"));
	let places = world.request(
		"breakpointLocations",
		json!({ "source": { "path": stepping }, "line": fresh_1 - 1, "endLine": fresh_3 + 3 })
	);
	let lines: Vec<u64> = places["breakpoints"]
		.as_array()
		.unwrap()
		.iter()
		.map(|place| place["line"].as_u64().unwrap())
		.collect();
	assert_eq!(lines, [fresh_1, world.marks.line("fresh_2"), fresh_3, world.marks.line("sc_middle_1")]);
	ok("breakpoint locations after proc additions");
	world.run_to("fresh", "/sc_fresh", "fresh_2");
	assert_eq!(world.local("a"), "1");
	world.step_on("next", "/sc_fresh", "fresh_3");
	assert_eq!(world.local("b"), "2");
	world.resume_through("fresh");
	ok("stepping into an added proc");

	// A reload must resolve a breakpoint in a previously unused, unchanged
	// file.
	let added = dir.join("added.dm");
	std::fs::write(&added, "/proc/sc_added()\n\tvar/a = 1\n\treturn a + 1\n").unwrap();
	std::fs::File::options()
		.write(true)
		.open(&added)
		.unwrap()
		.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
		.unwrap();
	let set = world.set_breakpoints(&added, json!([{ "line": 3 }]));
	assert_eq!(set[0]["verified"], false, "{}", set[0]);
	world.drop_events("breakpoint");
	edit(
		&mut world,
		"dap_test_host.dme",
		"#include \"main.dm\"\n",
		"#include \"main.dm\"\n#include \"added.dm\"\n"
	);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!(
		(&told["id"], &told["verified"], &told["line"]),
		(&set[0]["id"], &json!(true), &json!(3)),
		"{}",
		told
	);
	world.run("added");
	world.launcher.stop_at("breakpoint", "/sc_added", 3);
	world.set_breakpoints(&added, json!([]));
	world.resume_through("added");
	ok("breakpoint in a file a reload adds");

	// The world counts a breakpoint's hits from when it's set: a reload of
	// another file leaves it be
	let (file, line) = world.marks.place("recurse_leaf");
	world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": "% 2" }]));
	world.run_through("recurse");
	world.drop_events("breakpoint");
	edit(&mut world, "errors.dm", "\tvar/zero = 0\n", "\tvar/zero = 0 * 2\n");
	assert!(!world.has_event("breakpoint"), "a reload of another file set the breakpoint again");
	world.run("recurse");
	world.stop_on("breakpoint", "/recurse", "recurse_leaf");
	world.set_breakpoints(&file, json!([]));
	world.resume_through("recurse");
	ok("unrelated reload preserves hit counts");

	// BYOND's own list of the world's globals doesn't get one a reload adds
	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/added_later = 12\n");
	assert_eq!(world.eval_global("added_later"), "12");
	assert_eq!(world.eval_global("added_later = ticks > 0 ? 13 : 0"), "13");
	assert_eq!(world.eval_global("global.added_later + 1"), "14");
	ok("new global evaluation and assignment");

	// The proc's code changes, and then its first line too
	let set = world.function_breakpoints(&["/proc/leaf"]);
	assert_eq!(set[0]["verified"], true);
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/extra = 5 // @leaf_extra\n",
		"\tvar/extra = 6 // @leaf_extra\n"
	);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_1");
		world.resume();
	}
	world.finish("middle");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/leaf(x)\n\tvar/first = 1 // @leaf_first\n\tfirst++\n"
	);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_first");
		world.resume();
	}
	world.finish("middle");
	world.function_breakpoints(&[]);
	world.run_through("middle");
	ok("function breakpoint follows reloaded entry");

	// Resolve a pending function breakpoint when reload adds the proc.
	let set = world.function_breakpoints(&["/proc/later"]);
	assert_eq!(set[0]["message"], "there's no /proc/later", "{}", set[0]);
	world.drop_events("breakpoint");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/sc_later()\n\treturn later()\n\n/proc/later()\n\treturn 1 // @later_1\n\n/proc/sc_middle()\n"
	);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!(
		(&told["id"], &told["verified"], &told["line"]),
		(&set[0]["id"], &json!(true), &json!(world.marks.line("later_1"))),
		"{}",
		told
	);
	world.run("later");
	world.stop_on("breakpoint", "/later", "later_1");
	world.function_breakpoints(&[]);
	world.resume_through("later");
	ok("function breakpoint on a proc a reload adds");
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\ty += extra\n\treturn y * 2 // @leaf_2\n",
		"\tvar/renamed = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\trenamed += extra\n\treturn renamed * 2 // @leaf_2\n"
	);
	world.run_to("middle", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [
		".", "first", "renamed", "extra"
	]);
	assert_eq!(world.eval("renamed + extra"), "16");
	world.resume_through("middle");
	ok("renamed local inspection and evaluation");

	// The runtime error is a line further down now
	world.catch_runtimes(true);
	edit(
		&mut world,
		"errors.dm",
		"\tvar/list/L = null\n\treturn L[1] // @fail_null\n",
		"\tvar/list/L = null\n\tvar/unused = 1\n\treturn L[1] // @fail_null\n"
	);
	world.run("fail_null");
	let stop = world.stop();
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("exception", "/fail_null", world.marks.line("fail_null"))
	);
	world.resume_through("fail_null");
	world.catch_runtimes(false);
	ok("runtime error location after reload");

	// An edit the world can't take: nothing changes, breakpoints included
	world.break_on(&["leaf_2"]);
	world.marks.edit("stepping.dm", "\tvar/n = 1\n", "\tvar/n = 1\n\tvar/list/broken = (\n");
	world.wait_output("Hot reload: rejected; rebuild required");
	world.drop_events("breakpoint");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.break_on(&[]);
	world.resume_through("middle");
	world.marks.edit("stepping.dm", "\tvar/list/broken = (\n", "");
	world.wait_output("Hot reload: no runtime changes");
	world.drop_events("breakpoint");
	ok("rejected reload preserves code and breakpoints");

	// The next world is loaded from the .dmb, and gets the reloads again
	world.break_on(&["leaf_2"]);
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.wait_until("the reloads again", |output| {
		output.rsplit("The world restarted").next().is_some_and(|after| reloads(after) > 0) && output.contains("The world restarted")
	});
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("extra"), "6");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("restore reloads and breakpoints after reboot");

	// Before resources load, restored code must use the build's resource ID
	// (the first cache ID belongs to reload_built.dmi).
	let states = |world: &mut World, proc: &str| world.eval_global(&format!("jointext(icon_states({}()), \",\")", proc));
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_reloaded()\n\treturn null\n",
		"/proc/icon_reloaded()\n\treturn 'reload_built_2.dmi'\n"
	);
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	reboot(&mut world, 3);
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	let after = world.output.rsplit("The world restarted").next().unwrap_or_default().to_owned();
	assert!(
		!after.contains("applies as world/New starts"),
		"the reload waited for world/New:\n{}",
		after
	);
	ok("restored reload names a built resource");

	// A resource absent from the build must wait for world/New to get an ID.
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/proc/icon_new()\n\treturn 'reload_new.dmi'\n\n/proc/icon_built()\n"
	);
	assert_eq!(states(&mut world, "icon_new"), "\"new1\"");
	reboot(&mut world, 4);
	let after = world.output.rsplit("The world restarted").next().unwrap_or_default().to_owned();
	assert!(
		after.contains("resource files need the loaded cache (reload_new.dmi); reload applies as world/New starts"),
		"the launcher said:\n{}",
		after
	);
	assert_eq!(states(&mut world, "icon_new"), "\"new1\"");
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	assert_eq!(states(&mut world, "icon_built"), "\"built1,built2\"");
	ok("restored reload naming a new resource waits for world/New");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}
