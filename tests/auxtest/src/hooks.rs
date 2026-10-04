use auxtools::*;

const DISCARDED: &str = "a hook result nobody is going to look at";

#[hook("/datum/auxtest_discard/New")]
fn discard_new() {
	Value::from_string(DISCARDED)
}

#[hook("/proc/auxtest_hooks")]
fn test_hooks() {
	use raw_types::{funcs, strings};

	// held for the whole test so the string can't be freed and remade under us
	let held = StringRef::new(DISCARDED)?;

	unsafe {
		let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
		assert_eq!(funcs::get_string_table_entry(&mut entry, held.get_id()), 1);

		// BYOND runs New() with the "don't want the result" bit set and never
		// releases what comes back, so the hook's result has to be dropped for it
		let before = (*entry).ref_count;
		Proc::find("/proc/auxtest_new_discard").unwrap().call(&[])?;
		if (*entry).ref_count != before {
			return Err(runtime!("test_hooks: a discarded hook result kept its reference"));
		}
	}

	Ok(Value::from(true))
}
