mod paths;

use std::{fs, process::Command};

trait ByondCommand {
	fn with_byond_paths(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl ByondCommand for Command {
	// TODO: This doesn't read very nice
	fn with_byond_paths(&mut self) -> &mut Command {
		let byond_system = paths::find_byond();
		let byond_bin = paths::find_byond_bin();

		let old_path = std::env::var_os("PATH").unwrap_or_default();
		let path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			old_path.to_str().unwrap()
		);

		let ld_library_path = match std::env::var_os("LD_LIBRARY_PATH") {
			Some(old) if !old.is_empty() => format!("{}:{}", byond_bin.as_os_str().to_str().unwrap(), old.to_str().unwrap()),
			_ => byond_bin.as_os_str().to_str().unwrap().to_owned()
		};

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
	let dme = paths::find_dme();
	let dmb = paths::dmb_path();
	let dreamdaemon = paths::find_dreamdaemon();
	let auxtest = paths::find_dll();
	let auxtest_out = dmb.with_file_name("auxtest_output.log");
	let world_log = dmb.with_file_name("auxtest_host.txt");
	let _ = fs::remove_file(&dmb);
	let _ = fs::remove_file(&auxtest_out);
	let _ = fs::remove_file(&world_log);

	let dreammaker_output = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(&dme)
		.output()
		.unwrap();
	assert!(
		dreammaker_output.status.success(),
		"dreammaker build failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&dreammaker_output.stdout),
		String::from_utf8_lossy(&dreammaker_output.stderr)
	);
	assert!(dmb.is_file(), "dreammaker did not produce {}", dmb.display());
	let dmb_metadata = fs::metadata(&dmb).unwrap();

	// Here we depend on BYOND not fucking with stderr too much so we can hijack it
	// for our own communication
	let mut dreamdaemon_command = Command::new(&dreamdaemon);
	dreamdaemon_command
		.with_byond_paths()
		.current_dir(dmb.parent().unwrap())
		.env("AUXTEST_DLL", &auxtest)
		.env("AUXTEST_OUT", &auxtest_out)
		.arg(&dmb)
		.arg("0")
		.arg("-close")
		.arg("-trusted")
		.arg("-log")
		.arg("auxtest_host.txt");

	#[cfg(windows)]
	let (status, output) = {
		let status = dreamdaemon_command.status().unwrap();
		(status, Vec::new())
	};

	#[cfg(unix)]
	let (status, output) = {
		let output = dreamdaemon_command.output().unwrap();
		(output.status, [output.stdout, output.stderr].concat())
	};

	let world_log = fs::read_to_string(&world_log).unwrap_or_default();
	let marker_log = fs::read(&auxtest_out).unwrap_or_default();
	let output = [output, marker_log].concat();
	let output_with_world_log = [output.clone(), world_log.as_bytes().to_vec()].concat();

	let res = std::str::from_utf8(&output).unwrap();
	let res_with_world_log = std::str::from_utf8(&output_with_world_log).unwrap();

	// Check for any messages matching "FAILED: <msg>"
	let errors = res_with_world_log
		.lines()
		.filter(|x| x.starts_with("FAILED: "))
		.collect::<Vec<&str>>();

	if !errors.is_empty() {
		panic!("TESTS FAILED\n{}", errors.join("\n"));
	}

	// Now make sure we have only one message matching "SUCCESS: <msg>"
	let successes = res.lines().filter(|x| x.starts_with("SUCCESS: ")).collect::<Vec<&str>>();
	assert_eq!(
		successes.len(),
		1,
		"Tests didn't output success message\ndreamdaemon status: {}\ndreamdaemon: {}\ndmb: {}\nauxtest: {}\noutput:\n{}\nworld log:\n{}",
		status,
		dreamdaemon.display(),
		format_args!("{} ({} bytes)", dmb.display(), dmb_metadata.len()),
		auxtest.display(),
		res,
		world_log
	);
	assert!(
		status.success() || successes.len() == 1,
		"dreamdaemon failed with status {}\ndreamdaemon: {}\ndmb: {}\nauxtest: {}\noutput:\n{}\nworld log:\n{}",
		status,
		dreamdaemon.display(),
		dmb.display(),
		auxtest.display(),
		res,
		world_log
	);

	println!("Tests Succeeded");
}
