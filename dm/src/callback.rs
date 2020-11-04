use crate::hook;
use crate::runtime;
use crate::value::Value;
use dashmap::DashMap;
use std::cell::RefCell;
use std::sync::Mutex;

use crate as dm;

use lazy_static::lazy_static;

lazy_static! {
	static ref READY_CALLBACKS: Mutex<Vec<FinishedCallback>> = Mutex::new(Vec::new());
	static ref DROPPED_CALLBACKS: Mutex<Vec<CallbackId>> = Mutex::new(Vec::new());
}

static mut NEXT_CALLBACK_ID: CallbackId = CallbackId { 0: 0 };
thread_local! {
	static CALLBACKS: RefCell<DashMap<CallbackId, Callback>> = RefCell::new(DashMap::new());
}

/// # Callbacks
///
/// Allows you to pass callbacks from dm code. Invoking them is thread safe.
/// Use these as a building block for operations that may take a long time, and
/// as such the usual call()() or hook are unsuitable.
///
/// Due to the fact that Values are not thread safe, you need to wrap the returned
/// Vec in a closure. Refer to the example for details.
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
/// 		cb.invoke(move || vec![Value::from(result)]);
/// 		// The callback will execute in the main thread in the near future.
///			// As [Value] cannot be created on other threads, you have to return
///			// a closure that will produce your desired Values on the main thread.
/// 	});
///
/// 	Ok(Value::null())
///}
/// ```
#[derive(Clone)]
pub struct Callback {
	dm_callback: Value,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct CallbackId(u64);

struct FinishedCallback {
	id: CallbackId,
	closure: Box<dyn Fn() -> Vec<Value> + Send + Sync>,
}

/// Invoke callbacks using this.
pub struct CallbackHandle {
	id: CallbackId,
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
	/// Creates a new callback. The passed Value must be of type `/datum/callback`,
	/// otherwise a [runtime::Runtime] is produced.
	pub fn new<V: AsRef<Value>>(cb: V) -> Result<CallbackHandle, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback

		let cb = cb.as_ref().clone();

		CALLBACKS.with(|h| {
			h.borrow_mut()
				.insert(unsafe { NEXT_CALLBACK_ID }, Self { dm_callback: cb })
		});

		let handle = CallbackHandle {
			id: unsafe { NEXT_CALLBACK_ID },
		};

		unsafe { NEXT_CALLBACK_ID.0 += 1 };

		Ok(handle)
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	let mut ready_cbs = READY_CALLBACKS.lock().unwrap();
	let mut dropped_cbs = DROPPED_CALLBACKS.lock().unwrap();
	CALLBACKS.with(|h| {
		let cbs = h.borrow();

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
