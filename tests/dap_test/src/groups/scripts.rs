//! Client script reloads and reboot restoration, inspected with `#script`.

use std::process::Command;

use serde_json::json;

use crate::{ok, paths, paths::ByondCommand, world::World};

/// Script string ID (or "none"), file resource IDs and combined login text.
fn script(world: &mut World) -> (String, Vec<String>, String) {
	let shown = world.eval_global("#script");
	let mut lines = shown.splitn(3, '\n');
	let text = lines
		.next()
		.and_then(|line| line.strip_prefix("text "))
		.unwrap_or_else(|| panic!("{}", shown));
	let files = lines
		.next()
		.and_then(|line| line.strip_prefix("files"))
		.unwrap_or_else(|| panic!("{}", shown));
	(
		text.to_owned(),
		files.split_whitespace().map(str::to_owned).collect(),
		lines.next().unwrap_or_default().to_owned()
	)
}

/// Wait for a client script reload report.
fn told(world: &mut World) {
	world.wait_output("Hot reload: client script updated.");
	let event = world.event("auxtoolsScript");
	assert_eq!(event["body"], json!({ "clients": 0, "hosted": false }), "{}", event);
}

pub fn run() {
	let dir = super::reloading::long_name(super::reloading::copy_of_the_world("scripts"));
	std::fs::write(dir.join("x.dms"), "<style>x {}</style>\n").unwrap();
	std::fs::write(dir.join("a.css"), "<style>a {}</style>\n").unwrap();
	std::fs::write(dir.join("script.dm"), "/client/script = \"<style>first {}</style>\"\n").unwrap();
	let dme = dir.join("dap_test_host.dme");
	let source = std::fs::read_to_string(&dme).unwrap();
	std::fs::write(&dme, source + "#include \"x.dms\"\n#include \"script.dm\"\n").unwrap();
	let built = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build failed");

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let (text, files, full) = script(&mut world);
	assert_ne!(text, "none");
	assert_eq!(files.len(), 1, "{:?}", files);
	assert_eq!(full, "<style>x {}</style>\n\n<style>first {}</style>");
	ok("built client script");

	world.marks.edit("script.dm", "first", "second");
	told(&mut world);
	let (_, again, full) = script(&mut world);
	assert_eq!(again, files);
	assert_eq!(full, "<style>x {}</style>\n\n<style>second {}</style>");
	ok("client script text reload");

	world.marks.edit("script.dm", "\"<style>second {}</style>\"", "'a.css'");
	world.wait_output("client script: client/script references 'a.css'; resource-file scripts require a rebuild; the world keeps the script it has");
	assert_eq!(script(&mut world).2, full);
	ok("resource-file script requires rebuild");

	world.marks.edit("script.dm", "'a.css'", "null");
	told(&mut world);
	let (text, back, full) = script(&mut world);
	assert_eq!((text.as_str(), &back), ("none", &files));
	assert_eq!(full, "<style>x {}</style>\n\n");
	ok("client script removal");

	world.marks.edit("script.dm", "null", "\"<style>third {}</style>\"");
	told(&mut world);
	assert_eq!(script(&mut world).2, "<style>x {}</style>\n\n<style>third {}</style>");
	ok("client script restoration");

	world.drop_events("auxtoolsReload");
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.reloaded("client script");
	world.wait_output("The world restarted");
	assert_eq!(script(&mut world).2, "<style>x {}</style>\n\n<style>third {}</style>");
	ok("client script reload after reboot");

	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}
