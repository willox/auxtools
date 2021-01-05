use std::time::Instant;
use crate::Value;
use crate::raw_types::funcs::send_maps;
use crate::hook;

#[hook]
fn hsendmaps() {
	let now = Instant::now();
	unsafe { send_maps() };
	Value::globals().set("internal_tick_usage", (now.duration_since(now).as_micros() / 100000) as f32);
	Value::null()
}


pub fn enable_maptick() -> bool {
	match crate::hooks::hook("/proc/send_maps", hsendmaps) {
		Ok(_) => true,
		Err(_) => false,
	}
}
