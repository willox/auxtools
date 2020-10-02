mod raw_types;
mod byond_ffi;

use sigscan::Scanner;
use std::cell::RefCell;

thread_local!(static GlobalState: RefCell<Option<State>> = RefCell::new(None));


struct State {
    string_table: *mut raw_types::strings::StringTable,
    get_proc_array_entry: raw_types::funcs::GetProcArrayEntry,
}

pub struct Proc {
    internal: *mut raw_types::procs::Proc,
}

pub struct DMContext {
    state: &State,
}

impl DMContext {
    fn GetProc(index: u32) -> Option<Proc> {
        let ptr = (state.get_proc_array_entry)(raw_types::procs::ProcRef(index));

        if ptr.is_null() {
            return None
        }

        Some(Proc {
            internal: ptr,
        })
    }
}

byond_ffi_fn! { auxtools_init(input) {
    // Already initialized. Just succeed?
    if GlobalState.with(|state| { 
        state.borrow().is_some()
    }) {
        return Some("SUCCESS");
    }

    let byondcore = match Scanner::for_module("byondcore.dll") {
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

    GlobalState.with(|state| { 
        *state.borrow_mut() = Some(State {
            string_table: string_table,
            get_proc_array_entry: get_proc_array_entry,
        });
    });

    Some("SUCCESS")
} }

#[cfg(test)]
mod tests {
    #[test]
    fn test() {

    }
}