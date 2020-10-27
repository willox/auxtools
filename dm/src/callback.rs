use crate::hook;
use crate::raw_types;
use crate::raw_types::values::IntoRawValue;
use crate::runtime;
use crate::value::Value;
use std::sync::Mutex;

use crate as dm;

use lazy_static::lazy_static;

lazy_static! {
	static ref READY_CALLBACKS: Mutex<Vec<FinishedCallback>> = Mutex::new(Vec::new());
}

/// # Callbacks
///
/// Allows you to pass callbacks from dm code. Invoking them is thread safe.
/// Use these as a building block for operations that may take a long time, and
/// as such the usual call()() or hook are unsuitable.
///
/// Callbacks may be called multiple times.
///
/// # Examples
///
/// ```ignore
/// fn run_callback_from_thread(cb: Value) {
///		let cb = Callback::new(cb)?;
/// 	thread::spawn(move || {
///			let result = /* e.g some networking stuff... */
/// 		cb.invoke(&[Value::from(result)]);
/// 		// The callback will execute in the main thread in the near future.
/// 	});
///
/// 	Ok(Value::null())
///}
/// ```
///
///
#[derive(Clone)]
pub struct Callback {
	dm_callback: crate::raw_types::values::Value,
}

pub struct FinishedCallback {
	callback: Callback,
	closure: Box<dyn Fn() -> Vec<Value> + Send + Sync>,
}

impl Callback {
	pub fn new<V: AsRef<Value>>(cb: V) -> Result<Self, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback

		let cb = cb.as_ref();

		// TODO: LEAKS LEAKS LEAKS
		unsafe {
			raw_types::funcs::inc_ref_count(cb.value);
		}

		Ok(Self {
			dm_callback: unsafe { cb.as_ref().into_raw_value() },
		})
	}

	/// Queues this callback for execution on next timer tick.
	pub fn invoke<F>(&self, closure: F)
	where
		F: 'static,
		F: Fn() -> Vec<Value> + Send + Sync,
	{
		let finished = FinishedCallback {
			callback: self.clone(),
			closure: Box::new(closure),
		};
		READY_CALLBACKS.lock().unwrap().push(finished);
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	let mut cbs = READY_CALLBACKS.lock().unwrap();

	// We don't care if a callback runtimes.
	#[allow(unused_must_use)]
	for cb in cbs.iter() {
		unsafe {
			let args = (cb.closure)();
			Value::from_raw(cb.callback.dm_callback).call("Invoke", &args[..]);
		};
	}
	cbs.clear();

	Ok(Value::null())
}
