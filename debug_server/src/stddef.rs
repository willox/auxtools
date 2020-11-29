use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

use auxtools::*;

#[cfg(windows)]
static STDDEF_FN_SYMBOL: &[u8] = b"?StdDefDM@DungBuilder@@QAEPADXZ\0";

#[cfg(unix)]
static STDDEF_FN_SYMBOL: &[u8] = b"_ZN11DungBuilder8StdDefDMEv\0";

static mut STDDEF: Option<&'static str> = None;

#[init(full)]
fn stddef_init(_: &DMContext) -> Result<(), String> {
	#[cfg(windows)]
	{
		use winapi::um::libloaderapi;

		unsafe {
			let mut module = std::ptr::null_mut();
			if libloaderapi::GetModuleHandleExA(
				0,
				CString::new(BYONDCORE).unwrap().as_ptr(),
				&mut module,
			) == 0
			{
				return Err("Couldn't get module handle for BYONDCORE".into());
			}

			let stddef_fn =
				libloaderapi::GetProcAddress(module, STDDEF_FN_SYMBOL.as_ptr() as *const i8);
			if stddef_fn.is_null() {
				return Err("Couldn't find STDDEF_FN in BYONDCORE".into());
			}

			let stddef_fn: extern "C" fn(*const c_void) -> *const c_char =
				std::mem::transmute(stddef_fn);

			match CStr::from_ptr(stddef_fn(std::ptr::null())).to_str() {
				Ok(str) => STDDEF = Some(str),
				Err(e) => {
					return Err(format!("Couldn't convert STDDEF from CStr: {}", e));
				}
			}
		}
	}

	Ok(())
}

pub fn get_stddef() -> Option<&'static str> {
	unsafe { STDDEF }
}
