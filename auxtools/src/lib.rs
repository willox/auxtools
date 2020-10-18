use dm::*;

#[hook("/proc/react")]
fn hello_proc_hook(some_datum: Value) {
	Ok(Value::from("Hello"))
}
