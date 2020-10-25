use dm::*;

#[hook("/datum/gas_mixture/proc/react")]
fn hello_proc_hook() {
	Ok(Value::null())
}
