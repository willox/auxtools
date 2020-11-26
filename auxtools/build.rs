fn main() {
	cc::Build::new()
		.include("src/")
		.file("src/hooks.cpp")
		.file("src/raw_types/funcs.cpp")
		.cpp(true)
		.compile("auxtools-cpp");
}
