//! For when BYOND is not enough. Probably often.

//#[cfg(not(target_pointer_width = "32"))]
// compile_error!("Auxtools must be compiled for a 32-bit target");

// public so `byond_ffi_fn!` can reach it from another crate
#[doc(hidden)]
pub mod byond_ffi;
mod bytecode_manager;
pub mod debug;
mod hooks;
mod init;
mod list;
mod proc;
pub mod raw_types;
mod runtime;
mod string;
mod string_intern;
mod symbols;
mod value;
mod value_from;
pub mod version;
mod weak_value;

use std::sync::{
	atomic::{AtomicBool, Ordering},
	OnceLock
};

pub use auxtools_impl::{full_shutdown, hook, init, pin_dll, runtime_handler, shutdown};
/// For crates that need to find something else in BYOND's binary.
pub use byond_scan;
/// Used by the [pin_dll] macro to set dll pinning
pub use ctor;
pub use hooks::{CompileTimeHook, HookFailure, ProcHook, RuntimeErrorHook};
use init::{get_init_level, set_init_level, InitLevel};
pub use init::{FullInitFunc, FullShutdownFunc, PartialInitFunc, PartialShutdownFunc};
/// Used by the [hook](attr.hook.html) macro to aggregate all compile-time hooks
pub use inventory;
pub use list::List;
pub use proc::Proc;
pub use raw_types::variables::VariableNameIdTable;
pub use runtime::{DMResult, Runtime};
pub use string::StringRef;
pub use string_intern::InternedString;
pub use symbols::find_recipe;
pub use value::Value;
pub use weak_value::WeakValue;

// We need winapi to call GetModuleHandleExW which lets us prevent our DLL from
// unloading.
#[cfg(windows)]
extern crate winapi;

pub static PIN_DLL: AtomicBool = AtomicBool::new(true);

// This strange section of code retrieves our DLL using the init function's
// address. This increments the DLL reference count, which prevents unloading.
#[cfg(windows)]
fn pin_dll() -> Result<(), ()> {
	unsafe {
		use winapi::um::libloaderapi::{GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN};
		let mut module = std::ptr::null_mut();

		let flags = match PIN_DLL.load(Ordering::Relaxed) {
			true => GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
			false => GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
		};

		let res = GetModuleHandleExW(flags, pin_dll as *const _, &mut module);

		if res == 0 {
			return Err(());
		}
	}
	Ok(())
}
#[cfg(unix)]
fn pin_dll() -> Result<(), ()> {
	Ok(())
}

// The first failed init. Trying again would run setup on top of what the
// failed attempt left behind, so every later call hands this back instead.
static INIT_FAILURE: OnceLock<String> = OnceLock::new();

byond_ffi_fn! { auxtools_init(_input) {
	if let Some(failure) = INIT_FAILURE.get() {
		return Some(failure.clone());
	}

	match try_init() {
		Ok(()) => Some("SUCCESS".to_owned()),
		Err(e) => {
			// a hook must not fire in a library that only got half set up
			hooks::clear_hooks();
			proc::clear_procs();
			Some(INIT_FAILURE.get_or_init(|| format!("FAILED ({})", e)).clone())
		}
	}
} }

fn try_init() -> Result<(), String> {
	if get_init_level() == InitLevel::None {
		return Ok(());
	}

	let mut did_full = false;
	let mut did_partial = false;

	if get_init_level() == InitLevel::Full {
		did_full = true;
		version::init()?;
		symbols::resolve_full()?;
		pin_dll().map_err(|()| "Could not pin the library in memory.".to_owned())?;
		hooks::init().map_err(|_| "Couldn't initialize proc hooking".to_owned())?;

		set_init_level(InitLevel::Partial);
	}

	if get_init_level() == InitLevel::Partial {
		did_partial = true;

		// This is a heap ptr so fetch it on partial loads
		symbols::resolve_partial()?;

		proc::populate_procs();

		for cthook in inventory::iter::<hooks::CompileTimeHook> {
			hooks::hook(cthook.proc_path, cthook.hook).map_err(|e| format!("Could not hook proc {}: {:?}", cthook.proc_path, e))?;
		}
		set_init_level(InitLevel::None);
	}

	if did_partial {
		bytecode_manager::init();
		string_intern::setup_interned_strings();
	}

	// Run user-defined initializers
	if did_full {
		init::run_full_init()?;
	}

	if did_partial {
		init::run_partial_init()?;
	}

	Ok(())
}

byond_ffi_fn! { auxtools_shutdown(_input) {
	if get_init_level() != InitLevel::None {
		return Some("FAILED (already shut down)".to_owned())
	};
	init::run_partial_shutdown();
	string_intern::destroy_interned_strings();
	bytecode_manager::shutdown();

	hooks::clear_hooks();
	proc::clear_procs();

	unsafe {
		raw_types::funcs::VARIABLE_NAMES = std::ptr::null();
	}

	set_init_level(InitLevel::Partial);
	Some("SUCCESS".to_owned())
} }

byond_ffi_fn! { auxtools_full_shutdown(_input) {
	if get_init_level() == InitLevel::Full {
		return Some("FAILED (already shut down)".to_owned())
	};
	if get_init_level() == InitLevel::None {
		init::run_partial_shutdown();
		string_intern::destroy_interned_strings();
		bytecode_manager::shutdown();

		hooks::clear_hooks();
		proc::clear_procs();

		unsafe {
			raw_types::funcs::VARIABLE_NAMES = std::ptr::null();
		}
	}
	hooks::shutdown();
	set_init_level(InitLevel::Full);
	init::run_full_shutdown();

	if !PIN_DLL.load(Ordering::Relaxed) {
		#[cfg(windows)]
		unsafe {
			use winapi::um::libloaderapi::{
				FreeLibrary, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
				GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
			};
			let mut module = std::ptr::null_mut();

			let get_handle_res = GetModuleHandleExW(
				GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
				auxtools_full_shutdown as *const _,
				&mut module,
			);

			if get_handle_res == 0 {
				return Some("FAILED (Could not unpin the library from memory.)".to_owned())
			}

			FreeLibrary(module);
		}
	};
	Some("SUCCESS".to_owned())
} }

byond_ffi_fn! { auxtools_check_signatures(_input) {
	if let Err(e) = version::init() {
		return Some(format!("FAILED ({})", e));
	}
	match symbols::unresolved() {
		Err(e) => Some(format!("FAILED ({})", e)),
		Ok(missing) if missing.is_empty() => Some("SUCCESS".to_owned()),
		Ok(missing) => Some(format!("MISSING: {}", missing.join(", ")))
	}
} }
