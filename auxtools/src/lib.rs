#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

//! For when BYOND is not enough. Probably often.

//#[cfg(not(target_pointer_width = "32"))]
//compile_error!("Auxtools must be compiled for a 32-bit target");

mod byond_ffi;
mod bytecode_manager;
pub mod debug;
mod hooks;
mod init;
mod list;
mod proc;
pub mod raw_types;
mod runtime;
pub mod sigscan;
mod string;
mod string_intern;
mod value;
mod version;

use init::{get_init_level, set_init_level, InitLevel};

pub use auxtools_impl::{hook, init, runtime_handler, shutdown};
pub use hooks::{CompileTimeHook, RuntimeHook};
pub use init::{FullInitFunc, PartialInitFunc, PartialShutdownFunc};
pub use list::List;
pub use proc::Proc;
pub use raw_types::variables::VariableNameIdTable;
pub use runtime::{DMResult, Runtime};
use std::ffi::c_void;
pub use string::StringRef;
pub use string_intern::InternedString;
pub use value::Value;

/// Used by the [hook](attr.hook.html) macro to aggregate all compile-time hooks
pub use inventory;

// We need winapi to call GetModuleHandleExW which lets us prevent our DLL from unloading.
#[cfg(windows)]
extern crate winapi;

#[cfg(windows)]
pub const BYONDCORE: &str = "byondcore.dll";
#[cfg(windows)]
signatures! {
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B C8 8D 45 ?? 6A 01 50 FF 76 ?? 8A 46 ?? FF 76 ?? FE C0",
	get_string_id => "55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35 ?? ?? ?? ?? 57 85 C0 75 ?? 68 ?? ?? ?? ??",
	call_proc_by_id => "E8 ?? ?? ?? ?? 83 C4 2C 89 45 F4 89 55 F8 8B 45 F4 8B 55 F8 5F 5E 5B 8B E5 5D C3 CC 55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF",
	get_variable => "55 8B EC 8B 4D ?? 0F B6 C1 48 83 F8 ?? 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 ?? FF 75 ?? E8",
	get_string_table_entry => "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 10 A1",
	call_datum_proc_by_name => "55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF 56 8B 75 14 57 6A 01 50 FF 75 1C C6 45 FF 00 FF 75 18 6A 00 56",
	dec_ref_count => "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74 ?? 57 FF 75 ??",
	inc_ref_count => "E8 ?? ?? ?? ?? FF 77 ?? FF 77 ?? E8 ?? ?? ?? ?? 8D 77 ?? 56 E8 ?? ?? ?? ??",
	get_assoc_element => "55 8B EC 51 8B 4D 08 C6 45 FF 00 80 F9 05 76 11 80 F9 21 74 10 80 F9 0D 74 0B 80 F9 0E 75 65 EB 04 84 C9 74 5F 6A 00 8D 45 FF 50 FF 75 0C 51 6A 00 6A 7B",
	set_assoc_element => "55 8B EC 83 EC 14 8B 4D 08 C6 45 FF 00 80 F9 05 76 15 80 F9 21 74 14 80 F9 0D 74 0F 80 F9 0E 0F 85 ?? ?? ?? ?? EB 04 84 C9 74 7A 6A 00",
	create_list => "55 8B EC 8B ?? ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 ?? ?? ?? ?? ?? 8B 34 88 81 FE ?? ?? ?? ?? 0F 85 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 8B F1 81 F9 ?? ?? ?? ?? 75 1B 51 68 ?? ?? ?? ?? 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C B8 ?? ?? ?? ?? 5E 5D C3",
	append_to_list => "55 8B EC 8B 4D 08 0F B6 C1 48 56 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 83 C4 04 85 F6 0F 84 ?? ?? ?? ?? 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 56 0C 83 C4 08 85 D2",
	remove_from_list => "55 8B EC 8B 4D 08 83 EC 0C 0F B6 C1 48 53 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? 8B 55 10 FF 24 ?? ?? ?? ?? ?? 6A 0F FF 75 0C 51 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 ?? ?? ?? ?? 8B 48 0C 8B 10 85 C9 0F 84 ?? ?? ?? ?? 8B 45 14 8B 5D 10",
	get_length => "55 8B EC 8B 4D 08 83 EC 18 0F B6 C1 48 53 56 57 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C",
	get_misc_by_id => "E8 ?? ?? ?? ?? 83 C4 04 85 C0 75 ?? FF 75 ?? E8 ?? ?? ?? ?? FF 30 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? A1 ?? ?? ?? ??",
	runtime => "E8 ?? ?? ?? ?? 83 C4 04 8B 85 ?? ?? ?? ?? 0F B6 C0 51 66 0F 6E C0 0F 5B C0",
	suspended_procs => "A1 ?? ?? ?? ?? 8B D8 89 45 ?? 89 75 ?? 3B DA 73 ?? 8D 0C ?? D1 E9 8B 04 ??",
	suspended_procs_buffer => "8B 35 ?? ?? ?? ?? 8B 80 ?? ?? ?? ?? 57 8B 3D ?? ?? ?? ?? 8B D7 89 45 ??"
}

#[cfg(unix)]
pub const BYONDCORE: &str = "libbyond.so";
#[cfg(unix)]
signatures! {
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B 00 89 04 24 E8 ?? ?? ?? ?? 8B 00 89 44 24 ?? 8D 45 ??",
	get_string_id => "55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??",
	call_proc_by_id => "E8 ?? ?? ?? ?? 8B 45 ?? 8B 55 ?? 89 45 ?? 89 55 ?? 8B 55 ?? 8B 4D ?? 8B 5D ??",
	get_variable => "55 89 E5 81 EC C8 00 00 00 8B 55 ?? 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ??",
	get_string_table_entry => "55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??",
	call_datum_proc_by_name => "55 89 E5 57 56 53 83 EC 5C 8B 55 ?? 0F B6 45 ?? 8B 4D ?? 8B 5D ?? 89 14 24 8B 55 ?? 88 45 ?? 0F B6 F8 8B 75 ?? 8D 45 ?? 89 44 24 ?? 89 F8 89 4C 24 ?? 31 C9 C6 45 ?? 00 C7 44 24 ?? 01 00 00 00",
	dec_ref_count_513 => "E8 ?? ?? ?? ?? 8B 4D ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 0C 24",
	dec_ref_count_514 => "E8 ?? ?? ?? ?? C7 06 00 00 00 00 C7 46 ?? 00 00 00 00 A1 ?? ?? ?? ?? 0F B7 50 ??",
	inc_ref_count => "E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D",
	get_assoc_element => "55 89 E5 83 EC 68 89 4D ?? B9 7B 00 00 00 89 5D ?? 89 D3 89 75 ?? 89 C6",
	set_assoc_element => "55 B9 7C 00 00 00 89 E5 83 EC 58 89 7D ?? 8B 7D ?? 89 5D ?? 89 C3 8B 45 ??",
	create_list => "55 89 E5 57 56 53 83 EC 2C A1 ?? ?? ?? ?? 8B 75 ?? 85 C0 0F 84 ?? ?? ?? ??",
	append_to_list => "55 89 E5 83 EC 38 3C 54 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 76 ??",
	remove_from_list => "55 89 E5 83 EC 48 3C 54 89 5D ?? 89 C3 89 75 ?? 8B 75 ?? 89 7D ?? 8B 7D ??",
	get_length => "55 89 E5 57 56 53 83 EC 6C 8B 45 ?? 8B 5D ?? 3C 54 76 ?? 31 F6 8D 65 ??",
	get_misc_by_id => "E8 ?? ?? ?? ?? 0F B7 55 ?? 03 1F 0F B7 4B ?? 89 8D ?? ?? ?? ?? 0F B7 5B ??",
	runtime => "E8 ?? ?? ?? ?? 31 C0 8D B4 26 00 00 00 00 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC",
	suspended_procs => "A3 ?? ?? ?? ?? 8D 14 ?? 73 ?? 8D 74 26 00 83 C0 01 8B 14 ?? 39 C3 89 54 ?? ??",
	suspended_procs_buffer => "89 35 ?? ?? ?? ?? C7 04 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 45 ?? 83 C0 08"
}

macro_rules! find_function {
	($scanner:ident, $name:ident) => {
		let $name: *const c_void;
		if let Some(ptr) = $scanner.find(SIGNATURES.$name) {
			unsafe {
				$name = std::mem::transmute(ptr as *const c_void);
			}
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
		}
	};
}

macro_rules! find_function_by_call {
	($scanner:ident, $name:ident) => {
		let $name: *const c_void;
		if let Some(ptr) = $scanner.find(SIGNATURES.$name) {
			unsafe {
				let offset = *(ptr.offset(1) as *const isize);
				$name = ptr.offset(5).offset(offset) as *const () as *const std::ffi::c_void;
			}
		} else {
			return Some(format!("FAILED (Couldn't find {})", stringify!($name)));
		}
	};
}

macro_rules! with_scanner {
	($scanner:ident, $( $name:ident),* ) => {
		$( find_function!($scanner, $name); )*
	};
}

macro_rules! with_scanner_by_call {
	($scanner:ident, $( $name:ident),* ) => {
		$( find_function_by_call!($scanner, $name); )*
	};
}

// This strange section of code retrieves our DLL using the init function's address.
// This increments the DLL reference count, which prevents unloading.
#[cfg(windows)]
fn pin_dll() -> Result<(), ()> {
	unsafe {
		use winapi::um::libloaderapi::{
			GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
			GET_MODULE_HANDLE_EX_FLAG_PIN,
		};
		let mut module = std::ptr::null_mut();

		let res = GetModuleHandleExW(
			GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
			pin_dll as *const _,
			&mut module,
		);

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

byond_ffi_fn! { auxtools_init(_input) {
	if get_init_level() == InitLevel::None {
		return Some("SUCCESS".to_owned())
	}

	let byondcore = match sigscan::Scanner::for_module(BYONDCORE) {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
	};

	let mut did_full = false;
	let mut did_partial = false;

	if get_init_level() == InitLevel::Full {
		did_full = true;
		if let Err(e) = version::init() {
			return Some(format!("FAILED ({})", e));
		}



		with_scanner! { byondcore,
			get_string_id,
			get_variable,
			get_string_table_entry,
			call_datum_proc_by_name,
			get_assoc_element,
			set_assoc_element,
			append_to_list,
			remove_from_list,
			get_length,
			create_list,
			suspended_procs,
			suspended_procs_buffer
		}

		with_scanner_by_call! { byondcore,
			call_proc_by_id,
			get_proc_array_entry,
			inc_ref_count,
			get_misc_by_id,
			runtime
		}

		#[cfg(windows)]
		{
			with_scanner_by_call! { byondcore,
				dec_ref_count
			}

			unsafe {
				raw_types::funcs::dec_ref_count_byond = dec_ref_count;
			}
		}

		#[cfg(unix)]
		{
			if version::get().1 >= 1543 {
				with_scanner_by_call! { byondcore,
					dec_ref_count_514
				}

				unsafe {
					raw_types::funcs::dec_ref_count_byond = dec_ref_count_514;
				}
			} else {
				with_scanner_by_call! { byondcore,
					dec_ref_count_513
				}

				unsafe {
					raw_types::funcs::dec_ref_count_byond = dec_ref_count_513;
				}
			}
		}

		let mut to_string = std::ptr::null();
		{
			if cfg!(windows) {
				let res =
					if version::get().1 >= 1543 {
						byondcore.find(signature!("55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ??"))
					} else {
						byondcore.find(signature!("55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 10 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 0F B6 C3"))
					};

				if let Some(ptr) = res {
					to_string = ptr as *const std::ffi::c_void;
				}
			}

			if cfg!(unix) {
				let res =
					if version::get().1 >= 1543 {
						byondcore.find(signature!("55 89 E5 83 EC 68 A1 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 8B 0D ?? ?? ?? ?? 89 5D ??"))
					} else {
						byondcore.find(signature!("55 89 E5 83 EC 58 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 80 FB 54"))
					};

				if let Some(ptr) = res {
					to_string = ptr as *const std::ffi::c_void;
				}
			}

			if to_string.is_null() {
				return Some("FAILED (Couldn't find to_string)".to_owned());
			}
		}

		let mut set_variable = std::ptr::null();
		{
			if cfg!(windows) {
				let res = byondcore.find(signature!("55 8B EC 8B 4D 08 0F B6 C1 48 57 8B 7D 10 83 F8 53 0F ?? ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 18 FF 75 14 57 FF 75 0C E8 ?? ?? ?? ?? 83 C4 10 5F 5D C3"));

				if let Some(ptr) = res {
					set_variable = ptr as *const std::ffi::c_void;
				}
			}

			if cfg!(unix) {
				let res =
					if version::get().1 >= 1543 {
						byondcore.find(signature!("55 89 E5 81 EC A8 00 00 00 8B 55 ?? 89 5D ?? 8B 4D ?? 89 7D ?? 8B 5D ??"))
					} else {
						byondcore.find(signature!("55 89 E5 81 EC A8 00 00 00 8B 55 ?? 8B 45 ?? 89 5D ?? 8B 5D ?? 89 7D ??"))
					};

				if let Some(ptr) = res {
					set_variable = ptr as *const std::ffi::c_void;
				}
			}

			if set_variable.is_null() {
				return Some("FAILED (Couldn't find set_variable)".to_owned());
			}
		}

		let mut current_execution_context = std::ptr::null_mut();
		{
			if cfg!(windows) {
				if let Some(ptr) = byondcore.find(signature!("A1 ?? ?? ?? ?? FF 75 ?? 89 4D ?? 8B 4D ?? 8B 00 6A 00 52 6A 12 FF 70 ??")) {
					current_execution_context = unsafe { *((ptr.add(1)) as *mut *mut *mut raw_types::procs::ExecutionContext) };
				}
			}

			if cfg!(unix) {
				if let Some(ptr) = byondcore.find(signature!("A1 ?? ?? ?? ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 00 85 C0 0F 84 ?? ?? ?? ?? 8B 00")) {
					current_execution_context = unsafe { *((ptr.add(1)) as *mut *mut *mut raw_types::procs::ExecutionContext) };
				}
			}

			if current_execution_context.is_null() {
				return Some("FAILED (Couldn't find current_execution_context)".to_owned());
			}
		}

		unsafe {
			raw_types::funcs::CURRENT_EXECUTION_CONTEXT = current_execution_context;
			raw_types::funcs::SUSPENDED_PROCS = *(suspended_procs.add(1) as *mut *mut raw_types::procs::SuspendedProcs);
			raw_types::funcs::SUSPENDED_PROCS_BUFFER = *(suspended_procs_buffer.add(2) as *mut *mut raw_types::procs::SuspendedProcsBuffer);
			raw_types::funcs::call_proc_by_id_byond = call_proc_by_id;
			raw_types::funcs::call_datum_proc_by_name_byond = call_datum_proc_by_name;
			raw_types::funcs::get_proc_array_entry_byond = get_proc_array_entry;
			raw_types::funcs::get_string_id_byond = get_string_id;
			raw_types::funcs::get_variable_byond = get_variable;
			raw_types::funcs::set_variable_byond = set_variable;
			raw_types::funcs::get_string_table_entry_byond = get_string_table_entry;
			raw_types::funcs::inc_ref_count_byond = inc_ref_count;
			raw_types::funcs::get_assoc_element_byond = get_assoc_element;
			raw_types::funcs::set_assoc_element_byond = set_assoc_element;
			raw_types::funcs::create_list_byond = create_list;
			raw_types::funcs::append_to_list_byond = append_to_list;
			raw_types::funcs::remove_from_list_byond = remove_from_list;
			raw_types::funcs::get_length_byond = get_length;
			raw_types::funcs::get_misc_by_id_byond = get_misc_by_id;
			raw_types::funcs::to_string_byond = to_string;
			raw_types::funcs::runtime_byond = runtime;
		}

		if pin_dll().is_err() {
			return Some("FAILED (Could not pin the library in memory.)".to_owned());
		}

		if let Err(_) = hooks::init() {
			return Some("Failed (Couldn't initialize proc hooking)".to_owned());
		}

		set_init_level(InitLevel::Partial);
	}


	if get_init_level() == InitLevel::Partial {
		did_partial = true;

		// This is a heap ptr so fetch it on partial loads
		let mut variable_names = std::ptr::null();
		{
			if cfg!(windows) {
				if let Some(ptr) = byondcore.find(signature!("8B 1D ?? ?? ?? ?? 2B 0C ?? 8B 5D ?? 74 ?? 85 C9 79 ?? 0F B7 D0 EB ?? 83 C0 02")) {
					variable_names = unsafe { *((ptr.add(2)) as *mut *mut VariableNameIdTable) };
				}
			}

			if cfg!(unix) {
				if version::get().1 >= 1543 {
					if let Some(ptr) = byondcore.find(signature!("A1 ?? ?? ?? ?? 8B 13 8B 39 8B 75 ?? 8B 14 ?? 89 7D ?? 8B 3C ?? 83 EE 02")) {
						variable_names = unsafe { *((ptr.add(1)) as *mut *mut VariableNameIdTable) };
					}
				} else {
					if let Some(ptr) = byondcore.find(signature!("8B 35 ?? ?? ?? ?? 89 5D ?? 0F B7 08 89 75 ?? 66 C7 45 ?? 00 00 89 7D ??")) {
						variable_names = unsafe { *((ptr.add(2)) as *mut *mut VariableNameIdTable) };
					}
				};
			}

			if variable_names.is_null() {
				return Some("FAILED (Couldn't find variable_names)".to_owned());
			}
		}

		unsafe {
			raw_types::funcs::VARIABLE_NAMES = variable_names;
		}

		proc::populate_procs();

		for cthook in inventory::iter::<hooks::CompileTimeHook> {
			if let Err(e) = hooks::hook(cthook.proc_path, cthook.hook) {
				return Some(format!("FAILED (Could not hook proc {}: {:?})", cthook.proc_path, e));
			}
		}
		set_init_level(InitLevel::None);
	}

	if did_partial {
		bytecode_manager::init();
		string_intern::setup_interned_strings();
	}

	// Run user-defined initializers
	if did_full {
		if let Err(err) = init::run_full_init() {
			return Some(format!("FAILED ({})", err));
		}
	}

	if did_partial {
		if let Err(err) = init::run_partial_init() {
			return Some(format!("FAILED ({})", err));
		}
	}

	Some("SUCCESS".to_owned())
} }

byond_ffi_fn! { auxtools_shutdown(_input) {
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

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
