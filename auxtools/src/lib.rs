#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;
use std::thread;
use std::time::Duration;

#[hook("/proc/hooked")]
fn hello_proc_hook() {
	let obj = &args[0];
	let vars = obj.get_list("vars")?;

	let mut var_names = Vec::new();

	for i in 1..=vars.len() {
		let name = vars.get(i)?;
		var_names.push(name.as_string()?);
	}

	Ok(Value::null())
}

#[hook("/proc/auxtools_download_file")]
fn download_file(url: Value, cb: Value) {
	let url = url.as_string()?;
	let cb = Callback::new(cb)?;

	thread::spawn(move || {
		thread::sleep(Duration::from_secs(3));
		let result = "Top secret file contents";
		cb.invoke(move || vec![Value::from_string(result)]);
	});

	Ok(Value::from_string(format!(
		"Starting download of {}...",
		url
	)))
}
