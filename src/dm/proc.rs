use super::hooks;
use super::raw_types::procs::{ProcEntry, ProcId};
use super::string::StringRef;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use super::GLOBAL_STATE;

#[derive(Clone)]
pub struct Proc {
    pub id: ProcId,
    pub entry: *mut ProcEntry,
    pub path: String,
}

impl Proc {
    pub fn hook(&self, func: hooks::ProcHook) {
        hooks::hook(self, func);
    }
}

thread_local!(static PROCS_BY_NAME: RefCell<HashMap<String, Vec<Proc>>> = RefCell::new(HashMap::new()));

pub fn populate_procs() {
    let mut i: u32 = 0;
    loop {
        let proc_entry = unsafe { (GLOBAL_STATE.get().unwrap().get_proc_array_entry)(ProcId(i)) };
        if proc_entry.is_null() {
            break;
        }
        let proc_name: String = unsafe { StringRef::from_id((*proc_entry).path.0).into() };
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
    let s: String = path.into();
    Some(PROCS_BY_NAME.with(|h| {
        h.borrow()
            .get(&s)
            .unwrap()
            .get(override_id)
            .unwrap()
            .clone()
    }))
}

pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
    get_proc_override(path, 0)
}
