use std::{ffi::CString, os::raw::c_char};

#[cfg(windows)]
use auxtools::byond_scan::{Anchor, Extract, Recipe, SignatureTreatment, VersionRange};
use auxtools::*;
use region::Protection;

static mut STRING_PTR: *mut *const c_char = std::ptr::null_mut();

// The `push` of the "Guest-%u" format string. We want the address of its 4-byte
// immediate, one byte past the opcode, so we can swap the string pointer out.
// Copied from `byond_catalog` in the byond-re repo, which checks it against
// every local build.
#[cfg(windows)]
const GUEST_NAME_FORMAT: Recipe = Recipe {
	name: "guest_name_format",
	versions: VersionRange { min: 1659, max: 1688 },
	anchor: Anchor::Signature(
		SignatureTreatment::NoOffset,
		"68 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 0C 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 85 ?? ?? ?? ??"
	),
	hops: &[],
	extract: Extract::Offset(1)
};

#[init(full)]
fn ckey_override_init() -> Result<(), String> {
	// This feature soft-fails
	#[cfg(windows)]
	if let Ok(address) = find_recipe(&GUEST_NAME_FORMAT) {
		unsafe {
			STRING_PTR = address as *mut *const c_char;
		}
	}

	Ok(())
}

#[derive(Debug)]
pub enum Error {
	UnsupportedByondVersion,
	InvalidString
}

pub fn override_guest_ckey(name: &str) -> Result<(), Error> {
	unsafe {
		if STRING_PTR.is_null() {
			return Err(Error::UnsupportedByondVersion);
		}
	}

	let name = name.replace('%', "%%");

	let new_ptr = CString::new(name).map_err(|_| Error::InvalidString)?.into_raw();

	unsafe {
		region::protect(STRING_PTR as *const u8, 4, Protection::READ_WRITE_EXECUTE).unwrap();

		// Leak is fine
		*STRING_PTR = new_ptr;
	}

	Ok(())
}
