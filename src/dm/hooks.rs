use super::proc::Proc;
use super::raw_types;
use super::string::StringRef;
use super::value::Value;
use super::DMContext;
use super::GLOBAL_STATE;
use crate::raw_types::values::IntoRawValue;
use detour::static_detour;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

pub enum HookFailure {
	NotInitialized,
	ProcNotFound,
	AlreadyHooked,
	UnknownFailure,
}

impl std::fmt::Debug for HookFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInitialized => write!(f, "Library not initialized"),
			Self::ProcNotFound => write!(f, "Proc not found"),
			Self::AlreadyHooked => write!(f, "Proc is already hooked"),
			Self::UnknownFailure => write!(f, "Unknown failure"),
		}
	}
}

pub fn init() -> Result<(), String> {
	match GLOBAL_STATE.get() {
		Some(state) => unsafe {
			match PROC_HOOK_DETOUR.initialize(state.call_proc_by_id, call_proc_by_id_hook) {
				Ok(hook) => match hook.enable() {
					Ok(_) => Ok(()),
					Err(e) => Err(format!("Failed to enable hook: {}", e)),
				},
				Err(e) => Err(format!("Failed to initialize hook: {}", e)),
			}
		},
		None => Err(String::from(
			"Initialize the library first before initializing hooks.",
		)),
	}
}

// We can't use our fn types here so we have to provide the entire prototype again.
static_detour! {
	static PROC_HOOK_DETOUR: unsafe extern "cdecl" fn(
		raw_types::values::Value,
		u32,
		raw_types::procs::ProcId,
		u32,
		raw_types::values::Value,
		*mut raw_types::values::Value,
		usize,
		u32,
		u32
	) -> raw_types::values::Value;
}

pub type ProcHook =
	for<'a, 'r> fn(&'a DMContext<'r>, Value<'a>, Value<'a>, &Vec<Value<'a>>) -> Value<'a>;

thread_local! {
	static PROC_HOOKS: RefCell<HashMap<raw_types::procs::ProcId, ProcHook>> = RefCell::new(HashMap::new());
}

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook) -> Result<(), HookFailure> {
	PROC_HOOKS.with(|h| {
		let mut map = h.borrow_mut();
		match map.entry(id) {
			Entry::Vacant(v) => {
				v.insert(hook);
				Ok(())
			}
			Entry::Occupied(_) => Err(HookFailure::AlreadyHooked),
		}
	})
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook),
		None => Err(HookFailure::ProcNotFound),
	}
}

impl Proc {
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		hook_by_id(self.id, func)
	}
}

fn call_proc_by_id_hook(
	usr_raw: raw_types::values::Value,
	proc_type: u32,
	proc_id: raw_types::procs::ProcId,
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

			// Stealing our reference out of the Value
			let result = hook(&ctx, src, usr, &args);
			let result_raw = unsafe { result.into_raw_value() };
			std::mem::forget(result);

			// We have to
			for val in args {
				unsafe {
					(GLOBAL_STATE.get().unwrap().dec_ref_count)(val.into_raw_value());
				}
			}

			result_raw
		}
		None => unsafe {
			PROC_HOOK_DETOUR.call(
				usr_raw, proc_type, proc_id, unknown1, src_raw, args_ptr, num_args, unknown2,
				unknown3,
			)
		},
	});
}
