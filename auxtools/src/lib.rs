#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/hooked")]
fn hello_proc_hook(cb: Value) {
	let cb = Callback::new(cb).unwrap();
	std::thread::spawn(move || {
		std::thread::sleep(std::time::Duration::from_secs(1));
		cb.invoke(|| vec![Value::from(1337)]);
	});

	Ok(Value::null())
}
