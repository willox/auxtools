use auxtools::*;

#[hook("/proc/auxtest_lists")]
fn test_lists() {
	let list_a = List::new();

	// Should be empty
	if !list_a.is_empty() {
		return Err(runtime!("test_lists: list_a's len != 0"));
	}

	// Add 3 values
	list_a.append(Value::from(101));
	list_a.append(Value::from(102));
	list_a.append(Value::from(103));

	// Should contain 3 things
	if list_a.len() != 3 {
		return Err(runtime!("test_lists: list_a's len != 3"));
	}

	// Now we become assoc
	list_a.set(byond_string!("key"), byond_string!("value"))?;

	if list_a.get(byond_string!("key"))?.as_string()? != "value" {
		return Err(runtime!("test_lists: list_a[2] != 102"));
	}

	// Should contain 4 things
	if list_a.len() != 4 {
		return Err(runtime!("test_lists: list_a's len != 4"));
	}

	// Remove list_a[2]
	list_a.remove(Value::from(102));

	// Now list_a[2] should be 103
	if list_a.get(2)?.as_number()? != 103.0 {
		return Err(runtime!("test_lists: list_a[2] != 103"));
	}

	let list_b = List::with_size(6);

	// This list should have 6 nulls in it
	if list_b.len() != 6 {
		return Err(runtime!("test_lists: list_b's len != 6"));
	}

	for n in 1..=6 {
		if list_b.get(n)? != Value::NULL {
			return Err(runtime!("test_lists: list_b[{}] != null", n));
		}
	}

	Ok(Value::from(true))
}
