mod byond_ffi;
mod raw_types;
mod value;
mod string;

extern crate once_cell;

use once_cell::sync::OnceCell;
use std::marker::PhantomData;
use std::ffi::CString;
use value::Value;
use string::StringRef;

static GLOBAL_STATE: OnceCell<State> = OnceCell::new();

unsafe impl Sync for State {}
unsafe impl Send for State {}

struct State {
	get_proc_array_entry: raw_types::funcs::GetProcArrayEntry,
	execution_context: *mut raw_types::procs::ExecutionContext,
	string_table: *mut raw_types::strings::StringTable,
	get_string_id: raw_types::funcs::GetStringId,
}

// TODO: Bit of an assumption going on here. Procs are never destroyed... right?
pub struct Proc {
	internal: *mut raw_types::procs::Proc,
}

pub struct DMContext<'a> {
	state: &'a State,
}

impl<'a> DMContext<'_> {
	fn get_proc(&self, index: u32) -> Option<Proc> {
		unsafe {
			let ptr = (self.state.get_proc_array_entry)(raw_types::procs::ProcRef(index));

			if ptr.is_null() {
				return None;
			}

			Some(Proc {
				internal: ptr,
			})
		}
	}

	fn get_global(&self, name: &str) -> Value {
		Value::null()
	}

	fn get_string(&self, string: &str) -> Option<StringRef> {
		if let Ok(string) = CString::new(string) {
			unsafe {
				let index = (self.state.get_string_id)(string.as_ptr(), true, false, true);
				let strings = (*self.state.string_table).strings;

				return Some(StringRef::new(*strings.add(index as usize)));
			}
		}
		None
	}

	fn new() -> Option<Self> {
		if let Some(state) = GLOBAL_STATE.get() {
			Some(Self { state: &state })
		} else {
			None
		}
	}
}

byond_ffi_fn! { auxtools_init(_input) {
	// Already initialized. Just succeed?
	if GLOBAL_STATE.get().is_some() {
		return Some("SUCCESS".to_owned());
	}

	let byondcore = match sigscan::Scanner::for_module("byondcore.dll") {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
	};

	let string_table: *mut raw_types::strings::StringTable;
	if let Some(ptr) = byondcore.find(b"\xA1????\x8B\x04?\x85\xC0\x0F\x84????\x80\x3D????\x00\x8B\x18") {
		unsafe {
			// TODO: Could be nulls
			string_table = *(ptr.offset(1) as *mut *mut raw_types::strings::StringTable);
		}
	} else {
		return Some("FAILED (Couldn't find stringtable)".to_owned())
	}

	let get_proc_array_entry: raw_types::funcs::GetProcArrayEntry;
	if let Some(ptr) = byondcore.find(b"\xE8????\x8B\xC8\x8D\x45?\x6A\x01\x50\xFF\x76?\x8A\x46?\xFF\x76?\xFE\xC0") {
		unsafe {
			// TODO: Could be nulls
			let offset = *(ptr.offset(1) as *const isize);
			get_proc_array_entry = std::mem::transmute(ptr.offset(5).offset(offset) as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find GetProcArrayEntry)".to_owned())
	}

	let get_string_id: raw_types::funcs::GetStringId;
		if let Some(ptr) = byondcore.find(b"\x55\x8B\xEC\x8B\x45?\x83\xEC?\x53\x56\x8B\x35") {
		unsafe {
			// TODO: Could be nulls
			get_string_id = std::mem::transmute(ptr as *const ());
		}
	} else {
		return Some("FAILED (Couldn't find GetStringId)".to_owned())
	}

	if GLOBAL_STATE.set(State {
		get_proc_array_entry: get_proc_array_entry,
		get_string_id: get_string_id,
		execution_context: std::ptr::null_mut(),
		string_table: string_table,
	}).is_err() {
		return Some("FAILED (Couldn't set state)".to_owned())
	}

	return Some("SUCCESS".to_owned())
} }

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
