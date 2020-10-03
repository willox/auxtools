mod byond_ffi;
mod raw_types;

extern crate once_cell;

use once_cell::sync::OnceCell;
use raw_types::procs::ExecutionContext;
use raw_types::values::{RawValue, Value, ValueData, ValueTag};
use std::marker::PhantomData;

static GLOBAL_STATE: OnceCell<State> = OnceCell::new();

unsafe impl Sync for State {}
unsafe impl Send for State {}

struct State {
	get_proc_array_entry: raw_types::funcs::GetProcArrayEntry,
	execution_context: *mut ExecutionContext,
	string_table: *mut raw_types::strings::StringTable,
}

pub struct Proc {
	internal: *mut raw_types::procs::Proc,
}

pub struct DMContext<'a> {
	state: &'a State,
}

impl DMContext<'_> {
	fn get_proc(&self, index: u32) -> Option<Proc> {
		unsafe {
			let ptr = (self.state.get_proc_array_entry)(raw_types::procs::ProcRef(index));

			if ptr.is_null() {
				return None;
			}

			Some(Proc { internal: ptr })
		}
	}

	fn get_global<S: Into<String>>(&self, name: S) -> Value {
		Value {
			value: RawValue {
				tag: ValueTag::Null,
				data: ValueData { number: 0.0 },
			},
			phantom: PhantomData {},
		}
	}

	fn new() -> Self {
		if let Some(state) = GLOBAL_STATE.get() {
			Self { state: &state }
		} else {
			panic!("Attempt to create context before initializing!")
		}
	}
}

byond_ffi_fn! { auxtools_init(_input) {
	// Already initialized. Just succeed?
	if GLOBAL_STATE.get().is_some() {
		return Some("SUCCESS");
	}

	let byondcore = match sigscan::Scanner::for_module("byondcore.dll") {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)")
	};

	let string_table: *mut raw_types::strings::StringTable;
	if let Some(ptr) = byondcore.find(b"\xA1????\x8B\x04?\x85\xC0\x0F\x84????\x80\x3D????\x00\x8B\x18") {
		unsafe {
			// TODO: Could be nulls
			string_table = *(ptr.offset(1) as *mut *mut raw_types::strings::StringTable);
		}
	} else {
		return Some("FAILED (Couldn't find stringtable)")
	}

	let get_proc_array_entry: raw_types::funcs::GetProcArrayEntry;
	if let Some(ptr) = byondcore.find(b"\xE8????\x8B\xC8\x8D\x45?\x6A\x01\x50\xFF\x76?\x8A\x46?\xFF\x76?\xFE\xC0") {
		unsafe {
			// TODO: Could be nulls
			let offset = *(ptr.offset(1) as *const isize);
			get_proc_array_entry = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find GetProcArrayEntry)")
	}

	assert!(GLOBAL_STATE.set(State {
		get_proc_array_entry: get_proc_array_entry,
		execution_context: std::ptr::null_mut(),
		string_table: string_table,
	}).is_ok(), "Failed to set state during init!");

	let ctx = DMContext::new();

	Some("SUCCESS")
} }

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
