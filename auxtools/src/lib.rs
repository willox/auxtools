#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/install_instruction")]
fn hello_proc_hook() {
	Ok(Value::from(1337))
}
