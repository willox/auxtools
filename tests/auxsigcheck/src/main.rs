mod args;
mod result;

use self::{
	args::Args,
	result::{extract_test_result, TestResult}
};
use byond_get::OsType;
use cfg_if::cfg_if;
use color_eyre::eyre::{Result, WrapErr};
use std::process::Command;
use tempdir::TempDir;

static TEST_DM: &[u8] = include_bytes!("dm/auxsigcheck.dm");
static TEST_DME: &[u8] = include_bytes!("dm/auxsigcheck.dme");

cfg_if! {
	if #[cfg(windows)] {
		const OS: OsType = OsType::Windows;
		const DREAMMAKER_EXE: &str = "dm.exe";
		const DREAMDAEMON_EXE: &str = "dd.exe";
	} else {
		const OS: OsType = OsType::Linux;
		const DREAMMAKER_EXE: &str = "DreamMaker";
		const DREAMDAEMON_EXE: &str = "DreamDaemon";
	}
}

fn main() -> Result<()> {
	color_eyre::install()?;
	let args = Args::new();
	let auxtools_path = std::path::absolute(args.auxtools_path).wrap_err("failed to get absolute auxtools path")?;
	let tmpdir = TempDir::new("auxsigcheck").wrap_err("failed to crate tempdir")?;
	let base_path = tmpdir.path();

	let byond_path = base_path.join("byond");
	byond_get::download_bin(args.version, args.build, OS, &byond_path).wrap_err("failed to download byond")?;

	std::fs::write(base_path.join("auxsigcheck.dm"), TEST_DM).wrap_err("failed to write auxsigcheck.dm")?;
	std::fs::write(base_path.join("auxsigcheck.dme"), TEST_DME).wrap_err("failed to write auxsigcheck.dme")?;

	let status = Command::new(byond_path.join(DREAMMAKER_EXE))
		.arg(base_path.join("auxsigcheck.dme"))
		.output()
		.wrap_err("failed to run DreamMaker")?
		.status;
	if !status.success() {
		panic!("Failed to compile auxsigcheck.dme, DreamMaker exited with code {status}")
	}

	std::env::set_var("LD_LIBRARY_PATH", byond_path.to_str().unwrap());
	std::env::set_var("AUXTOOLS_DLL", auxtools_path);

	let test_run = Command::new(byond_path.join(DREAMDAEMON_EXE))
		.arg(base_path.join("auxsigcheck.dmb"))
		.args(["-trusted", "-invisible"])
		.output()
		.wrap_err("failed to run DreamDaemon")?;

	// cleanup env variables
	std::env::remove_var("AUXTOOLS_DLL");
	if !test_run.status.success() {
		panic!("Failed to run auxsigcheck.dmb, DreamDaemon exited with code {status}")
	}

	let stderr = String::from_utf8_lossy(&test_run.stderr).into_owned();
	match extract_test_result(&stderr) {
		TestResult::Success => println!("success"),
		TestResult::Failed(reason) => println!("failed: {reason}"),
		TestResult::Missing(sigs) => println!("missing: {sigs}")
	}

	Ok(())
}
