use std::env;

fn main() {
	let mut build = cc::Build::new();
	build.cpp(true).file("src/execute_instruction_data.cpp");

	match env::var("CARGO_CFG_TARGET_FAMILY").unwrap().as_str() {
		"unix" => {
			build.file("src/execute_instruction_hook.unix.S");
		}
		"windows" => match env::var("CARGO_CFG_TARGET_ENV").unwrap().as_str() {
			"gnu" => {
				build.file("src/execute_instruction_hook.windows.S");
			}
			"msvc" => {
				build.file("src/execute_instruction_hook.windows.asm");
			}
			other => panic!(
				"don't know how to build hook for family=\"windows\", env={:?}",
				other
			),
		},
		other => panic!("don't know how to build hook for family={:?}", other),
	}

	build.compile("debug-server-cpp");
}
