use super::hooks;
use super::raw_types::procs::{ProcEntry, ProcRef};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

use super::GLOBAL_STATE;

#[derive(Clone)]
pub struct Proc {
    pub id: ProcRef,
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
        let proc_entry = unsafe { (GLOBAL_STATE.get().unwrap().get_proc_array_entry)(ProcRef(i)) };
        if proc_entry.is_null() {
            break;
        }
        let proc_name: String = unsafe {
            CStr::from_ptr(
                (*(GLOBAL_STATE.get().unwrap().get_string_table_entry)((*proc_entry).path.0)).data,
            )
            .to_string_lossy()
            .into()
        };
        let proc = Proc {
            id: ProcRef(i),
            entry: proc_entry,
            path: proc_name.clone(),
        };

        PROCS_BY_NAME.with(|h| {
            h.borrow_mut().insert(proc_name, vec![proc]);
        });

        i += 1;
    }
}

pub fn get_proc<S: Into<String> + Eq>(path: S) -> Option<Proc> {
    let s: String = path.into();
    Some(PROCS_BY_NAME.with(|h| h.borrow().get(&s).unwrap()[0].clone()))
}
