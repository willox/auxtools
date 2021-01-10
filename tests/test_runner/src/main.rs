use std::path::PathBuf;
use std::process::Command;

fn find_dm() -> PathBuf {
	let mut path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
	path.push("bin\\dm.exe");
	assert!(path.is_file(), "couldn't find dreammaker");
	path
}

fn find_dreamdaemon() -> PathBuf {
	let mut path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
	path.push("bin\\dreamdaemon.exe");
	assert!(path.is_file(), "couldn't find dreamdaemon");
	path
}

fn find_dll() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();
	path.push("auxtest.dll");
	assert!(path.is_file(), "couldn't find auxtest");
	path
}

fn find_dme() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests\\auxtest_host\\auxtest_host.dme");
	assert!(path.is_file(), "couldn't find auxtest_host.dme");
	path
}

fn find_dmb() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests\\auxtest_host\\auxtest_host.dmb");
	assert!(path.is_file(), "couldn't find auxtest_host.dmb");
	path
}

fn main() {
	let res = Command::new(find_dm()).arg(find_dme()).status().unwrap();
	assert!(res.success(), "dreamdaemon build failed");

	// Here we depend on BYOND not fucking with stderr so we can hijack it for our own communication
	let output = Command::new(find_dreamdaemon())
		.env("AUXTEST_DLL", find_dll())
		.arg(find_dmb())
		.arg("-trusted")
		.arg("-close")
		.output()
		.unwrap()
		.stderr;

	println!("{:?}", std::str::from_utf8(&output));
}
