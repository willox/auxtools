use std::ffi::CString;

use auxtools::*;

#[hook("/proc/auxtest_strings")]
fn test_strings() {
	use raw_types::{funcs, strings, values};

	unsafe {
		let string_a_contents = CString::new("relatively unique testing string").unwrap();
		let string_b_contents = CString::new("another string that should be unique").unwrap();

		let mut string_a = strings::StringId(0);
		let mut string_b = strings::StringId(0);
		let mut string_a_entry: *mut strings::StringEntry = std::ptr::null_mut();
		let mut string_b_entry: *mut strings::StringEntry = std::ptr::null_mut();
		{
			assert_eq!(funcs::get_string_id(&mut string_a, string_a_contents.as_ptr()), 1);
			assert_eq!(funcs::get_string_id(&mut string_b, string_b_contents.as_ptr()), 1);
			assert_eq!(funcs::get_string_table_entry(&mut string_a_entry, string_a), 1);
			assert_eq!(funcs::get_string_table_entry(&mut string_b_entry, string_b), 1);
		}

		// New strings should start with a reference count of 0.
		if (*string_a_entry).ref_count != 0 {
			return Err(runtime!("test_string: string_a's reference count != 0"));
		}

		if (*string_b_entry).ref_count != 0 {
			return Err(runtime!("test_string: string_a's reference count != 0"));
		}

		// Creating a value from our strings should result in both having a
		// reference count of 1.
		let value_a = Value::new(values::ValueTag::String, values::ValueData { string: string_a });

		let value_b = Value::new(values::ValueTag::String, values::ValueData { string: string_b });

		if (*string_a_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after Value::new"));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after Value::new"));
		}

		let concatenated = Proc::find("/proc/concat_strings").unwrap().call(&[&value_a, &value_b])?;

		// Returned value should be equal to string_a_contents ..
		// string_b_contents and have a ref count of 1
		if concatenated.raw.tag != values::ValueTag::String {
			return Err(runtime!("test_string: concat_strings did not return a string"));
		}

		let mut concatenated_entry: *mut strings::StringEntry = std::ptr::null_mut();
		assert_eq!(funcs::get_string_table_entry(&mut concatenated_entry, concatenated.raw.data.string), 1);

		if (*concatenated_entry).ref_count != 1 {
			return Err(runtime!("test_string: concatenated's reference count != 1 after concat_strings()"));
		}

		let expected_concat = format!("{}{}", string_a_contents.to_str().unwrap(), string_b_contents.to_str().unwrap());
		let actual_concat = concatenated.as_string()?;

		if actual_concat != expected_concat {
			return Err(runtime!("test_string: expected_concat != actual_concat"));
		}

		// The strings should still have a reference count of 1 after
		// concat_strings has used them
		if (*string_a_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after concat_strings"));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after concat_strings"));
		}

		// Rust strings are UTF-8: "aé" has three bytes and two characters.
		let accented = Value::from_string("a\u{e9}")?;
		let lengths = Proc::find("/proc/text_lengths").unwrap().call(&[&accented])?.as_list()?;
		let lengths = (1..=3)
			.map(|i| lengths.get(i as f32).and_then(|v| v.as_number()))
			.collect::<Result<Vec<_>, _>>()?;
		if lengths != [3.0, 2.0, 233.0] {
			return Err(runtime!(
				"test_string: \"a\u{e9}\" has length, length_char and its second character {:?}, not [3, 2, 233]",
				lengths
			));
		}

		// DreamMaker and dmasm encode "\xff\x41" as UTF-8 ÿ followed by A.
		// Latin-1 conversion would corrupt the second character.
		let escaped = Value::from_string_raw(b"\xc3\xbf\x41")?;
		let lengths = Proc::find("/proc/text_lengths").unwrap().call(&[&escaped])?.as_list()?;
		let second = lengths.get(3.0).and_then(|v| v.as_number())?;
		if second != 65.0 {
			return Err(runtime!("test_string: the second character of \"\\xff\\x41\" is {}, not 65", second));
		}

		Ok(Value::from(true))
	}
}
