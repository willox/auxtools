use std::env;

fn main() {
	if env::var("CARGO_CFG_TARGET_FAMILY").unwrap() == "unix" {
		cc::Build::new()
			.file("src/execute_instruction_hook.S")
			.file("src/execute_instruction_data.cpp")
			.cpp(true)
			.compile("debug-server-cpp");
	}

	if env::var("CARGO_CFG_TARGET_FAMILY").unwrap() == "windows" {
		cc::Build::new()
			.file("src/execute_instruction_hook.asm")
			.file("src/execute_instruction_data.cpp")
			.cpp(true)
			.compile("debug-server-cpp");
	}
}
