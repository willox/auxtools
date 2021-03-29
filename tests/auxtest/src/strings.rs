use auxtools::*;
use std::ffi::CString;

#[hook("/proc/auxtest_strings")]
fn test_strings() {
	use raw_types::funcs;
	use raw_types::strings;
	use raw_types::values;

	unsafe {
		let string_a_contents = CString::new("relatively unique testing string").unwrap();
		let string_b_contents = CString::new("another string that should be unique").unwrap();

		let mut string_a = strings::StringId(0);
		let mut string_b = strings::StringId(0);
		let mut string_a_entry: *mut strings::StringEntry = std::ptr::null_mut();
		let mut string_b_entry: *mut strings::StringEntry = std::ptr::null_mut();
		{
			assert_eq!(
				funcs::get_string_id(&mut string_a, string_a_contents.as_ptr()),
				1
			);
			assert_eq!(
				funcs::get_string_id(&mut string_b, string_b_contents.as_ptr()),
				1
			);
			assert_eq!(
				funcs::get_string_table_entry(&mut string_a_entry, string_a),
				1
			);
			assert_eq!(
				funcs::get_string_table_entry(&mut string_b_entry, string_b),
				1
			);
		}

		// New strings should start with a reference count of 0.
		if (*string_a_entry).ref_count != 0 {
			return Err(runtime!("test_string: string_a's reference count != 0"));
		}

		if (*string_b_entry).ref_count != 0 {
			return Err(runtime!("test_string: string_a's reference count != 0"));
		}

		// Creating a value from our strings should result in both having a reference count of 1.
		let value_a = Value::new(
			values::ValueTag::String,
			values::ValueData { string: string_a },
		);

		let value_b = Value::new(
			values::ValueTag::String,
			values::ValueData { string: string_b },
		);

		if (*string_a_entry).ref_count != 1 {
			return Err(runtime!(
				"test_string: string_a's reference count != 1 after Value::new"
			));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!(
				"test_string: string_a's reference count != 1 after Value::new"
			));
		}

		let concatenated = Proc::find("/proc/concat_strings")
			.unwrap()
			.call(&[&value_a, &value_b])?;

		// Returned value should be equal to string_a_contents .. string_b_contents
		// and have a ref count of 1
		if concatenated.raw.tag != values::ValueTag::String {
			return Err(runtime!(
				"test_string: concat_strings did not return a string"
			));
		}

		let mut concatenated_entry: *mut strings::StringEntry = std::ptr::null_mut();
		assert_eq!(
			funcs::get_string_table_entry(&mut concatenated_entry, concatenated.raw.data.string),
			1
		);

		if (*concatenated_entry).ref_count != 1 {
			return Err(runtime!(
				"test_string: concatenated's reference count != 1 after concat_strings()"
			));
		}

		let expected_concat = format!(
			"{}{}",
			string_a_contents.to_str().unwrap(),
			string_b_contents.to_str().unwrap()
		);
		let actual_concat = concatenated.as_string()?;

		if actual_concat != expected_concat {
			return Err(runtime!("test_string: expected_concat != actual_concat"));
		}

		// The strings should still have a reference count of 1 after concat_strings has used them
		if (*string_a_entry).ref_count != 1 {
			return Err(runtime!(
				"test_string: string_a's reference count != 1 after concat_strings"
			));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!(
				"test_string: string_a's reference count != 1 after concat_strings"
			));
		}

		Ok(Value::from(true))
	}
}
