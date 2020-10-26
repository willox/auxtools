use dm::*;

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
