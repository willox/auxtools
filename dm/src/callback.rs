use crate::hook;
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
#[derive(Clone, Debug)]
pub struct Callback {
	// dm_callback: Value,
	// args: Vec<Value>,
}

impl Callback {
	fn new<V: AsRef<Value>>(cb: V) -> Result<Self, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback
		Ok(Self {
			// dm_callback: cb.as_ref().clone(),
			// args: Vec::new(),
		})
	}

	/// Queues this callback for execution on next timer tick.
	fn invoke<V: AsRef<Value>>(&self, args: &[V]) {
		let mut perfect_reflection = self.clone();
		// perfect_reflection.args = args.iter().map(|a| a.as_ref().clone()).collect();
		READY_CALLBACKS.lock().unwrap().push(perfect_reflection);
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	let mut cbs = READY_CALLBACKS.lock().unwrap();

	// We don't care if a callback runtimes.
	#[allow(unused_must_use)]
	for cb in cbs.iter() {
		// cb.dm_callback.call("Invoke", cb.args.as_slice());
	}
	cbs.clear();

	Ok(Value::null())
}
