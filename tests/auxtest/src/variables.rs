use auxtools::*;

#[hook("/proc/auxtest_variables_get_set")]
fn variables_get_set(datum: Value) {
	datum.set(byond_string!("auxtest_number"), Value::from(42))?;

	if datum.get_number(byond_string!("auxtest_number"))? != 42.0 {
		return Err(runtime!("variables_get_set: datum.auxtest_number != 42"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_value_to_string")]
fn value_to_string(datum: Value) {
	let string = datum.to_string()?;
	if string.is_empty() {
		return Err(runtime!("value_to_string: datum string was empty"));
	}

	Ok(Value::from(true))
}
