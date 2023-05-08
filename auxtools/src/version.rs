use super::*;
use std::ffi::CString;
use std::os::raw::c_char;

pub static mut BYOND_VERSION_MAJOR: u32 = 0;
pub static mut BYOND_VERSION_MINOR: u32 = 0;

#[cfg(windows)]
static GET_BYOND_VERSION_SYMBOL: &[u8] = b"?GetByondVersion@ByondLib@@QAEJXZ\0";

#[cfg(windows)]
static GET_BYOND_BUILD_SYMBOL: &[u8] = b"?GetByondBuild@ByondLib@@QAEJXZ\0";

#[cfg(unix)]
static GET_BYOND_VERSION_SYMBOL: &[u8] = b"_ZN8ByondLib15GetByondVersionEv\0";

#[cfg(unix)]
static GET_BYOND_BUILD_SYMBOL: &[u8] = b"_ZN8ByondLib13GetByondBuildEv\0";

pub fn init() -> Result<(), String> {
	let get_byond_version: extern "C" fn() -> u32;
	let get_byond_build: extern "C" fn() -> u32;

	#[cfg(windows)]
	{
		use winapi::um::libloaderapi;

		unsafe {
			let mut module = std::ptr::null_mut();
			let core_str = CString::new(BYONDCORE).unwrap();
			if libloaderapi::GetModuleHandleExA(0, core_str.as_ptr(), &mut module) == 0 {
				return Err("Couldn't get module handle for BYONDCORE".into());
			}

			let symbol = libloaderapi::GetProcAddress(
				module,
				GET_BYOND_VERSION_SYMBOL.as_ptr() as *const c_char,
			);
			if symbol.is_null() {
				return Err("Couldn't find get_byond_version in BYONDCORE".into());
			}

			get_byond_version = std::mem::transmute(symbol);

			let symbol = libloaderapi::GetProcAddress(
				module,
				GET_BYOND_BUILD_SYMBOL.as_ptr() as *const c_char,
			);
			if symbol.is_null() {
				return Err("Couldn't find get_byond_build in BYONDCORE".into());
			}

			get_byond_build = std::mem::transmute(symbol);
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

			let symbol = dlsym(module, GET_BYOND_VERSION_SYMBOL.as_ptr() as *const c_char);
			if symbol.is_null() {
				return Err("Couldn't find get_byond_version in BYONDCORE".into());
			}

			get_byond_version = std::mem::transmute(symbol);

			let symbol = dlsym(module, GET_BYOND_BUILD_SYMBOL.as_ptr() as *const c_char);
			if symbol.is_null() {
				return Err("Couldn't find get_byond_build in BYONDCORE".into());
			}

			get_byond_build = std::mem::transmute(symbol);
		}
	}

	unsafe {
		BYOND_VERSION_MAJOR = get_byond_version();
		BYOND_VERSION_MINOR = get_byond_build();
	}

	Ok(())
}

pub fn get() -> (u32, u32) {
	unsafe { (BYOND_VERSION_MAJOR, BYOND_VERSION_MINOR) }
}
