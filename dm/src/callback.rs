use crate::hook;
use crate::raw_types::values::IntoRawValue;
use crate::runtime;
use crate::value::Value;
use std::sync::Mutex;

use crate as dm;

use lazy_static::lazy_static;

lazy_static! {
	static ref READY_CALLBACKS: Mutex<Vec<Callback>> = Mutex::new(Vec::new());
}

/// # Callbacks
///
/// Allows you to pass callbacks from dm code. Invoking them is thread safe.
/// Use these as a building block for operations that may take a long time, and
/// as such the usual call()() or hook are unsuitable.
///
///	Callbacks may be called multiple times.
///
/// # Examples
///
/// ```ignore
/// fn run_callback_from_thread(cb: Value) {
/// 	let cb = Callback::new(cb)?;
/// 	thread::spawn(move || {
///			let result = /* e.g some networking stuff... */
/// 		cb.invoke(&[Value::from(result)]);
/// 		// The callback will execute in the main thread in the near future.
/// 	});
///
/// 	Ok(Value::null())
/// }
/// ```
///
///
#[derive(Clone)]
pub struct Callback {
	dm_callback: crate::raw_types::values::Value,
	args: Box<dyn Fn() -> Vec<Value> + Send>,
}

impl Callback {
	pub fn new<V: AsRef<Value>>(cb: V) -> Result<Self, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback
		Ok(Self {
			dm_callback: unsafe { cb.as_ref().into_raw_value() },
			args: Vec::new(),
		})
	}

	/// Queues this callback for execution on next timer tick.
	pub fn invoke<V: AsRef<Value>>(&self, args: &[V]) {
		let mut perfect_reflection = self.clone();
		perfect_reflection.args = args
			.iter()
			.map(|a| unsafe { a.as_ref().into_raw_value() })
			.collect();
		READY_CALLBACKS.lock().unwrap().push(perfect_reflection);
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	let mut cbs = READY_CALLBACKS.lock().unwrap();

	// We don't care if a callback runtimes.
	#[allow(unused_must_use)]
	for cb in cbs.iter() {
		unsafe {
			Value::from_raw_owned(cb.dm_callback).call(
				"Invoke",
				cb.args
					.iter()
					.map(|v| Value::from_raw_owned(*v))
					.collect::<Vec<Value>>()
					.as_slice(),
			)
		};
	}
	cbs.clear();

	Ok(Value::null())
}
