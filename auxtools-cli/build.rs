//! Use `auxtools-cli.pdb` for the launcher so it does not overwrite the
//! `auxtools` library's PDB in the same output directory.

use std::path::PathBuf;

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
		return;
	}
	// OUT_DIR is somewhere under target/[TRIPLE/]PROFILE/build, and the
	// program is built in target/[TRIPLE/]PROFILE
	let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
	let profile = out
		.ancestors()
		.find(|dir| dir.file_name().is_some_and(|name| name == "build"))
		.and_then(|build| build.parent());
	if let Some(profile) = profile {
		println!("cargo:rustc-link-arg-bin=auxtools=/PDB:{}", profile.join("auxtools-cli.pdb").display());
	}
}
