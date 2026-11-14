//! Skin edits, reboot restoration and changed .dme includes, inspected with
//! `#skin`.

use std::{path::Path, process::Command};

use serde_json::json;

use crate::{ok, paths, paths::ByondCommand, world::World};

const SKIN: &str = "macro \"default\"\n\telem \"k_f\"\n\t\tname = \"F\"\n\t\tcommand = \"say \\\"f\\\"\"\n\nwindow \"mainwindow\"\n\telem \
                    \"mainwindow\"\n\t\ttype = MAIN\n\t\tsize = 640x480\n\t\tis-default = true\n\t\ttitle = \"first\"\n\telem \"label\"\n\t\ttype = \
                    LABEL\n\t\tpos = 0,0\n\t\tsize = 100x20\n\t\timage = 'built.png'\n";

/// The skin's resource and text, as `#skin` gives them
fn skin(world: &mut World) -> (String, String) {
	let shown = world.eval_global("#skin");
	let (resource, text) = shown.split_once('\n').unwrap_or((&shown, ""));
	assert!(resource.starts_with("resource "), "{}", shown);
	(resource.to_owned(), text.to_owned())
}

fn text_of(dir: &Path, file: &str) -> String {
	std::fs::read_to_string(dir.join(file)).unwrap()
}

pub fn run() {
	let dir = super::reloading::long_name(super::reloading::copy_of_the_world("skins"));
	std::fs::write(dir.join("skin.dmf"), SKIN).unwrap();
	std::fs::write(dir.join("built.png"), b"\x89PNG\r\n\x1a\n").unwrap();
	let dme = dir.join("dap_test_host.dme");
	let source = std::fs::read_to_string(&dme).unwrap();
	std::fs::write(&dme, source + "#include \"skin.dmf\"\n").unwrap();
	let built = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build failed");

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let (first, text) = skin(&mut world);
	assert_eq!(text, SKIN);
	ok("built skin");

	// The idle-world waker connects without logging in. Its client has no
	// key and must not receive winset() calls.
	let port: u16 = world.eval_global("world.port").parse().unwrap();
	let _waker = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
	while world.eval_global("var/n = 0\nfor(var/client/C)\n\tn++\nn") != "1" {
		assert!(std::time::Instant::now() < deadline, "the world made no client for a connection");
		std::thread::sleep(std::time::Duration::from_millis(200));
	}

	world
		.marks
		.edit("skin.dmf", "title = \"first\"", "title = \"second\"\n\t\timage = 'new.png'");
	world.wait_output("Hot reload: skin skin.dmf updated.");
	let told = world.event("auxtoolsSkin");
	assert_eq!(told["body"], json!({ "file": "skin.dmf", "clients": 0, "hosted": false }), "{}", told);
	let (second, text) = skin(&mut world);
	assert_ne!(second, first);
	assert_eq!(text, text_of(&dir, "skin.dmf"));
	ok("skin reload");
	world.wait_output("'new.png' is new to the skin and needs a rebuild.");
	ok("new skin image warnings");

	let reconnected = world.request("auxtoolsReconnect", json!({}));
	assert_eq!(reconnected, json!({ "reconnected": 0 }));
	ok("reconnect skips unlogged connections");

	world.drop_events("auxtoolsReload");
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.reloaded("skin skin.dmf");
	world.wait_output("The world restarted");
	let (_, text) = skin(&mut world);
	assert_eq!(text, text_of(&dir, "skin.dmf"));
	// Skin changes must not enter resource reloads and defer code until
	// world/New.
	assert!(
		!world.output.contains("reloaded skin.dmf") && !world.output.contains("doesn't have (skin.dmf)"),
		"the skin was reloaded as a resource file:\n{}",
		world.output
	);
	ok("skin reload after reboot");

	std::fs::write(dir.join("other.dmf"), SKIN.replace("first", "other")).unwrap();
	world.marks.edit(
		"dap_test_host.dme",
		"#include \"skin.dmf\"\n",
		"#include \"skin.dmf\"\n#include \"other.dmf\"\n"
	);
	world.wait_output("Hot reload: skin other.dmf updated.");
	let (_, text) = skin(&mut world);
	assert_eq!(text, text_of(&dir, "other.dmf"));
	ok("last included skin");

	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}
