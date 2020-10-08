use super::raw_types;
use super::raw_types::procs::{ProcEntry, ProcId};
use super::string::StringRef;
use super::value::Value;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::raw_types::values::IntoRawValue;

use super::GLOBAL_STATE;

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

pub fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let proc_entry = unsafe { (GLOBAL_STATE.get().unwrap().get_proc_array_entry)(ProcId(i)) };
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

pub fn get_proc_override<S: Into<String>>(path: S, override_id: usize) -> Option<Proc> {
	let s = strip_path(path.into());
	PROCS_BY_NAME.with(|h| match h.borrow().get(&s)?.get(override_id) {
		Some(p) => Some(p.clone()),
		None => None,
	})
}

pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
