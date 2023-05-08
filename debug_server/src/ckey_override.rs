use region::Protection;
use std::{ffi::CString, os::raw::c_char};

use auxtools::*;

static mut STRING_PTR: *mut *const c_char = std::ptr::null_mut();

#[init(full)]
fn ckey_override_init() -> Result<(), String> {
	#[cfg(windows)]
	{
		let byondcore = sigscan::Scanner::for_module(BYONDCORE).unwrap();

		// This feature soft-fails
		if let Some(ptr) = byondcore.find(signature!(
			"68 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 0C 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 85 ?? ?? ?? ??"
		)) {
			unsafe {
				STRING_PTR = ptr.add(1) as *mut *const c_char;
			}
		}
	}

	Ok(())
}

#[derive(Debug)]
pub enum Error {
	UnsupportedByondVersion,
	InvalidString,
}

pub fn override_guest_ckey(name: &str) -> Result<(), Error> {
	unsafe {
		if STRING_PTR.is_null() {
			return Err(Error::UnsupportedByondVersion);
		}
	}

	let name = name.replace('%', "%%");

	let new_ptr = CString::new(name)
		.map_err(|_| Error::InvalidString)?
		.into_raw();

	unsafe {
		region::protect(STRING_PTR as *const u8, 4, Protection::READ_WRITE_EXECUTE).unwrap();

		// Leak is fine
		*STRING_PTR = new_ptr;
	}

	Ok(())
}
