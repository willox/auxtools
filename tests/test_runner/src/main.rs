mod paths;

use std::process::Command;

trait ByondCommand {
	fn with_byond_paths(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl ByondCommand for Command {
	// TODO: This doesn't read very nice
	fn with_byond_paths(&mut self) -> &mut Command {
		let byond_system = paths::find_byond();
		let byond_bin = paths::find_byond_bin();

		let path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("PATH").unwrap().to_str().unwrap()
		);

		let ld_library_path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("LD_LIBRARY_PATH")
				.unwrap()
				.to_str()
				.unwrap()
		);

		self.env("BYOND_SYSTEM", byond_system)
			.env("PATH", path)
			.env("LD_LIBRARY_PATH", ld_library_path)
	}
}

#[cfg(windows)]
impl ByondCommand for Command {
	fn with_byond_paths(&mut self) -> &mut Command {
		self
	}
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(paths::find_dme())
		.status()
		.unwrap();
	assert!(res.success(), "dreamdaemon build failed");

	// Here we depend on BYOND not fucking with stderr too much so we can hijack it for our own communication
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
	let errors = res
		.lines()
		.filter(|x| x.starts_with("FAILED: "))
		.collect::<Vec<&str>>();

	if !errors.is_empty() {
		panic!("TESTS FAILED\n{}", errors.join("\n"));
	}

	// Now make sure we have only one message matching "SUCCESS: <msg>"
	let successes = res
		.lines()
		.filter(|x| x.starts_with("SUCCESS: "))
		.collect::<Vec<&str>>();
	assert_eq!(successes.len(), 1, "Tests didn't output success message");

	println!("Tests Succeeded");
}
