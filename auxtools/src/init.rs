use crate::inventory;

#[derive(PartialEq, Clone, Copy)]
pub enum InitLevel {
	Full,
	Partial,
	None,
}

pub static mut REQUIRED_INIT: InitLevel = InitLevel::Full;

pub fn get_init_level() -> InitLevel {
	unsafe { REQUIRED_INIT }
}

pub fn set_init_level(level: InitLevel) {
	unsafe { REQUIRED_INIT = level }
}

//
// Hooks that run on intiailization
//
pub type InitFunc = fn() -> Result<(), String>;

#[doc(hidden)]
pub struct FullInitFunc(pub InitFunc);

#[doc(hidden)]
pub struct PartialInitFunc(pub InitFunc);

#[doc(hidden)]
pub struct PartialShutdownFunc(pub fn());

inventory::collect!(FullInitFunc);
inventory::collect!(PartialInitFunc);
inventory::collect!(PartialShutdownFunc);

pub fn run_full_init() -> Result<(), String> {
	for func in inventory::iter::<FullInitFunc> {
		func.0()?;
	}

	Ok(())
}

pub fn run_partial_init() -> Result<(), String> {
	for func in inventory::iter::<PartialInitFunc> {
		func.0()?;
	}

	Ok(())
}

pub fn run_partial_shutdown() {
	for func in inventory::iter::<PartialShutdownFunc> {
		func.0();
	}
}
