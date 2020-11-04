use super::disassembler;
use super::raw_types;
use super::raw_types::misc;
use super::raw_types::misc::AsMiscId;
use super::raw_types::procs::{ProcEntry, ProcId};
use super::runtime;
use super::string::StringRef;
use super::value::Value;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use std::cell::RefCell;
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
	pub id: ProcId,
	pub entry: *mut ProcEntry,
	pub path: String,
}

impl<'a> Proc {
	/// Finds the first proc with the given path
	pub fn find<S: Into<String>>(path: S) -> Option<Self> {
		get_proc(path)
	}

	/// Finds the n'th re-defined proc with the given path
	pub fn find_override<S: Into<String>>(path: S, override_id: u32) -> Option<Self> {
		get_proc_override(path, override_id)
	}

	pub fn from_id(id: ProcId) -> Option<Self> {
		let mut proc_entry: *mut ProcEntry = std::ptr::null_mut();
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
		let mut misc: *mut misc::Misc = std::ptr::null_mut();
		unsafe {
			assert_eq!(
				raw_types::funcs::get_misc_by_id(&mut misc, (*self.entry).parameters.as_misc_id()),
				1
			);

			let count = (*misc).parameters.count();
			let data = (*misc).parameters.data;
			(0..count)
				.map(|i| StringRef::from_variable_id((*data.add(i as usize)).name))
				.collect()
		}
	}

	pub fn local_names(&self) -> Vec<StringRef> {
		let mut misc: *mut misc::Misc = std::ptr::null_mut();
		unsafe {
			assert_eq!(
				raw_types::funcs::get_misc_by_id(&mut misc, (*self.entry).locals.as_misc_id()),
				1
			);

			let count = (*misc).locals.count;
			let names = (*misc).locals.names;
			(0..count)
				.map(|i| StringRef::from_variable_id(*names.add(i as usize)))
				.collect()
		}
	}

	pub unsafe fn bytecode(&self) -> (*mut u32, usize) {
		let mut misc: *mut misc::Misc = std::ptr::null_mut();
		assert_eq!(
			raw_types::funcs::get_misc_by_id(&mut misc, (*self.entry).bytecode.as_misc_id()),
			1
		);

		((*misc).bytecode.bytecode, (*misc).bytecode.count as usize)
	}

	pub fn disassemble(
		&self,
	) -> (
		Vec<(u32, u32, disassembler::Instruction)>,
		Option<disassembler::DisassembleError>,
	) {
		disassembler::disassemble(self)
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
				raw_types::funcs::inc_ref_count(v.value);
			}

			let args: Vec<_> = args.iter().map(|e| e.value).collect();

			if raw_types::funcs::call_proc_by_id(
				&mut ret,
				Value::null().value,
				0,
				self.id,
				0,
				Value::null().value,
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
}

impl fmt::Debug for Proc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = unsafe { (*self.entry).path };
		write!(f, "Proc({:?})", unsafe { StringRef::from_id(path) })
	}
}

thread_local!(static PROCS_BY_NAME: RefCell<DashMap<String, Vec<Proc>>> = RefCell::new(DashMap::new()));

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let proc = Proc::from_id(ProcId(i));
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();

		PROCS_BY_NAME.with(|h| {
			match h.borrow_mut().entry(proc.path.clone()) {
				Entry::Occupied(mut o) => {
					o.get_mut().push(proc);
				}
				Entry::Vacant(v) => {
					v.insert(vec![proc]);
				}
			};
		});

		i += 1;
	}
}

pub fn clear_procs() {
	PROCS_BY_NAME.with(|h| h.borrow_mut().clear())
}

pub fn get_proc_override<S: Into<String>>(path: S, override_id: u32) -> Option<Proc> {
	let s = strip_path(path.into());
	PROCS_BY_NAME.with(|h| match h.borrow().get(&s)?.get(override_id as usize) {
		Some(p) => Some(p.clone()),
		None => None,
	})
}

/// Retrieves the 0th override of a proc.
pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
