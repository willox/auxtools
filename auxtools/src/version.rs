use super::*;

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
		use windows::Win32::Foundation;
		use windows::Win32::System::LibraryLoader;

		unsafe {
			let mut module = Foundation::HINSTANCE::default();
			let core_str = windows::core::PCSTR::from_raw(BYONDCORE.as_ptr());
			if !LibraryLoader::GetModuleHandleExA(0, core_str, &mut module).as_bool() {
				return Err("Couldn't get module handle for BYONDCORE".into());
			}

			let symbol = LibraryLoader::GetProcAddress(
				module,
				windows::core::PCSTR::from_raw(GET_BYOND_VERSION_SYMBOL.as_ptr()),
			);
			if symbol.is_none() {
				return Err("Couldn't find get_byond_version in BYONDCORE".into());
			}

			get_byond_version = std::mem::transmute(symbol.unwrap());

			let symbol = LibraryLoader::GetProcAddress(
				module,
				windows::core::PCSTR::from_raw(GET_BYOND_BUILD_SYMBOL.as_ptr()),
			);
			if symbol.is_none() {
				return Err("Couldn't find get_byond_build in BYONDCORE".into());
			}

			get_byond_build = std::mem::transmute(symbol.unwrap());
		}
	}

	#[cfg(unix)]
	{
		use libc::{dlopen, dlsym, RTLD_LAZY};
		use std::ffi::{c_char, CString};

		unsafe {
			let module = dlopen(CString::new(BYONDCORE).unwrap().as_ptr(), RTLD_LAZY);
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
