fn main() {
	println!("cargo:rerun-if-changed=src/hooks.cpp");
	println!("cargo:rerun-if-changed=src/hooks.h");
	println!("cargo:rerun-if-changed=src/raw_types/funcs.cpp");
	println!("cargo:rerun-if-changed=src/instruction_hook");

	cc::Build::new()
		.include("src/")
		.file("src/hooks.cpp")
		.file("src/raw_types/funcs.cpp")
		.cpp(true)
		.compile("auxtools-cpp");

	// The instruction hook's entry, in assembly: it has no calling convention
	// Rust knows
	let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set");
	let target_env = std::env::var("CARGO_CFG_TARGET_ENV").expect("CARGO_CFG_TARGET_ENV not set");
	let mut build = cc::Build::new();
	build.cpp(true).file("src/instruction_hook/execute_instruction_data.cpp");
	match target_family.as_str() {
		"unix" => {
			build.file("src/instruction_hook/execute_instruction_hook.unix.S");
		}
		"windows" => match target_env.as_str() {
			"gnu" => {
				build.file("src/instruction_hook/execute_instruction_hook.windows.S");
				build.file("src/instruction_hook/514/execute_instruction_hook.windows.S");
			}
			"msvc" => {
				build.file("src/instruction_hook/execute_instruction_hook.windows.asm");
				build.file("src/instruction_hook/514/execute_instruction_hook.windows.asm");
			}
			other => panic!("don't know how to build the instruction hook for family=\"windows\", env={:?}", other)
		},
		other => panic!("don't know how to build the instruction hook for family={:?}", other)
	}
	build.compile("instruction-hook-asm");
}
