use super::raw_types;
use super::raw_types::procs::{ProcEntry, ProcId};
use super::runtime;
use super::string::StringRef;
use super::value::Value;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Once;

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

	/// Calls a global proc with the given arguments.
	///
	/// # Examples
	///
	/// This function is equivalent to `return do_explode(3)` in DM.
	/// ```ignore
	/// #[hook("/proc/my_proc")]
	/// fn my_proc_hook() -> DMResult {
	/// 	let proc = Proc::find("/proc/do_explode").unwrap();
	/// 	proc.call(&[&Value::from(3.0)])
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

thread_local!(static PROCS_BY_NAME: RefCell<HashMap<String, Vec<Proc>>> = RefCell::new(HashMap::new()));

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let mut proc_entry: *mut ProcEntry = std::ptr::null_mut();
		unsafe {
			assert_eq!(
				raw_types::funcs::get_proc_array_entry(&mut proc_entry, ProcId(i)),
				1
			);
		}
		if proc_entry.is_null() {
			break;
		}
		let proc_name = strip_path(unsafe { StringRef::from_id((*proc_entry).path.0).into() });
		let proc = Proc {
			id: ProcId(i),
			entry: proc_entry,
			path: proc_name.clone(),
		};

		PROCS_BY_NAME.with(|h| {
			match h.borrow_mut().entry(proc_name) {
				Entry::Occupied(o) => {
					o.into_mut().push(proc);
				}
				Entry::Vacant(v) => {
					v.insert(vec![proc]);
				}
			};
		});

		i += 1;
	}
}

static LOAD_PROCS: Once = Once::new();

pub fn get_proc_override<S: Into<String>>(path: S, override_id: usize) -> Option<Proc> {
	LOAD_PROCS.call_once(|| {
		populate_procs();
	});
	let s = strip_path(path.into());
	PROCS_BY_NAME.with(|h| match h.borrow().get(&s)?.get(override_id) {
		Some(p) => Some(p.clone()),
		None => None,
	})
}

/// Retrieves the 0th override of a proc.
pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
