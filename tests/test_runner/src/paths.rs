use std::{path::PathBuf, process::Command};

pub trait ByondCommand {
	fn with_byond_paths(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl ByondCommand for Command {
	// TODO: This doesn't read very nice
	fn with_byond_paths(&mut self) -> &mut Command {
		let byond_system = find_byond();
		let byond_bin = find_byond_bin();

		let path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("PATH").unwrap().to_str().unwrap()
		);

		let ld_library_path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("LD_LIBRARY_PATH").unwrap().to_str().unwrap()
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

pub fn find_byond() -> PathBuf {
	let path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
	assert!(path.is_dir(), "couldn't find byond");
	path
}

#[allow(dead_code)]
pub fn find_byond_bin() -> PathBuf {
	let mut path = find_byond();
	path.push("bin");
	assert!(path.is_dir(), "couldn't find byond/bin");
	path
}

pub fn find_dm() -> PathBuf {
	let mut path = find_byond();

	#[cfg(unix)]
	path.push("bin/DreamMaker");

	#[cfg(windows)]
	path.push("bin/dm.exe");

	assert!(path.is_file(), "couldn't find dreammaker");
	path
}

pub fn find_dreamdaemon() -> PathBuf {
	let mut path = find_byond();

	#[cfg(unix)]
	path.push("bin/DreamDaemon");

	#[cfg(windows)]
	path.push("bin/dd.exe");

	assert!(path.is_file(), "couldn't find dreamdaemon");
	path
}

pub fn find_dll() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libauxtest.so");

	#[cfg(windows)]
	path.push("auxtest.dll");

	assert!(path.is_file(), "couldn't find auxtest");
	path
}

pub fn find_dme() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/auxtest_host/auxtest_host.dme");
	assert!(path.is_file(), "couldn't find auxtest_host.dme");
	path
}

pub fn find_dmb() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/auxtest_host/auxtest_host.dmb");
	assert!(path.is_file(), "couldn't find auxtest_host.dmb");
	path
}
