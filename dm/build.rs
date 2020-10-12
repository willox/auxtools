use cc;

fn main() {
	cc::Build::new().file("src/hooks.cpp").compile("hooks-cpp");
}
