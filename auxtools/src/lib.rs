use dm::*;

#[hook("/proc/react")]
fn hello_proc_hook(M: Value) {
	let x = Value::from("test");

	let mut string : *mut raw_types::strings::StringEntry = std::ptr::null_mut();

	unsafe {
		let id = x.value.data.string;
		assert_eq!(raw_types::funcs::get_string_table_entry(&mut string, id), 1);
	}

	M.call("print", &[&Value::from(4.0)]);
	M.call("print", &[&x]);
	M.call("print", &[&x]);

	Ok(Value::from("Hello"))
}