use std::path::PathBuf;

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
	path.push("bin/dreamdaemon.exe");

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
