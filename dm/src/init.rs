#[derive(PartialEq, Clone, Copy)]
pub enum RequiredInitLevel {
	Full,
	Partial,
	None,
}

pub static mut REQUIRED_INIT: RequiredInitLevel = RequiredInitLevel::Full;

pub fn get_init_level() -> RequiredInitLevel {
	unsafe { REQUIRED_INIT }
}

pub fn set_init_level(level: RequiredInitLevel) {
	unsafe { REQUIRED_INIT = level }
}
