use std::{
	cell::RefCell,
	collections::{hash_map::Entry, HashMap},
	fmt
};

use ahash::RandomState;
use fxhash::FxHashMap;

use crate::*;

// ### A note on Override IDs
//
// Procs in DM can be defined multiple times.
//
// ```
// /proc/hello() // Override #0 or base proc
// 		world << "Hello"
//
// 	/hello() // Override #1
// 		..() // Calls override #0
// 		world << "World"
//
// 	/hello() // Override #2
// 		..() // Calls override #1
// 		world << "!!!"
// 	```
//
// 	To get the nth override, use [get_proc_override]: `let hello = get_proc_override("/proc/hello", n).unwrap()`
// [get_proc] retrieves the base proc.

/// Used to hook and call procs.
#[derive(Clone)]
pub struct Proc {
	pub id: raw_types::procs::ProcId,
	pub entry: *mut raw_types::procs::ProcEntry,
	pub path: String
}

impl Proc {
	/// Finds the first proc with the given path
	pub fn find<S: Into<String>>(path: S) -> Option<Self> {
		get_proc(path)
	}

	/// Finds the n'th re-defined proc with the given path
	pub fn find_override<S: Into<String>>(path: S, override_id: u32) -> Option<Self> {
		get_proc_override(path, override_id)
	}

	pub fn from_id(id: raw_types::procs::ProcId) -> Option<Self> {
		let mut proc_entry: *mut raw_types::procs::ProcEntry = std::ptr::null_mut();
		unsafe {
			assert_eq!(raw_types::funcs::get_proc_array_entry(&mut proc_entry, id), 1);
		}
		if proc_entry.is_null() {
			return None;
		}
		let proc_name = strip_path(unsafe { StringRef::from_id((*proc_entry).path).into() });
		Some(Proc {
			id,
			entry: proc_entry,
			path: proc_name.clone()
		})
	}

	pub unsafe fn file_name(&self) -> Option<StringRef> {
		let bytecode = self.bytecode();
		if bytecode.len() < 2 || bytecode[0] != 0x84 {
			return None;
		}

		let file_id = raw_types::strings::StringId(bytecode[0x01]);
		if !file_id.valid() {
			return None;
		}

		Some(StringRef::from_id(file_id))
	}

	pub fn parameter_names(&self) -> Vec<StringRef> {
		unsafe {
			let (data, count) = raw_types::misc::get_parameters((*self.entry).metadata.get_parameters());
			(0..count).map(|i| StringRef::from_variable_id((*data.add(i)).name)).collect()
		}
	}

	pub fn local_names(&self) -> Vec<StringRef> {
		unsafe {
			let (names, count) = raw_types::misc::get_locals((*self.entry).metadata.get_locals());
			(0..count).map(|i| StringRef::from_variable_id(*names.add(i))).collect()
		}
	}

	pub fn set_bytecode(&self, bytecode: Vec<u32>) {
		crate::bytecode_manager::set_bytecode(self, bytecode);
	}

	pub unsafe fn bytecode_mut_ptr(&self) -> (*mut u32, u16) {
		raw_types::misc::get_bytecode((*self.entry).metadata.get_bytecode())
	}

	pub unsafe fn bytecode(&self) -> &[u32] {
		let (ptr, count) = self.bytecode_mut_ptr();
		std::slice::from_raw_parts(ptr, count as usize)
	}

	/// Calls a global proc with the given arguments.
	///
	/// # Examples
	///
	/// This function is equivalent to `return do_explode(3)` in DM.
	/// ```ignore
	/// #[hook("/proc/my_proc")]
	/// fn my_proc_hook() -> DMResult {
	///     let proc = Proc::find("/proc/do_explode").unwrap();
	///     proc.call(&[&Value::from(3.0)])
	/// }
	/// ```
	pub fn call(&self, args: &[&Value]) -> runtime::DMResult {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			// Increment ref-count of args permenently before passing them on
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			let args: Vec<_> = args.iter().map(|e| e.raw).collect();

			if raw_types::funcs::call_proc_by_id(&mut ret, Value::NULL.raw, 0, self.id, 0, Value::NULL.raw, args.as_ptr(), args.len(), 0, 0) == 1 {
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	pub fn override_id(&self) -> u32 {
		PROC_OVERRIDE_IDS.with(|override_ids| match override_ids.borrow().get(&self.id) {
			Some(id) => *id,
			None => 0
		})
	}
}

impl fmt::Debug for Proc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = unsafe { (*self.entry).path };
		write!(f, "Proc({:?})", unsafe { StringRef::from_id(path) })
	}
}

thread_local!(static PROCS_BY_NAME: RefCell<HashMap<String, Vec<Proc>, RandomState>> = RefCell::new(HashMap::with_hasher(RandomState::default())));
thread_local!(static PROC_OVERRIDE_IDS: RefCell<FxHashMap<raw_types::procs::ProcId, u32>> = RefCell::new(FxHashMap::default()));

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(i));
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();

		PROC_OVERRIDE_IDS.with(|override_ids| {
			let mut override_ids = override_ids.borrow_mut();

			PROCS_BY_NAME.with(|h| {
				match h.borrow_mut().entry(proc.path.clone()) {
					Entry::Occupied(mut o) => {
						let vec = o.get_mut();
						override_ids.insert(proc.id, vec.len() as u32);
						vec.push(proc);
					}
					Entry::Vacant(v) => {
						override_ids.insert(proc.id, 0);
						v.insert(vec![proc]);
					}
				};
			});
		});

		i += 1;
	}
}

pub fn clear_procs() {
	PROCS_BY_NAME.with(|h| h.borrow_mut().clear());
	PROC_OVERRIDE_IDS.with(|override_ids| override_ids.borrow_mut().clear());
}

pub fn get_proc_override<S: Into<String>>(path: S, override_id: u32) -> Option<Proc> {
	let s = strip_path(path.into());
	PROCS_BY_NAME.with(|h| h.borrow().get(&s)?.get(override_id as usize).cloned())
}

/// Retrieves the 0th override of a proc.
pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
