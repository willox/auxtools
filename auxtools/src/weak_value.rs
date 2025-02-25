use crate::{byond_string, raw_types, DMResult, Value};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

fn get_next_id() -> f32 {
	// This can (should) be only called from the main thread but we need to shut
	// Rust up.
	static NEXT_WEAKREF_ID: AtomicU32 = AtomicU32::new(1);
	let id = NEXT_WEAKREF_ID.fetch_add(1, Relaxed);
	id as f32
}

/// A weak reference to some datum or atom in the game.
///
/// Normal [`Value`]s are not safe to move between threads.
/// Using methods like [`Value::set`] or [`Value::call`] can at best cause
/// undefined behavior, at worst crash the server.
///
/// A way to bypass that limitation is to store a raw value
/// and use [`Value::from_raw`] on the main thread to actually work
/// with it. However, if that [`Value`] is deleted, your stored value
/// will point to another datum or to simply nothing.
///
/// This struct serves to solve the latter problem. You can use
/// [`Value::as_weak`] to create a weak reference to it.
/// The reference can be stored in global structures or passed to
/// other threads. You can then return it to the main thread as needed,
/// and call [`WeakValue::upgrade`] to turn it back into a real [`Value`].
/// If the datum pointed to was deleted in the meantime, `upgrade` will
/// return None, otherwise you get your datum back.
///
/// However, this struct is not entirely thread safe, since you can
/// [`WeakValue::upgrade`] on another thread and invoke undefined behavior with
/// the resulting [`Value`]. So, don't do that.
///
/// Using this struct requires all datums to have a `__auxtools_weakref_id`
/// variable.
///
/// # Example
/// ```ignore
/// let weakref = thing.as_weak()?;
/// callbacks.set(some_id, weakref);
///
/// ... some proc calls later ...
///
/// let weakref = callbacks.get(some_id);
/// if let Some(thing) = weakref.upgrade() {
///     thing.call("callback", &[])?;
/// }
/// ```
#[derive(Copy, Clone)]
pub struct WeakValue {
	inner: raw_types::values::Value,
	id: f32
}

impl WeakValue {
	/// Creates a weak reference to the given datum.
	pub(crate) fn new(val: &Value) -> DMResult<Self> {
		if let Ok(id) = val.get_number(byond_string!("__auxtools_weakref_id")) {
			return Ok(Self { inner: val.raw, id });
		}

		let id = get_next_id();
		val.set(byond_string!("__auxtools_weakref_id"), Value::from(id))?;
		Ok(Self { inner: val.raw, id })
	}

	/// Converts the stored raw value to a full fledged [`Value`]
	/// and checks if it has been deleted in the meantime.
	pub fn upgrade(&self) -> Option<Value> {
		let real_val = unsafe { Value::from_raw(self.inner) };
		let id = real_val.get_number(byond_string!("__auxtools_weakref_id")).ok()?;

		if self.id != id {
			return None;
		}

		Some(real_val)
	}

	/// Same as [`WeakValue::upgrade`] but returns a null if the datum was
	/// deleted, so you can pass it straight into DM.
	pub fn upgrade_or_null(&self) -> Value {
		match self.upgrade() {
			Some(v) => v,
			None => Value::NULL
		}
	}
}

impl Value {
	/// Creates a [`WeakValue`] referencing this datum.
	pub fn as_weak(&self) -> DMResult<WeakValue> {
		WeakValue::new(self)
	}
}
