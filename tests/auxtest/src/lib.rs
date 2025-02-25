use auxtools::*;

mod lists;
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

#[hook("/proc/auxtest_out")]
fn out(msg: Value) {
	eprintln!("\n{}", msg.as_string()?);
	Ok(Value::NULL)
}
