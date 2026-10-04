pub static mut BYOND_VERSION_MAJOR: u32 = 0;
pub static mut BYOND_VERSION_MINOR: u32 = 0;

pub fn init() -> Result<(), String> {
	let (major, minor) = byond_scan::version().map_err(|err| format!("Couldn't read the BYOND version ({err})"))?;

	unsafe {
		BYOND_VERSION_MAJOR = major;
		BYOND_VERSION_MINOR = minor;
	}

	Ok(())
}

pub fn get() -> (u32, u32) {
	unsafe { (BYOND_VERSION_MAJOR, BYOND_VERSION_MINOR) }
}
