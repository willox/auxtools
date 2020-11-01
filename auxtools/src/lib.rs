#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/hooked")]
fn hello_proc_hook() {
	let obj = &args[0];
	let vars = obj.get_list("vars")?;

	let var_names: Vec<String> = vars
		.to_vec()
		.iter()
		.map(|v| v.as_string().unwrap_or("".to_owned()))
		.collect();

	Ok(Value::null())
}
