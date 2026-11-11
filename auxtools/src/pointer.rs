//! Read DM pointers through the exported byondapi function.
//! This avoids signature scanning and dependence on internal pointer layouts.

use std::sync::OnceLock;

use byond_scan::{Anchor, Extract, Recipe, VersionRange};

use crate::{
	raw_types::{
		funcs,
		values::{Value as RawValue, ValueData, ValueTag}
	},
	Value
};

const READ_POINTER: Recipe = Recipe {
	name: "Byond_ReadPointer",
	versions: VersionRange { min: 1637, max: 1688 },
	anchor: Anchor::Export(c"Byond_ReadPointer"),
	hops: &[],
	extract: Extract::Entry
};

/// Resolve `Byond_ReadPointer` for the C++ wrapper, which catches runtime
/// errors raised by some builds when the target object has been deleted.
fn found() -> bool {
	static FOUND: OnceLock<bool> = OnceLock::new();
	*FOUND.get_or_init(|| match crate::find_recipe(&READ_POINTER) {
		Ok(address) => {
			unsafe { funcs::read_pointer_byond = address as *const std::ffi::c_void };
			true
		}
		Err(_) => false
	})
}

/// Whether pointers can be read in this world
pub fn readable() -> bool {
	found()
}

/// Read a pointer without retaining the returned value. Compare it or
/// convert it to [`Value`] immediately. Returns `None` for invalid pointers
/// or targets that no longer exist.
pub fn read_raw(pointer: &Value) -> Option<RawValue> {
	if pointer.raw.tag != ValueTag::Pointer {
		return None;
	}
	if !found() {
		return None;
	}
	let mut result = RawValue {
		tag: ValueTag::Null,
		data: ValueData { id: 0 }
	};
	(unsafe { funcs::read_pointer(&mut result, &pointer.raw) } == 1).then_some(result)
}

/// What a pointer names now
pub fn read(pointer: &Value) -> Option<Value> {
	read_raw(pointer).map(|raw| unsafe { Value::from_raw(raw) })
}
