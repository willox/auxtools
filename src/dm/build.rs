use cc;

fn main() {
	cc::Build::new().file("hooks.cpp").compile("hooks-cpp");
}
