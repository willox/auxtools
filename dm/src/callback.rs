use crate::hook;
use crate::runtime;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;

use crate as dm;
extern crate flume;
extern crate lazy_static;
use lazy_static::lazy_static;

enum CallbackMessage {
	Invoke(CallbackInvocation),
	Drop(CallbackId),
}

static mut NEXT_CALLBACK_ID: CallbackId = CallbackId { 0: 0 };
thread_local! {
	static CALLBACKS: RefCell<HashMap<CallbackId, Callback>> = RefCell::new(HashMap::new());
}

lazy_static! {
	static ref INVOCATION_CHANNEL: (
		flume::Sender<CallbackMessage>,
		flume::Receiver<CallbackMessage>
	) = flume::unbounded();
	static ref INVOCATION_CHANNEL_BACKGROUND: (
		flume::Sender<CallbackMessage>,
		flume::Receiver<CallbackMessage>
	) = flume::unbounded();
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
	invoke_channel: flume::Sender<CallbackMessage>,
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
	fn new_impl<V: AsRef<Value>>(
		cb: V,
		channel: flume::Sender<CallbackMessage>,
	) -> Result<CallbackHandle, runtime::Runtime> {
		// TODO: Verify this is indeed a /datum/callback

		let cb = cb.as_ref().clone();

		CALLBACKS.with(|h| {
			h.borrow_mut()
				.insert(unsafe { NEXT_CALLBACK_ID }, Self { dm_callback: cb })
		});

		let handle = CallbackHandle {
			id: unsafe { NEXT_CALLBACK_ID },
			invoke_channel: channel, // Clone the sender part.
		};

		unsafe { NEXT_CALLBACK_ID.0 += 1 };

		Ok(handle)
	}
	/// Creates a new callback. The passed Value must be of type `/datum/callback`,
	/// otherwise a [runtime::Runtime] is produced. This should be run next tick.
	pub fn new<V: AsRef<Value>>(cb: V) -> Result<CallbackHandle, runtime::Runtime> {
		Self::new_impl(cb, INVOCATION_CHANNEL.0.clone())
	}
	/// Creates a new callback. The passed Value must be of type `/datum/callback`,
	/// otherwise a [runtime::Runtime] is produced. These should be run as soon
	/// as it wouldn't lag the game to do so.
	pub fn new_background<V: AsRef<Value>>(cb: V) -> Result<CallbackHandle, runtime::Runtime> {
		Self::new_impl(cb, INVOCATION_CHANNEL_BACKGROUND.0.clone())
	}
}

#[hook("/proc/_process_callbacks")]
fn process_callbacks() {
	CALLBACKS.with(|h| {
		let mut cbs = h.borrow_mut();

		for msg in INVOCATION_CHANNEL.1.try_iter() {
			#[allow(unused_must_use)]
			match msg {
				CallbackMessage::Invoke(cb) => {
					let args = (cb.closure)();
					let cb_val = cbs.get(&cb.id).unwrap();
					cb_val.dm_callback.call("Invoke", &args[..]);
				}
				CallbackMessage::Drop(id) => {
					cbs.remove(&id).unwrap();
				}
			}
		}
	});
	Ok(Value::null())
}

#[hook("/proc/_process_callbacks_background")]
fn process_callbacks_background() {
	CALLBACKS.with(|h| -> crate::DMResult {
		let mut cbs = h.borrow_mut();
		let start_time = coarsetime::Instant::now();
		let arg_limit = args
			.get(0)
			.ok_or_else(|| runtime!("Background callback process expects a time limit."))?
			.as_number()?;
		let time_limit = coarsetime::Duration::from_millis(arg_limit as u64);
		for msg in INVOCATION_CHANNEL_BACKGROUND.1.try_iter() {
			#[allow(unused_must_use)]
			match msg {
				CallbackMessage::Invoke(cb) => {
					let args = (cb.closure)();
					let cb_val = cbs.get(&cb.id).unwrap();
					cb_val.dm_callback.call("Invoke", &args[..]);
				}
				CallbackMessage::Drop(id) => {
					cbs.remove(&id).unwrap();
				}
			}
			if start_time.elapsed() > time_limit {
				break;
			}
		}
		Ok(Value::from(start_time.elapsed_since_recent() > time_limit))
	})
}
