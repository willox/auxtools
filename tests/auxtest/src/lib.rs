use auxtools::*;
use std::io::Write;

mod ffi_tests;
mod lists;
mod procs;
mod strings;
mod value_from;
mod variables;
mod weak;

#[hook("/proc/auxtest_hook_basic")]
fn hook_basic() {
	Ok(Value::from(true))
}

#[hook("/proc/auxtest_runtime_globals")]
fn runtime_globals() {
	unsafe {
		if raw_types::funcs::CURRENT_EXECUTION_CONTEXT.is_null() {
			return Err(runtime!("runtime_globals: CURRENT_EXECUTION_CONTEXT is null"));
		}

		if (*raw_types::funcs::CURRENT_EXECUTION_CONTEXT).is_null() {
			return Err(runtime!("runtime_globals: current execution context is null"));
		}

		if raw_types::funcs::SUSPENDED_PROCS.is_null() {
			return Err(runtime!("runtime_globals: SUSPENDED_PROCS is null"));
		}

		if raw_types::funcs::SUSPENDED_PROCS_BUFFER.is_null() {
			return Err(runtime!("runtime_globals: SUSPENDED_PROCS_BUFFER is null"));
		}
	}

	Ok(Value::from(true))
}

#[hook("/proc/auxtest_inc_counter")]
fn inc_counter() {
	static mut COUNTER: u32 = 0;

	Ok(Value::from(unsafe {
		COUNTER += 1;
		COUNTER
	}))
}

#[hook("/proc/auxtest_out")]
fn out(msg: Value) {
	let msg = msg.as_string()?;
	eprintln!("\n{}", msg);
	if let Some(path) = std::env::var_os("AUXTEST_OUT") {
		if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
			let _ = writeln!(file, "{}", msg);
		}
	}
	Ok(Value::NULL)
}
