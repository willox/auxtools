use auxtools::*;

#[hook("/proc/auxtest_weak_values")]
fn test_weak_values(someval: Value) {
	let weak = someval.as_weak()?;

	if weak.upgrade().is_none() {
		return Err(runtime!("test_weak_values: Failed to upgrade weak reference to existing value"));
	}

	Proc::find("/proc/del_value")
		.ok_or_else(|| runtime!("test_weak_values: /proc/del_value not defined"))?
		.call(&[someval])?;

	Proc::find("/proc/create_datum_for_weak")
		.ok_or_else(|| runtime!("test_weak_values: /proc/create_datum_for_weak not defined"))?
		.call(&[])?;

	if weak.upgrade().is_some() {
		return Err(runtime!("test_weak_values: Upgraded a weak reference to deleted value"));
	}

	Ok(Value::from(true))
}
