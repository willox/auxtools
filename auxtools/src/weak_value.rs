use crate::byond_string;
use crate::raw_types;
use crate::DMResult;
use crate::Value;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

fn get_next_id() -> f32 {
	// This can (should) be only called from the main thread but we need to shut Rust up.
	static NEXT_WEAKREF_ID: AtomicU32 = AtomicU32::new(1);
	let id = NEXT_WEAKREF_ID.fetch_add(1, Relaxed);
	id as f32
}

pub struct WeakValue {
	inner: raw_types::values::Value,
	id: f32,
}

impl WeakValue {
	pub fn new(val: &Value) -> DMResult<Self> {
		let id = get_next_id();
		val.set(byond_string!("__auxtools_weakref_id"), Value::from(id))?;
		Ok(Self { inner: val.raw, id })
	}

	pub fn upgrade(&self) -> Option<Value> {
		let real_val = unsafe { Value::from_raw(self.inner) };
		let id = real_val
			.get_number(byond_string!("__auxtools_weakref_id"))
			.ok()?;

		if self.id != id {
			return None;
		}

		Some(real_val)
	}

	pub fn upgrade_or_null(&self) -> Value {
		match self.upgrade() {
			Some(v) => v,
			None => Value::null(),
		}
	}
}
