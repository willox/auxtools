use super::raw_types;
use super::raw_types::procs::{ProcEntry, ProcId};
use super::string::StringRef;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Once;

/// Used to manipulate procs.
///
/// ### Override ID
///
/// Procs in DM can be defined multiple times.
///
/// ```
/// /proc/hello() // Override #0 or base proc
///		world << "Hello"
///
///	/hello() // Override #1
///		..() // Calls override #0
///		world << "World"
///
///	/hello() // Override #2
///		..() // Calls override #1
///		world << "!!!"
///	```
///
///	To get the nth override, use [get_proc_override]: `let hello = get_proc_override("/proc/hello", n).unwrap()`
/// [get_proc] retrieves the base proc.
///
///

#[derive(Clone)]
pub struct Proc {
	pub id: ProcId,
	pub entry: *mut ProcEntry,
	pub path: String,
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

/// Retrieves the specified override of a proc.
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
