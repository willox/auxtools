use std::sync::atomic::{AtomicBool, Ordering};

use auxtools::*;

mod hooks;
mod lists;
mod procs;
mod strings;
mod value_from;
mod weak;

#[hook("/proc/auxtest_inc_counter")]
fn inc_counter() {
	static mut COUNTER: u32 = 0;

	Ok(Value::from(unsafe {
		COUNTER += 1;
		COUNTER
	}))
}

// Not a hook, so a failure still gets reported when hooks are what broke
byond_ffi_fn! { auxtest_out(msg) {
	eprintln!("\n{}", msg);
	Some(String::new())
} }

static FAIL_NEXT_INIT: AtomicBool = AtomicBool::new(false);

#[hook("/proc/auxtest_fail_next_init")]
fn fail_next_init() {
	FAIL_NEXT_INIT.store(true, Ordering::Relaxed);
	Ok(Value::NULL)
}

#[init(partial)]
fn fail_init_on_request() -> Result<(), String> {
	if FAIL_NEXT_INIT.load(Ordering::Relaxed) {
		return Err("auxtest asked for this init to fail".to_owned());
	}
	Ok(())
}
