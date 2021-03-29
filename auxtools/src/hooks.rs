use super::proc::Proc;
use super::raw_types;
use super::value::Value;
use crate::runtime::DMResult;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use detour::RawDetour;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::{cell::RefCell, ffi::CStr};

#[doc(hidden)]
pub struct CompileTimeHook {
	pub proc_path: &'static str,
	pub hook: ProcHook,
}

impl CompileTimeHook {
	pub fn new(proc_path: &'static str, hook: ProcHook) -> Self {
		CompileTimeHook { proc_path, hook }
	}
}

inventory::collect!(CompileTimeHook);

// TODO: This is super deceptively named
#[doc(hidden)]
pub struct RuntimeHook(pub fn(&str));
inventory::collect!(RuntimeHook);

extern "C" {
	static mut call_proc_by_id_original: *const c_void;

	static mut runtime_original: *const c_void;
	fn runtime_hook(error: *const c_char);

	fn call_proc_by_id_hook_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		unk_0: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_count_l: usize,
		unk_1: u32,
		unk_2: u32,
	) -> raw_types::values::Value;
}

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
	unsafe {
		let runtime_hook = RawDetour::new(
			raw_types::funcs::runtime_byond as *const (),
			runtime_hook as *const (),
		)
		.unwrap();

		runtime_hook.enable().unwrap();
		runtime_original = std::mem::transmute(runtime_hook.trampoline());
		std::mem::forget(runtime_hook);

		let call_hook = RawDetour::new(
			raw_types::funcs::call_proc_by_id_byond as *const (),
			call_proc_by_id_hook_trampoline as *const (),
		)
		.unwrap();

		call_hook.enable().unwrap();
		call_proc_by_id_original = std::mem::transmute(call_hook.trampoline());
		std::mem::forget(call_hook);
	}
	Ok(())
}

pub type ProcHook = fn(&Value, &Value, &mut Vec<Value>) -> DMResult;

thread_local! {
	static PROC_HOOKS: RefCell<DashMap<raw_types::procs::ProcId, ProcHook>> = RefCell::new(DashMap::new());
}

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook) -> Result<(), HookFailure> {
	PROC_HOOKS.with(|h| {
		let map = h.borrow();
		let entry = map.entry(id);
		match entry {
			Entry::Vacant(v) => {
				v.insert(hook);
				Ok(())
			}
			Entry::Occupied(_) => Err(HookFailure::AlreadyHooked),
		}
	})
}

pub fn clear_hooks() {
	PROC_HOOKS.with(|h| h.borrow().clear());
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

#[no_mangle]
extern "C" fn on_runtime(error: *const c_char) {
	let str = unsafe { CStr::from_ptr(error) }.to_string_lossy();

	for func in inventory::iter::<RuntimeHook> {
		func.0(&str);
	}
}

#[no_mangle]
extern "C" fn call_proc_by_id_hook(
	ret: *mut raw_types::values::Value,
	usr_raw: raw_types::values::Value,
	_proc_type: u32,
	proc_id: raw_types::procs::ProcId,
	_unknown1: u32,
	src_raw: raw_types::values::Value,
	args_ptr: *mut raw_types::values::Value,
	num_args: usize,
	_unknown2: u32,
	_unknown3: u32,
) -> u8 {
	match PROC_HOOKS.with(|h| match h.borrow().get(&proc_id) {
		Some(hook) => {
			let src;
			let usr;
			let mut args: Vec<Value>;

			unsafe {
				src = Value::from_raw(src_raw);
				usr = Value::from_raw(usr_raw);

				// Taking ownership of args here
				args = std::slice::from_raw_parts(args_ptr, num_args)
					.iter()
					.map(|v| Value::from_raw_owned(*v))
					.collect();
			}

			let result = hook(&src, &usr, &mut args);

			match result {
				Ok(r) => {
					let result_raw = (&r).raw;
					// Stealing our reference out of the Value
					std::mem::forget(r);
					Some(result_raw)
				}
				Err(e) => {
					// TODO: Some info about the hook would be useful (as the hook is never part of byond's stack, the runtime won't show it.)
					Proc::find("/proc/auxtools_stack_trace")
						.unwrap()
						.call(&[&Value::from_string(e.message.as_str()).unwrap()])
						.unwrap();
					Some(Value::null().raw)
				}
			}
		}
		None => None,
	}) {
		Some(result) => {
			unsafe {
				*ret = result;
			}
			1
		}
		None => 0,
	}
}
