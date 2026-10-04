use std::{
	cell::RefCell,
	collections::HashMap,
	ffi::{c_void, CStr},
	hash::BuildHasherDefault,
	os::raw::c_char
};

use fxhash::FxHashMap;
use retour::RawDetour;

use super::{proc::Proc, raw_types, value::Value};
use crate::runtime::DMResult;

#[doc(hidden)]
pub struct CompileTimeHook {
	pub proc_path: &'static str,
	pub hook: ProcHook
}

inventory::collect!(CompileTimeHook);

#[doc(hidden)]
pub struct RuntimeErrorHook(pub fn(&str));
inventory::collect!(RuntimeErrorHook);

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
		unk_2: u32
	) -> raw_types::values::Value;
}

struct Detours {
	pub runtime_detour: Option<RawDetour>,
	pub call_proc_detour: Option<RawDetour>
}

impl Detours {
	pub const fn new() -> Self {
		Self {
			runtime_detour: None,
			call_proc_detour: None
		}
	}
}

thread_local!(static DETOURS: RefCell<Detours> = const { RefCell::new(Detours::new()) });

pub enum HookFailure {
	NotInitialized,
	ProcNotFound,
	AlreadyHooked,
	UnknownFailure
}

impl std::fmt::Debug for HookFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInitialized => write!(f, "Library not initialized"),
			Self::ProcNotFound => write!(f, "Proc not found"),
			Self::AlreadyHooked => write!(f, "Proc is already hooked"),
			Self::UnknownFailure => write!(f, "Unknown failure")
		}
	}
}

pub fn init() -> Result<(), String> {
	unsafe {
		let runtime_hook = RawDetour::new(raw_types::funcs::runtime_byond as *const (), runtime_hook as *const ()).unwrap();

		runtime_hook.enable().unwrap();
		runtime_original = runtime_hook.trampoline() as *const () as *const c_void;

		let call_hook = RawDetour::new(
			raw_types::funcs::call_proc_by_id_byond as *const (),
			call_proc_by_id_hook_trampoline as *const ()
		)
		.unwrap();

		call_hook.enable().unwrap();
		call_proc_by_id_original = call_hook.trampoline() as *const () as *const c_void;

		DETOURS.with(|detours_cell| {
			let mut detours = detours_cell.borrow_mut();
			detours.runtime_detour = Some(runtime_hook);
			detours.call_proc_detour = Some(call_hook);
		});
	}
	Ok(())
}

pub fn shutdown() {
	unsafe {
		DETOURS.with(|detours_cell| {
			let detours = detours_cell.borrow();
			let runtime_hook = detours.runtime_detour.as_ref().unwrap();
			let call_proc_hook = detours.call_proc_detour.as_ref().unwrap();
			runtime_hook.disable().unwrap();
			call_proc_hook.disable().unwrap();
		});
	}
}

pub type ProcHook = fn(&Value, &Value, Vec<Value>) -> DMResult;

/// Lets a `RefCell` live in a plain `static`.
///
/// Everything here is only touched from BYOND's game thread: its proc calls and
/// init/shutdown, which it also drives. The table used to be a `thread_local!`,
/// but in a dlopen'd .so every access goes through `__tls_get_addr`, and it is
/// read on every single DM proc call.
struct GameThreadOnly<T>(T);

// SAFETY: only the game thread ever reaches these statics, see above
unsafe impl<T> Sync for GameThreadOnly<T> {}

static PROC_HOOKS: GameThreadOnly<RefCell<FxHashMap<raw_types::procs::ProcId, (ProcHook, String)>>> =
	GameThreadOnly(RefCell::new(HashMap::with_hasher(BuildHasherDefault::new())));

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook, hook_path: String) -> Result<(), HookFailure> {
	let mut map = PROC_HOOKS.0.borrow_mut();
	if let std::collections::hash_map::Entry::Vacant(e) = map.entry(id) {
		e.insert((hook, hook_path));
		Ok(())
	} else {
		Err(HookFailure::AlreadyHooked)
	}
}

pub fn clear_hooks() {
	PROC_HOOKS.0.borrow_mut().clear();
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook, p.path.to_owned()),
		None => Err(HookFailure::ProcNotFound)
	}
}

impl Proc {
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		hook_by_id(self.id, func, self.path.to_owned())
	}
}

#[no_mangle]
extern "C" fn on_runtime(error: *const c_char) {
	let str = unsafe { CStr::from_ptr(error) }.to_string_lossy();

	for func in inventory::iter::<RuntimeErrorHook> {
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
	_unknown3: u32
) -> u8 {
	// copy the fn out so no borrow is held while the hook runs DM code, which
	// can call back into here or register hooks
	let Some(hook) = PROC_HOOKS.0.borrow().get(&proc_id).map(|(hook, _)| *hook) else {
		return 0;
	};

	let (src, usr, args) = unsafe {
		(
			Value::from_raw(src_raw),
			Value::from_raw(usr_raw),
			// Taking ownership of args here
			std::slice::from_raw_parts(args_ptr, num_args)
				.iter()
				.map(|v| Value::from_raw_owned(*v))
				.collect()
		)
	};

	let result = match hook(&src, &usr, args) {
		Ok(r) => {
			let result_raw = r.raw;
			// Stealing our reference out of the Value
			std::mem::forget(r);
			result_raw
		}
		Err(e) => {
			let path = PROC_HOOKS.0.borrow()[&proc_id].1.clone();
			Proc::find("/proc/auxtools_stack_trace")
				.unwrap()
				.call(&[&Value::from_string(format!("{} HookPath: {}", e.message.as_str(), path)).unwrap()])
				.unwrap();
			Value::NULL.raw
		}
	};
	unsafe {
		*ret = result;
	}
	1
}
