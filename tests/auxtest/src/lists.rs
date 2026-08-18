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

#[hook("/proc/auxtest_list_create_empty")]
fn list_create_empty() {
	let list = List::new();
	if !list.is_empty() {
		return Err(runtime!("list_create_empty: list len != 0"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_list_append_len")]
fn list_append_len() {
	let list = List::new();
	list.append(Value::from(101));
	list.append(Value::from(102));
	list.append(Value::from(103));

	if list.len() != 3 {
		return Err(runtime!("list_append_len: list len != 3"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_list_index_get")]
fn list_index_get() {
	let list = List::new();
	list.append(Value::from(101));
	list.append(Value::from(102));

	if list.get(2)?.as_number()? != 102.0 {
		return Err(runtime!("list_index_get: list[2] != 102"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_list_assoc_set_get")]
fn list_assoc_set_get() {
	let list = List::new();
	list.set(byond_string!("key"), byond_string!("value"))?;

	if list.get(byond_string!("key"))?.as_string()? != "value" {
		return Err(runtime!("list_assoc_set_get: list[key] != value"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_list_remove")]
fn list_remove() {
	let list = List::new();
	list.append(Value::from(101));
	list.append(Value::from(102));
	list.append(Value::from(103));
	list.remove(Value::from(102));

	if list.get(2)?.as_number()? != 103.0 {
		return Err(runtime!("list_remove: list[2] != 103 after remove"));
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_list_with_size")]
fn list_with_size() {
	let list = List::with_size(6);

	if list.len() != 6 {
		return Err(runtime!("list_with_size: list len != 6"));
	}

	for n in 1..=6 {
		if list.get(n)? != Value::NULL {
			return Err(runtime!("list_with_size: list[{}] != null", n));
		}
	}

	Ok(Value::from(true))
}
