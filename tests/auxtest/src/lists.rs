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

	// is_list should agree with DM's own islist() on everything, and len() with
	// length() on everything that is a list
	let samples = Proc::find("/proc/auxtest_islist_samples").unwrap().call(&[])?.as_list()?;
	let dm_islist = Proc::find("/proc/auxtest_islist").unwrap();
	let dm_length = Proc::find("/proc/auxtest_length").unwrap();
	for n in 1..=samples.len() {
		let sample = samples.get(n)?;
		let expected = dm_islist.call(&[&sample])?.as_number()? != 0.0;
		if List::is_list(&sample) != expected {
			return Err(runtime!("test_lists: is_list disagrees with islist() on sample {}", n));
		}
		if expected && sample.as_list()?.len() as f32 != dm_length.call(&[&sample])?.as_number()? {
			return Err(runtime!("test_lists: len disagrees with length() on sample {}", n));
		}
	}

	let sample = Proc::find("/proc/auxtest_alist_sample").unwrap().call(&[])?;
	let alist = sample.as_list()?;
	if !alist.is_alist() || list_a.is_alist() {
		return Err(runtime!("test_lists: is_alist is wrong"));
	}
	if alist.get(byond_string!("a"))?.as_number()? != 1.0 {
		return Err(runtime!("test_lists: alist[\"a\"] != 1"));
	}
	// a number is a key here, not a position
	if alist.get(7)?.as_string()? != "seven" {
		return Err(runtime!("test_lists: alist[7] != \"seven\""));
	}

	let keys = Proc::find("/proc/auxtest_alist_keys").unwrap().call(&[&sample])?.as_list()?;
	let pairs = alist.alist_pairs()?;
	if pairs.len() != 3 || pairs.len() != keys.len() as usize {
		return Err(runtime!("test_lists: alist_pairs found {} pairs, not 3", pairs.len()));
	}
	for (n, (key, value)) in pairs.iter().enumerate() {
		if *key != keys.get(n as u32 + 1)? {
			return Err(runtime!("test_lists: alist_pairs key {} is out of DM's order", n + 1));
		}
		if *value != alist.get(key)? {
			return Err(runtime!("test_lists: alist_pairs value {} != alist[key]", n + 1));
		}
	}

	alist.set(8, byond_string!("eight"))?;
	if alist.get(8)?.as_string()? != "eight" || alist.len() != 4 {
		return Err(runtime!("test_lists: set on an alist didn't stick"));
	}

	if list_a.alist_pairs().is_ok() {
		return Err(runtime!("test_lists: alist_pairs accepted a plain list"));
	}

	Ok(Value::from(true))
}
