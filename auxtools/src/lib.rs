use dm::*;

#[hook("/datum/gas_mixture/proc/react")]
fn hello_proc_hook() {
	let x = src.get("a");

	Ok(Value::null())
}
