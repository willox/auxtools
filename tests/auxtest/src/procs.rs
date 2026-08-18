use auxtools::*;

#[hook("/proc/auxtest_proc_find")]
fn proc_find() {
	let proc = Proc::find("/proc/concat_strings").ok_or_else(|| runtime!("proc_find: /proc/concat_strings not found"))?;

	if proc.path != "/concat_strings" {
		return Err(runtime!("proc_find: unexpected proc path {}", proc.path));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_proc_call_global")]
fn proc_call_global() {
	let value_a = Value::from_string("relatively unique testing string")?;
	let value_b = Value::from_string("another string that should be unique")?;
	let concatenated = Proc::find("/proc/concat_strings")
		.ok_or_else(|| runtime!("proc_call_global: /proc/concat_strings not found"))?
		.call(&[&value_a, &value_b])?;

	let expected = "relatively unique testing stringanother string that should be unique";
	let actual = concatenated.as_string()?;
	if actual != expected {
		return Err(runtime!("proc_call_global: expected {:?}, got {:?}", expected, actual));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_proc_metadata")]
fn proc_metadata() {
	let proc =
		Proc::find("/proc/auxtest_proc_metadata_subject").ok_or_else(|| runtime!("proc_metadata: /proc/auxtest_proc_metadata_subject not found"))?;

	let parameter_names = proc.parameter_names().into_iter().map(String::from).collect::<Vec<_>>();
	if parameter_names != ["alpha", "beta"] {
		return Err(runtime!("proc_metadata: unexpected parameter names {:?}", parameter_names));
	}

	let local_names = proc.local_names().into_iter().map(String::from).collect::<Vec<_>>();
	if !local_names.iter().any(|name| name == "local_one") {
		return Err(runtime!("proc_metadata: local_one not found in {:?}", local_names));
	}

	Ok(Value::from(true))
}
