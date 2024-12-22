use std::{collections::HashMap, convert::TryFrom};

use auxtools::*;

#[hook("/proc/auxtest_value_from")]
fn test_value_from() {
	// Numbers
	let value = Value::from(30);
	if value.as_number()? != 30.0 {
		return Err(runtime!("value_from: Value failed to convert i32"));
	}

	// Vectors
	// The simplest case: A Vec of Value's.
	let vector: Vec<Value> = vec![5.into()];
	let value = Value::from(&vector);
	let list = List::from_value(&value)?;
	if list.len() != 1 {
		return Err(runtime!("value_from: Vec with one entry did not result in a list length of one"));
	}
	let value = list.get(1)?.as_number()?;
	if value != 5.0 {
		return Err(runtime!(
			"value_from: Instead of containing 5 at index 1, list from vec contained {}",
			value
		));
	};

	// Hashmaps
	// The simplest case: Value -> Value
	let mut hashmap: HashMap<Value, Value> = HashMap::new();
	hashmap.insert(Value::from_string("meow")?, 1.into());
	let value = Value::try_from(&hashmap)?;
	assert_meow_equals_one(value)?;

	// Slightly more complicated: String -> Value
	let mut hashmap: HashMap<String, Value> = HashMap::new();
	hashmap.insert("meow".to_owned(), 1.into());
	let value = Value::try_from(&hashmap)?;
	assert_meow_equals_one(value)?;

	// Todo: Other stuff

	Ok(Value::from(true))
}

fn assert_meow_equals_one(value: Value) -> Result<(), Runtime> {
	match value.raw.tag {
		raw_types::values::ValueTag::List => (),
		_ => {
			return Err(runtime!(
				"value_from: Hashmap became a ValueTag::{:?} instead of ValueTag::List",
				value.raw.tag
			))
		}
	}

	let list = List::from_value(&value)?;

	if list.len() != 1 {
		return Err(runtime!("value_from: Hashmap with one key did not result in a list length of one"));
	}

	let value = list.get(byond_string!("meow"))?.as_number()?;
	if value != 1.0 {
		return Err(runtime!(
			"value_from: Instead of containing 1 at index `meow`, list from hashmap contained {}",
			value
		));
	};

	Ok(())
}
