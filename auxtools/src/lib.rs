use dm::*;

#[hook("/proc/react")]
fn hello_proc_hook(some_datum: value::Value) {
	Ok(value::Value::from("Hello"))
}
