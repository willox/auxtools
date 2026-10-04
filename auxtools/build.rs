fn main() {
	println!("cargo:rerun-if-changed=src/hooks.cpp");
	println!("cargo:rerun-if-changed=src/hooks.h");
	println!("cargo:rerun-if-changed=src/raw_types/funcs.cpp");

	cc::Build::new()
		.include("src/")
		.file("src/hooks.cpp")
		.file("src/raw_types/funcs.cpp")
		.cpp(true)
		.compile("auxtools-cpp");
}
