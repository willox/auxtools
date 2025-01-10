use std::env;

fn main() {
	let target_family = env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set");
	let target_env = env::var("CARGO_CFG_TARGET_ENV").expect("CARGO_CFG_TARGET_ENV not set");

	let mut build = cc::Build::new();
	build.cpp(true).file("src/execute_instruction_data.cpp");

	match target_family.as_str() {
		"unix" => {
			build.file("src/execute_instruction_hook.unix.S");
		}
		"windows" => match target_env.as_str() {
			"gnu" => {
				build.file("src/execute_instruction_hook.windows.S");
				build.file("src/514/execute_instruction_hook.windows.S");
			}
			"msvc" => {
				build.file("src/execute_instruction_hook.windows.asm");
				build.file("src/514/execute_instruction_hook.windows.asm");
			}
			other => panic!("don't know how to build hook for family=\"windows\", env={:?}", other)
		},
		other => panic!("don't know how to build hook for family={:?}", other)
	}

	build.compile("instruction-hooking-cpp");
}
