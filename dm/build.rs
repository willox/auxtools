fn main() {
	cc::Build::new()
		.file("src/hooks_opcode.asm")
		.file("src/hooks.cpp")
		.file("src/raw_types/funcs.cpp")
		.cpp(true)
		.compile("dm-cpp");
}
