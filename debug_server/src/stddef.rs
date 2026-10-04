use std::{
	ffi::{c_void, CStr},
	os::raw::c_char
};

use auxtools::{byond_scan::find_export, *};

#[cfg(windows)]
const STDDEF_FN_SYMBOL: &CStr = c"?StdDefDM@DungBuilder@@QAEPADXZ";

#[cfg(unix)]
const STDDEF_FN_SYMBOL: &CStr = c"_ZN11DungBuilder8StdDefDMEv";

static mut STDDEF: Option<&'static str> = None;

#[init(full)]
fn stddef_init() -> Result<(), String> {
	let symbol = find_export(STDDEF_FN_SYMBOL);
	if symbol.is_null() {
		return Err("Couldn't find STDDEF_FN in BYONDCORE".into());
	}
	let stddef_fn: extern "C" fn(*const c_void) -> *const c_char = unsafe { std::mem::transmute(symbol) };

	unsafe {
		match CStr::from_ptr(stddef_fn(std::ptr::null())).to_str() {
			Ok(str) => STDDEF = Some(str),
			Err(e) => {
				return Err(format!("Couldn't convert STDDEF from CStr: {}", e));
			}
		}
	}

	Ok(())
}

pub fn get_stddef() -> Option<&'static str> {
	unsafe { STDDEF }
}
