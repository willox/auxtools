use crate::hook;
use crate::runtime;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;

use crate as dm;
enum CallbackMessage {
	Invoke(CallbackInvocation),
	Drop(CallbackId),
}

static mut NEXT_CALLBACK_ID: CallbackId = CallbackId { 0: 0 };
thread_local! {
	static CALLBACKS: RefCell<HashMap<CallbackId, Callback>> = RefCell::new(HashMap::new());
	static INVOCATION_CHANNEL: (mpsc::Sender<CallbackMessage>, mpsc::Receiver<CallbackMessage>) = mpsc::channel();
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

struct CallbackInvocation {
	id: CallbackId,
	closure: Box<dyn Fn() -> Vec<Value> + Send + Sync>,
}

/// Invoke callbacks using this.
pub struct CallbackHandle {
	id: CallbackId,
	invoke_channel: mpsc::Sender<CallbackMessage>,
}

impl Drop for CallbackHandle {
	fn drop(&mut self) {
		self.invoke_channel
			.send(CallbackMessage::Drop(self.id))
			.unwrap();
	}
}

impl CallbackHandle {
	/// Queues this callback for execution on next timer tick.
	pub fn invoke<F>(&self, closure: F)
	where
		F: 'static,
		F: Fn() -> Vec<Value> + Send + Sync,
	{
		let finished = CallbackInvocation {
			id: self.id,
			closure: Box::new(closure),
		};
		self.invoke_channel
			.send(CallbackMessage::Invoke(finished))
			.unwrap();
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

		let handle = INVOCATION_CHANNEL.with(|c| {
			CallbackHandle {
				id: unsafe { NEXT_CALLBACK_ID },
				invoke_channel: c.0.clone(), // Clone the sender part.
			}
		});

		unsafe { NEXT_CALLBACK_ID.0 += 1 };

		Ok(handle)
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	CALLBACKS.with(|h| {
		let mut cbs = h.borrow_mut();

		INVOCATION_CHANNEL.with(|c| loop {
			match c.1.try_recv() {
				Ok(msg) => match msg {
					#[allow(unused_must_use)]
					CallbackMessage::Invoke(cb) => {
						let args = (cb.closure)();
						let cb_val = cbs.get(&cb.id).unwrap();
						cb_val.dm_callback.call("Invoke", &args[..]);
					}
					CallbackMessage::Drop(id) => {
						cbs.remove(&id).unwrap();
					}
				},
				Err(_) => break, // There are no messages for us to process
			}
		});
	});
	Ok(Value::null())
}
