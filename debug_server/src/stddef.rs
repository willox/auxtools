use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

use auxtools::*;

#[cfg(windows)]
static STDDEF_FN_SYMBOL: &[u8] = b"?StdDefDM@DungBuilder@@QAEPADXZ\0";

#[cfg(unix)]
static STDDEF_FN_SYMBOL: &[u8] = b"_ZN11DungBuilder8StdDefDMEv\0";

static mut STDDEF: Option<&'static str> = None;

#[init(full)]
fn stddef_init() -> Result<(), String> {
	let stddef_fn: extern "C" fn(*const c_void) -> *const c_char;

	#[cfg(windows)]
	{
		use winapi::um::libloaderapi;

		unsafe {
			let mut module = std::ptr::null_mut();
			let core_str = CString::new(BYONDCORE).unwrap();
			if libloaderapi::GetModuleHandleExA(0, core_str.as_ptr(), &mut module) == 0 {
				return Err("Couldn't get module handle for BYONDCORE".into());
			}

			let symbol =
				libloaderapi::GetProcAddress(module, STDDEF_FN_SYMBOL.as_ptr() as *const c_char);
			if symbol.is_null() {
				return Err("Couldn't find STDDEF_FN in BYONDCORE".into());
			}

			stddef_fn = std::mem::transmute(symbol);
		}
	}

	#[cfg(unix)]
	{
		use libc::{dlopen, dlsym, RTLD_LAZY};

		unsafe {
			let byond_core_str = CString::new(BYONDCORE).unwrap();
			let module = dlopen(byond_core_str.as_ptr(), RTLD_LAZY);
			if module.is_null() {
				return Err("Couldn't get module handle for BYONDCORE".into());
			}

			let symbol = dlsym(module, STDDEF_FN_SYMBOL.as_ptr() as *const c_char);
			if symbol.is_null() {
				return Err("Couldn't find STDDEF_FN in BYONDCORE".into());
			}

			stddef_fn = std::mem::transmute(symbol);
		}
	}

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
