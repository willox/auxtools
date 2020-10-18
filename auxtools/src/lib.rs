use dm::*;

#[hook("/datum/gas_mixture/proc/react")]
fn hello_proc_hook() {
	let x = Value::from_string("test")?;
	Ok(Value::from_string("Hello")?)
}
