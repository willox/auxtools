fn main() {
	cc::Build::new()
		.file("src/execute_instruction_hook.asm")
		.file("src/execute_instruction_data.cpp")
		.cpp(true)
		.compile("debug-server-cpp");
}
