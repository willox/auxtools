use auxtools::*;

#[hook("/proc/auxtest_lists")]
fn test_lists() {
	let list_a = List::new();

	// Should be empty
	if list_a.len() != 0 {
		return Err(runtime!("test_lists: list_a's len != 0"));
	}

	// Add 3 values
	list_a.append(&Value::from(101));
	list_a.append(&Value::from(102));
	list_a.append(&Value::from(103));

	// Should contain 3 things
	if list_a.len() != 3 {
		return Err(runtime!("test_lists: list_a's len != 3"));
	}

	// Now we become assoc
	list_a.set(&Value::from_string("key"), &Value::from_string("value"))?;

	if list_a.get(&Value::from_string("key"))?.as_string()? != "value" {
		return Err(runtime!("test_lists: list_a[2] != 102"));
	}

	// Should contain 4 things
	if list_a.len() != 4 {
		return Err(runtime!("test_lists: list_a's len != 4"));
	}

	// Remove list_a[2]
	list_a.remove(&Value::from(102));

	// Now list_a[2] should be 103
	if list_a.get(2)?.as_number()? != 103.0 {
		return Err(runtime!("test_lists: list_a[2] != 103"));
	}

	Ok(Value::from(true))
}
