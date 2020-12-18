use auxtools::*;
use std::ffi::CString;

#[hook("/proc/auxtest_out")]
fn out(msg: Value) {
	use std::io::{self, Write};

	io::stdout().write_all(b"Fuck").unwrap();
	Ok(Value::null())
}

#[hook("/proc/auxtest_strings")]
fn test_strings() {
	use raw_types::funcs;
	use raw_types::values;
	use raw_types::strings;

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
			return Err(runtime!("test_string: string_a's reference count != 1 after Value::new"));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after Value::new"));
		}

		let concatenated = Proc::find("/proc/concat_strings").unwrap().call(&[&value_a, &value_b])?;

		// Returned value should be equal to string_a_contents .. string_b_contents
		// and have a ref count of 1
		if concatenated.value.tag != values::ValueTag::String {
			return Err(runtime!("test_string: concat_strings did not return a string"));
		}

		let mut concatenated_entry: *mut strings::StringEntry = std::ptr::null_mut();
		assert_eq!(
			funcs::get_string_table_entry(&mut concatenated_entry, concatenated.value.data.string),
			1
		);

		if (*concatenated_entry).ref_count != 1 {
			return Err(runtime!("test_string: concatenated's reference count != 1 after concat_strings()"));
		}

		let expected_concat = format!("{}{}", string_a_contents.to_str().unwrap(), string_b_contents.to_str().unwrap());
		let actual_concat = concatenated.as_string()?;

		if actual_concat != expected_concat {
			return Err(runtime!("test_string: expected_concat != actual_concat"));
		}

		// The strings should still have a reference count of 1 after concat_strings has used them
		if (*string_a_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after concat_strings"));
		}

		if (*string_b_entry).ref_count != 1 {
			return Err(runtime!("test_string: string_a's reference count != 1 after concat_strings"));
		}

		Ok(Value::from(true))
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;
	use std::process::Command;
	use std::net::TcpListener;

	fn find_dm() -> PathBuf {
		let mut path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
		path.push("bin\\dm.exe");
		assert!(path.is_file());
		path
	}

	fn find_dreamdaemon() -> PathBuf {
		let mut path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
		path.push("bin\\dreamdaemon.exe");
		assert!(path.is_file());
		path
	}

	#[test]
	fn run() {
		let dll = test_cdylib::build_current_project();

		println!("{:?}", dll);

		let res = Command::new(find_dm())
			.arg("auxtest.dme")
			.status()
			.unwrap();
		assert!(res.success());

		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let port = listener.local_addr().unwrap().port();

		let output = Command::new(find_dreamdaemon())
			.env("AUXTEST_PORT", port.to_string())
			.env("AUXTEST_DLL", dll)
			.arg("auxtest.dmb")
			.arg("-trusted")
			.arg("-close")
			.output()
			.unwrap()
			.stderr;
		println!("{:?}", output);

		panic!("fak");
/*
		match listener.accept() {
			Ok((socket, _)) => {

			},

			Err(e) => panic!(e)
		}
*/
	}
}
