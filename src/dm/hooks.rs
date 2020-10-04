use super::proc::Proc;
use super::raw_types;
use super::value::Value;
use super::DMContext;
use super::GLOBAL_STATE;
use detour::static_detour;
use std::cell::RefCell;
use std::collections::HashMap;

pub fn init() -> Result<(), String> {
    let state = GLOBAL_STATE.get().unwrap();

    unsafe {
        let x = PROC_HOOK_DETOUR.initialize(state.call_proc_by_id, call_proc_by_id_hook);
        x.ok().unwrap().enable().unwrap();
    }

    Ok(())
}

// We can't use our fn types here so we have to provide the entire prototype again.
static_detour! {
    static PROC_HOOK_DETOUR: unsafe extern "cdecl" fn(
        raw_types::values::Value,
        u32,
        raw_types::procs::ProcRef,
        u32,
        raw_types::values::Value,
        *mut raw_types::values::Value,
        usize,
        u32,
        u32
    ) -> raw_types::values::Value;
}

pub type ProcHook =
    for<'a, 'r> fn(&'a DMContext<'r>, Value<'a>, Value<'a>, Vec<Value<'a>>) -> Value<'a>;

thread_local!(static PROC_HOOKS: RefCell<HashMap<raw_types::procs::ProcRef, ProcHook>> = RefCell::new(HashMap::new()));

pub fn hook(proc: &Proc, hook: ProcHook) {
    PROC_HOOKS.with(|h| h.borrow_mut().insert(proc.id, hook));
}

fn call_proc_by_id_hook(
    usr_raw: raw_types::values::Value,
    proc_type: u32,
    proc_id: raw_types::procs::ProcRef,
    unknown1: u32,
    src_raw: raw_types::values::Value,
    args_ptr: *mut raw_types::values::Value,
    num_args: usize,
    unknown2: u32,
    unknown3: u32,
) -> raw_types::values::Value {
    return PROC_HOOKS.with(|h| match h.borrow().get(&proc_id) {
        Some(hook) => {
            let ctx = DMContext::new().unwrap();

            let src;
            let usr;
            let args: Vec<Value>;

            unsafe {
                src = Value::from_raw(src_raw);
                usr = Value::from_raw(usr_raw);
                args = std::slice::from_raw_parts(args_ptr, num_args)
                    .iter()
                    .map(|v| Value::from_raw(*v))
                    .collect();
            }

            raw_types::values::Value::from(&hook(&ctx, src, usr, args))
        }
        None => unsafe {
            PROC_HOOK_DETOUR.call(
                usr_raw, proc_type, proc_id, unknown1, src_raw, args_ptr, num_args, unknown2,
                unknown3,
            )
        },
    });
}
