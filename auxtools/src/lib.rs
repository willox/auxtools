use dm::*;

#[hook("/datum/gas_mixture/proc/print")]
fn hello_proc_hook() {
	let x = Value::from_string("test")?;

	let mut string : *mut raw_types::strings::StringEntry = std::ptr::null_mut();

	unsafe {
		let id = x.value.data.string;
		assert_eq!(raw_types::funcs::get_string_table_entry(&mut string, id), 1);
	}

	Ok(Value::from_string("Hello")?)
}