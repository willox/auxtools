use detour::static_detour;
use super::raw_types;
use super::DMContext;
use super::value::Value;
use super::GLOBAL_STATE;

pub fn init() -> Result<(), String> {
    let state = GLOBAL_STATE.get().unwrap();

    unsafe {
        let x = PROC_HOOK_DETOUR.initialize(state.call_global_proc, CallGlobalProcHook);
        x.ok().unwrap().enable().unwrap();
    }

    Ok(())
}

// We can't use our fn types here so we have to provide the entire prototype again.
static_detour! {
    static PROC_HOOK_DETOUR: unsafe extern "cdecl" fn(
        raw_types::values::Value,
        u32,
        u32,
        u32,
        raw_types::values::Value,
        *mut raw_types::values::Value,
        usize,
        u32,
        u32
    ) -> raw_types::values::Value;
}

fn CallGlobalProcHook(
    usr_raw: raw_types::values::Value,
    proc_type: u32,
    proc_id: u32,
    unknown1: u32,
    src_raw: raw_types::values::Value,
    args_ptr: *mut raw_types::values::Value,
    num_args: usize,
    unknown2: u32,
    unknown3: u32,
) -> raw_types::values::Value {
    let ctx = DMContext::new().unwrap();

    if proc_id == 1 {
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

        return raw_types::values::Value::from(&hello_proc_hook(&ctx, src, usr, args));
    }

    unsafe {
        PROC_HOOK_DETOUR.call(
            usr_raw, proc_type, proc_id, unknown1, src_raw, args_ptr, num_args, unknown2, unknown3,
        )
    }
}

fn hello_proc_hook<'a>(
    ctx: &'a DMContext,
    src: Value<'a>,
    usr: Value<'a>,
    args: Vec<Value<'a>>,
) -> Value<'a> {
    let dat = args[0];
    let v;
    {
        v = dat.get("hello").unwrap();
    }
    v
}