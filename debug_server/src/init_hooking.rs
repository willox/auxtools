use auxtools::*;
use detour::RawDetour;
use raw_types::values::IntoRawValue;
use std::collections::{hash_map, HashMap};
use std::ffi::c_void;
use std::sync::Mutex;

extern crate lazy_static;
use lazy_static::lazy_static;

// Hooking code stolen, er, adapted from instruction_hooking.rs, look there if you need explanation

// This points to the bit of space after the object is created and (init) called but before New() is called.
// Really don't like this name
static mut PRE_CALL_NEW: *const c_void = std::ptr::null();

lazy_static! {
	static ref DEFAULT_VARIABLE_OVERRIDES: Mutex<HashMap<String, HashMap<String, raw_types::values::Value>>> =
		Mutex::new(HashMap::new());
}

extern "C" {
	static mut pre_call_new_original: *const c_void;
	fn pre_call_new_hook();
}

#[init(full)]
fn init_hooking_init(_: &DMContext) -> Result<(), String> {
	let byondcore = sigscan::Scanner::for_module(BYONDCORE).unwrap();

	if cfg!(windows) {
		let ptr = byondcore
			.find(signature!(
				"8B 45 14 85 C0 6A 00 6A 00 0F B7 C3 50 57 FF 76 04 FF 36 6A 03"
			))
			.ok_or_else(|| "Couldn't find PRE_CALL_NEW")?;

		unsafe {
			PRE_CALL_NEW = ptr as *const c_void;
		}
	}

	// Willox please add details
	/*if cfg!(unix) {
		let ptr = byondcore
			.find(signature!(
				"0F B7 47 ?? 8B 57 ?? 0F B7 D8 8B 0C ?? 81 F9 ?? ?? 00 00 77 ?? FF 24 8D ?? ?? ?? ??"
			))
			.ok_or_else(|| "Couldn't find EXECUTE_INSTRUCTION")?;

		unsafe {
			PRE_CALL_NEW = ptr as *const c_void;
		}
	}*/

	unsafe {
		let hook = RawDetour::new(PRE_CALL_NEW as *const (), pre_call_new_hook as *const ())
			.map_err(|_| "Couldn't detour PRE_CALL_NEW")?;

		hook.enable()
			.map_err(|_| "Couldn't enable PRE_CALL_NEW detour")?;

		pre_call_new_original = std::mem::transmute(hook.trampoline());

		// We never remove or disable the hook, so just forget about it.
		std::mem::forget(hook);
	}

	Ok(())
}

#[no_mangle]
extern "C" fn handle_init_hook(object: *mut raw_types::values::Value) {
	let val = unsafe { Value::from_raw(*object) };
	let typ = val.get_type().unwrap();

	let mut types = DEFAULT_VARIABLE_OVERRIDES.lock().unwrap();
	if let hash_map::Entry::Occupied(e) = types.entry(typ) {
		for (name, newval) in e.get() {
			val.set(name.as_ref(), unsafe { &Value::from_raw(*newval) });
		}
	}
}

pub fn override_default_for_type<S: Into<String>>(typ: S, variable_name: S, new_value: Value) {
	let typ = typ.into();
	let variable_name = variable_name.into();

	let mut types = DEFAULT_VARIABLE_OVERRIDES.lock().unwrap();
	let vars = types.entry(typ).or_insert(HashMap::new());
	vars.insert(variable_name, unsafe { new_value.into_raw_value() });
}

#[init(full)]
fn blah(_: &DMContext) -> Result<(), String> {
	override_default_for_type("/obj/fart", "lmao", Value::from(1337.0));
	Ok(())
}
