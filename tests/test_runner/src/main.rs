mod paths;

use std::process::Command;

use paths::ByondCommand;

fn main() {
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(paths::find_dme()).status().unwrap();
	assert!(res.success(), "dreamdaemon build failed");

	// Here we depend on BYOND not fucking with stderr too much so we can hijack
	// it for our own communication
	let output = Command::new(paths::find_dreamdaemon())
		.with_byond_paths()
		.env("AUXTEST_DLL", paths::find_dll())
		.arg(paths::find_dmb())
		.arg("-trusted")
		.arg("-close")
		.output()
		.unwrap()
		.stderr;

	let res = std::str::from_utf8(&output).unwrap();

	// Check for any messages matching "FAILED: <msg>"
	let errors = res.lines().filter(|x| x.starts_with("FAILED: ")).collect::<Vec<&str>>();

	if !errors.is_empty() {
		panic!("TESTS FAILED\n{}", errors.join("\n"));
	}

	// Now make sure we have only one message matching "SUCCESS: <msg>"
	let successes = res.lines().filter(|x| x.starts_with("SUCCESS: ")).collect::<Vec<&str>>();
	assert_eq!(successes.len(), 1, "Tests didn't output success message");

	println!("Tests Succeeded");
}
