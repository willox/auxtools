//! CLI entry point. `auxtools dap` runs the debug adapter on stdin/stdout;
//! other commands launch worlds or connect directly to their debug servers.

mod adapter;
mod cli;
mod code;
mod dap;
mod debuggee;
mod lines;
mod stddef;

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	cli::run(&args)
}
