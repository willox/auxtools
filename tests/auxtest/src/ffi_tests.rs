use std::{cell::RefCell, collections::HashMap, convert::TryFrom, ffi::CString, os::raw::c_char};

use auxtools::*;

fn test_result(name: &str, test: impl FnOnce() -> Result<(), Runtime>) -> Option<String> {
	match test() {
		Ok(()) => Some("SUCCESS".to_owned()),
		Err(err) => Some(format!("FAILED: {}: {}", name, err.message)),
	}
}

thread_local! {
	static RETURN_STRING: RefCell<CString> = RefCell::new(CString::default());
}

fn byond_return(value: Option<String>) -> *const c_char {
	RETURN_STRING.with(|cell| {
		cell.replace(CString::new(value.unwrap_or_default()).unwrap_or_default());
		cell.borrow().as_ptr()
	})
}

macro_rules! ffi_test {
	($name:ident, $body:block) => {
		#[no_mangle]
		extern "C" fn $name(_argc: std::os::raw::c_int, _argv: *const *const c_char) -> *const c_char {
			byond_return($body)
		}
	};
}

ffi_test! { auxtest_ffi_runtime_globals, {
	test_result("ffi_runtime_globals", || unsafe {
		if raw_types::funcs::CURRENT_EXECUTION_CONTEXT.is_null() {
			return Err(runtime!("CURRENT_EXECUTION_CONTEXT is null"));
		}

		if (*raw_types::funcs::CURRENT_EXECUTION_CONTEXT).is_null() {
			return Err(runtime!("current execution context is null"));
		}

		if raw_types::funcs::SUSPENDED_PROCS.is_null() {
			return Err(runtime!("SUSPENDED_PROCS is null"));
		}

		if raw_types::funcs::SUSPENDED_PROCS_BUFFER.is_null() {
			return Err(runtime!("SUSPENDED_PROCS_BUFFER is null"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_string_id_and_entry, {
	test_result("ffi_string_id_and_entry", || unsafe {
		use raw_types::{funcs, strings};

		let contents = CString::new("relatively unique testing string").unwrap();
		let mut id = strings::StringId(0);
		let mut entry: *mut strings::StringEntry = std::ptr::null_mut();

		if funcs::get_string_id(&mut id, contents.as_ptr()) != 1 {
			return Err(runtime!("get_string_id failed"));
		}

		if funcs::get_string_table_entry(&mut entry, id) != 1 {
			return Err(runtime!("get_string_table_entry failed"));
		}

		if entry.is_null() {
			return Err(runtime!("string entry is null"));
		}

		if std::ffi::CStr::from_ptr((*entry).data).to_bytes() != contents.as_bytes() {
			return Err(runtime!("string table entry data mismatch"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_string_value_refcount, {
	test_result("ffi_string_value_refcount", || unsafe {
		use raw_types::{funcs, strings, values};

		let contents = CString::new("string refcount test").unwrap();
		let mut id = strings::StringId(0);
		let mut entry: *mut strings::StringEntry = std::ptr::null_mut();

		assert_eq!(funcs::get_string_id(&mut id, contents.as_ptr()), 1);
		assert_eq!(funcs::get_string_table_entry(&mut entry, id), 1);

		if (*entry).ref_count != 0 {
			return Err(runtime!("new string refcount != 0"));
		}

		{
			let value = Value::new(values::ValueTag::String, values::ValueData { string: id });
			if (*entry).ref_count != 1 {
				return Err(runtime!("Value::new did not increment refcount"));
			}
			drop(value);
		}

		if (*entry).ref_count != 0 {
			return Err(runtime!("Value drop did not decrement refcount"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_string_value_roundtrip, {
	test_result("ffi_string_value_roundtrip", || {
		let value = Value::from_string("roundtrip string")?;
		if value.as_string()? != "roundtrip string" {
			return Err(runtime!("string did not roundtrip"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_proc_find, {
	test_result("ffi_proc_find", || {
		let proc = Proc::find("/proc/concat_strings").ok_or_else(|| runtime!("/proc/concat_strings not found"))?;

		if proc.path != "/concat_strings" {
			return Err(runtime!("unexpected proc path {}", proc.path));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_proc_call_global, {
	test_result("ffi_proc_call_global", || {
		let value_a = Value::from_string("relatively unique testing string")?;
		let value_b = Value::from_string("another string that should be unique")?;
		let concatenated = Proc::find("/proc/concat_strings")
			.ok_or_else(|| runtime!("/proc/concat_strings not found"))?
			.call(&[&value_a, &value_b])?;

		let expected = "relatively unique testing stringanother string that should be unique";
		let actual = concatenated.as_string()?;
		if actual != expected {
			return Err(runtime!("expected {:?}, got {:?}", expected, actual));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_proc_metadata, {
	test_result("ffi_proc_metadata", || {
		let proc = Proc::find("/proc/auxtest_proc_metadata_subject")
			.ok_or_else(|| runtime!("/proc/auxtest_proc_metadata_subject not found"))?;

		let parameter_names = proc
			.parameter_names()
			.into_iter()
			.map(String::from)
			.collect::<Vec<_>>();
		if parameter_names != ["alpha", "beta"] {
			return Err(runtime!("unexpected parameter names {:?}", parameter_names));
		}

		let local_names = proc.local_names().into_iter().map(String::from).collect::<Vec<_>>();
		if !local_names.iter().any(|name| name == "local_one") {
			return Err(runtime!("local_one not found in {:?}", local_names));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_create_empty, {
	test_result("ffi_list_create_empty", || {
		let list = List::new();
		if !list.is_empty() {
			return Err(runtime!("list len != 0"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_append_len, {
	test_result("ffi_list_append_len", || {
		let list = List::new();
		list.append(Value::from(101));
		list.append(Value::from(102));
		list.append(Value::from(103));

		if list.len() != 3 {
			return Err(runtime!("list len != 3"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_index_get, {
	test_result("ffi_list_index_get", || {
		let list = List::new();
		list.append(Value::from(101));
		list.append(Value::from(102));

		if list.get(2)?.as_number()? != 102.0 {
			return Err(runtime!("list[2] != 102"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_assoc_set_get, {
	test_result("ffi_list_assoc_set_get", || {
		let list = List::new();
		let key = Value::from_string("key")?;
		let value = Value::from_string("value")?;
		list.set(&key, &value)?;

		if list.get(&key)?.as_string()? != "value" {
			return Err(runtime!("list[key] != value"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_remove, {
	test_result("ffi_list_remove", || {
		let list = List::new();
		list.append(Value::from(101));
		list.append(Value::from(102));
		list.append(Value::from(103));
		list.remove(Value::from(102));

		if list.get(2)?.as_number()? != 103.0 {
			return Err(runtime!("list[2] != 103 after remove"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_list_with_size, {
	test_result("ffi_list_with_size", || {
		let list = List::with_size(6);

		if list.len() != 6 {
			return Err(runtime!("list len != 6"));
		}

		for n in 1..=6 {
			if list.get(n)? != Value::NULL {
				return Err(runtime!("list[{}] != null", n));
			}
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_value_from_number, {
	test_result("ffi_value_from_number", || {
		let value = Value::from(30);
		if value.as_number()? != 30.0 {
			return Err(runtime!("Value failed to convert i32"));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_value_from_vec, {
	test_result("ffi_value_from_vec", || {
		let vector: Vec<Value> = vec![5.into()];
		let value = Value::from(&vector);
		let list = List::from_value(&value)?;
		if list.len() != 1 {
			return Err(runtime!("Vec with one entry did not produce len 1"));
		}

		let value = list.get(1)?.as_number()?;
		if value != 5.0 {
			return Err(runtime!("list[1] was {} instead of 5", value));
		}

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_value_from_hashmap_value_key, {
	test_result("ffi_value_from_hashmap_value_key", || {
		let mut hashmap: HashMap<Value, Value> = HashMap::new();
		hashmap.insert(Value::from_string("meow")?, 1.into());
		let value = Value::try_from(&hashmap)?;
		assert_meow_equals_one(value)?;

		Ok(())
	})
} }

ffi_test! { auxtest_ffi_value_from_hashmap_string_key, {
	test_result("ffi_value_from_hashmap_string_key", || {
		let mut hashmap: HashMap<String, Value> = HashMap::new();
		hashmap.insert("meow".to_owned(), 1.into());
		let value = Value::try_from(&hashmap)?;
		assert_meow_equals_one(value)?;

		Ok(())
	})
} }

fn assert_meow_equals_one(value: Value) -> Result<(), Runtime> {
	match value.raw.tag {
		raw_types::values::ValueTag::List => (),
		_ => return Err(runtime!("Hashmap became a {:?} instead of a list", value.raw.tag)),
	}

	let list = List::from_value(&value)?;
	if list.len() != 1 {
		return Err(runtime!("Hashmap with one key did not produce len 1"));
	}

	let value = list.get(Value::from_string("meow")?)?.as_number()?;
	if value != 1.0 {
		return Err(runtime!("list[meow] was {} instead of 1", value));
	}

	Ok(())
}
