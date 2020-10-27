use crate::hook;
use crate::runtime;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate as dm;

use lazy_static::lazy_static;

lazy_static! {
	static ref READY_CALLBACKS: Mutex<Vec<FinishedCallback>> = Mutex::new(Vec::new());
	static ref DROPPED_CALLBACKS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
}

static mut NEXT_CALLBACK_ID: usize = 0;
thread_local! {
	static CALLBACKS: RefCell<HashMap<usize, Callback>> = RefCell::new(HashMap::new());
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
	dm_callback: Value,
}

pub struct FinishedCallback {
	id: usize,
	closure: Box<dyn Fn() -> Vec<Value> + Send + Sync>,
}

pub struct CallbackHandle {
	id: usize,
}

impl Drop for CallbackHandle {
	fn drop(&mut self) {
		DROPPED_CALLBACKS.lock().unwrap().push(self.id);
	}
}

impl CallbackHandle {
	/// Queues this callback for execution on next timer tick.
	pub fn invoke<F>(&self, closure: F)
	where
		F: 'static,
		F: Fn() -> Vec<Value> + Send + Sync,
	{
		let finished = FinishedCallback {
			id: self.id,
			closure: Box::new(closure),
		};
		READY_CALLBACKS.lock().unwrap().push(finished);
	}
}

impl Callback {
	pub fn new<V: AsRef<Value>>(cb: V) -> Result<Arc<CallbackHandle>, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback

		let cb = cb.as_ref();

		// TODO: LEAKS LEAKS LEAKS

		CALLBACKS.with(|h| {
			h.borrow_mut().insert(
				unsafe { NEXT_CALLBACK_ID },
				Self {
					dm_callback: cb.as_ref().clone(),
				},
			)
		});

		let handle = Arc::new(CallbackHandle {
			id: unsafe { NEXT_CALLBACK_ID },
		});

		unsafe { NEXT_CALLBACK_ID += 1 };

		Ok(handle)
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	let mut ready_cbs = READY_CALLBACKS.lock().unwrap();
	let mut dropped_cbs = DROPPED_CALLBACKS.lock().unwrap();
	CALLBACKS.with(|h| {
		let mut cbs = h.borrow_mut();

		// We don't care if a callback runtimes.
		#[allow(unused_must_use)]
		for cb in ready_cbs.iter() {
			let args = (cb.closure)();
			let cb_val = cbs.get(&cb.id).unwrap();
			cb_val.dm_callback.call("Invoke", &args[..]);
		}
		ready_cbs.clear();

		for dropped in dropped_cbs.iter() {
			cbs.remove(dropped);
		}
		dropped_cbs.clear();
	});
	Ok(Value::null())
}
