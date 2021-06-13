use crate::*;
use fxhash::FxBuildHasher;
use std::collections::{hash_map::Entry, HashMap};
use std::fmt;

//
// ### A note on Override IDs
//
// Procs in DM can be defined multiple times.
//
// ```
// /proc/hello() // Override #0 or base proc
//		world << "Hello"
//
//	/hello() // Override #1
//		..() // Calls override #0
//		world << "World"
//
//	/hello() // Override #2
//		..() // Calls override #1
//		world << "!!!"
//	```
//
//	To get the nth override, use [get_proc_override]: `let hello = get_proc_override("/proc/hello", n).unwrap()`
// [get_proc] retrieves the base proc.
//
//

/// Used to hook and call procs.
#[derive(Clone)]
pub struct Proc {
	pub id: raw_types::procs::ProcId,
	pub entry: *mut raw_types::procs::ProcEntry,
	pub path: String,
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
			assert_eq!(
				raw_types::funcs::get_proc_array_entry(&mut proc_entry, id),
				1
			);
		}
		if proc_entry.is_null() {
			return None;
		}
		let proc_name = strip_path(unsafe { StringRef::from_id((*proc_entry).path).into() });
		Some(Proc {
			id: id,
			entry: proc_entry,
			path: proc_name.clone(),
		})
	}

	pub fn parameter_names(&self) -> Vec<StringRef> {
		unsafe {
			let (data, count) = raw_types::misc::get_parameters((*self.entry).parameters);
			(0..count)
				.map(|i| StringRef::from_variable_id((*data.add(i as usize)).name))
				.collect()
		}
	}

	pub fn local_names(&self) -> Vec<StringRef> {
		unsafe {
			let (names, count) = raw_types::misc::get_locals((*self.entry).locals);
			(0..count)
				.map(|i| StringRef::from_variable_id(*names.add(i as usize)))
				.collect()
		}
	}

	pub fn set_bytecode(&self, bytecode: Vec<u32>) {
		crate::bytecode_manager::set_bytecode(self, bytecode);
	}

	pub unsafe fn bytecode_mut_ptr(&self) -> (*mut u32, u16) {
		raw_types::misc::get_bytecode((*self.entry).bytecode)
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
			data: raw_types::values::ValueData { id: 0 },
		};

		unsafe {
			// Increment ref-count of args permenently before passing them on
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			let args: Vec<_> = args.iter().map(|e| e.raw).collect();

			if raw_types::funcs::call_proc_by_id(
				&mut ret,
				Value::null().raw,
				0,
				self.id,
				0,
				Value::null().raw,
				args.as_ptr(),
				args.len(),
				0,
				0,
			) == 1
			{
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	pub fn override_id(&self) -> u32 {
		unsafe { PROC_OVERRIDE_IDS.as_ref() }
			.and_then(|override_ids| override_ids.get(&self.id))
			.map_or(0, |id| *id)
	}
}

impl fmt::Debug for Proc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = unsafe { (*self.entry).path };
		write!(f, "Proc({:?})", unsafe { StringRef::from_id(path) })
	}
}

static mut PROCS_BY_NAME: Option<HashMap<String, Vec<Proc>, FxBuildHasher>> = None;
static mut PROC_OVERRIDE_IDS: Option<HashMap<raw_types::procs::ProcId, u32, FxBuildHasher>> = None;

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn populate_procs() {
	let mut i: u32 = 0;
	let override_ids =
		unsafe { PROC_OVERRIDE_IDS.get_or_insert_with(|| HashMap::with_hasher(FxBuildHasher::default())) };
	let h = unsafe { PROCS_BY_NAME .get_or_insert_with(|| HashMap::with_hasher(FxBuildHasher::default())) };
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(i));
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();
		 // "why not use the fancy methods and_modify and or_insert_with"?
		 // closure moving non-Copy `proc` is why
		match h.entry(proc.path.clone()) {
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

		i += 1;
	}
}

pub fn clear_procs() {
	unsafe {
		PROCS_BY_NAME = None;
		PROC_OVERRIDE_IDS = None;
	}
}

pub fn get_proc_override<S: Into<String>>(path: S, override_id: u32) -> Option<Proc> {
	let s = strip_path(path.into());
	unsafe { PROCS_BY_NAME.as_ref() }
		.and_then(|h| h.get(&s)?.get(override_id as usize))
		.map(|p| p.clone())
}

/// Retrieves the 0th override of a proc.
pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
