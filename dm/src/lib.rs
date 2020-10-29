#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

//! For when BYOND is not enough. Probably often.

#[cfg(not(target_pointer_width = "32"))]
compile_error!("Auxtools must be compiled for a 32-bit target");

mod byond_ffi;
mod callback;
mod context;
mod hooks;
mod init;
mod list;
mod proc;
pub mod raw_types;
mod runtime;
mod string;
mod value;

use init::{get_init_level, set_init_level, RequiredInitLevel};

pub use callback::Callback;
pub use context::DMContext;
pub use dm_impl::hook;
pub use hooks::CompileTimeHook;
pub use list::List;
pub use proc::Proc;
pub use runtime::{ConversionResult, DMResult, Runtime};
use std::ffi::c_void;
pub use string::StringRef;
pub use value::Value;

/// Used by the [hook](attr.hook.html) macro to aggregate all compile-time hooks
pub use inventory;

// We need winapi to call GetModuleHandleExW which lets us prevent our DLL from unloading.
#[cfg(windows)]
extern crate winapi;

macro_rules! signature {
	($sig:tt) => {
		dm_impl::convert_signature!($sig)
	};
}

macro_rules! signatures {
	( $( $name:ident => $sig:tt ),* ) => {
		struct Signatures {
			$( $name: &'static [Option<u8>], )*
		}

		static SIGNATURES: Signatures = Signatures {
			$( $name: signature!($sig), )*
		};
	}
}

#[cfg(windows)]
const BYONDCORE: &str = "byondcore.dll";
#[cfg(windows)]
signatures! {
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B C8 8D 45 ?? 6A 01 50 FF 76 ?? 8A 46 ?? FF 76 ?? FE C0",
	get_string_id => "55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35",
	call_proc_by_id => "55 8B EC 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45",
	get_variable => "55 8B EC 8B 4D ?? 0F B6 C1 48 83 F8 ?? 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 ?? FF 75 ?? E8",
	set_variable => "55 8B EC 8B 4D 08 0F B6 C1 48 57 8B 7D 10 83 F8 53 0F ?? ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 18 FF 75 14 57 FF 75 0C E8 ?? ?? ?? ?? 83 C4 10 5F 5D C3",
	get_string_table_entry => "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 10 A1",
	call_datum_proc_by_name => "55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF 56 8B 75 14 57 6A 01 50 FF 75 1C C6 45 FF 00 FF 75 18 6A 00 56",
	dec_ref_count => "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74 ?? 57 FF 75 ??",
	inc_ref_count => "E8 ?? ?? ?? ?? FF 77 ?? FF 77 ?? E8 ?? ?? ?? ?? 8D 77 ?? 56 E8 ?? ?? ?? ??",
	get_list_by_id => "E8 ?? ?? ?? ?? 83 C4 04 85 C0 75 13 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 04 5D E9 ?? ?? ?? ?? 5D C3",
	get_assoc_element => "55 8B EC 51 8B 4D 08 C6 45 FF 00 80 F9 05 76 11 80 F9 21 74 10 80 F9 0D 74 0B 80 F9 0E 75 65 EB 04 84 C9 74 5F 6A 00 8D 45 FF 50 FF 75 0C 51 6A 00 6A 7B",
	set_assoc_element => "55 8B EC 83 EC 14 8B 4D 08 C6 45 FF 00 80 F9 05 76 15 80 F9 21 74 14 80 F9 0D 74 0F 80 F9 0E 0F 85 ?? ?? ?? ?? EB 04 84 C9 74 7A 6A 00",
	create_list => "55 8B EC 8B ?? ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 ?? ?? ?? ?? ?? 8B 34 88 81 FE ?? ?? ?? ?? 0F 85 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 8B F1 81 F9 ?? ?? ?? ?? 75 1B 51 68 ?? ?? ?? ?? 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C B8 ?? ?? ?? ?? 5E 5D C3",
	append_to_list => "55 8B EC 8B 4D 08 0F B6 C1 48 56 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 83 C4 04 85 F6 0F 84 ?? ?? ?? ?? 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 56 0C 83 C4 08 85 D2",
	remove_from_list => "55 8B EC 8B 4D 08 83 EC 0C 0F B6 C1 48 53 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? 8B 55 10 FF 24 ?? ?? ?? ?? ?? 6A 0F FF 75 0C 51 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 ?? ?? ?? ?? 8B 48 0C 8B 10 85 C9 0F 84 ?? ?? ?? ?? 8B 45 14 8B 5D 10",
	get_length => "55 8B EC 8B 4D 08 83 EC 18 0F B6 C1 48 53 56 57 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C"
}

#[cfg(unix)]
const BYONDCORE: &str = "libbyond.so";
#[cfg(unix)]
signatures! {
	get_proc_array_entry => "E8 ?? ?? ?? ?? 8B 00 89 04 24 E8 ?? ?? ?? ?? 8B 00 89 44 24 ?? 8D 45 ??",
	get_string_id => "55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??",
	call_proc_by_id => "55 89 E5 81 EC D8 00 00 00 89 5D ?? 89 C3 0F B6 45 ?? 81 7D ?? FF FF 00 00",
	get_variable => "55 89 E5 81 EC C8 00 00 00 8B 55 ?? 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ??",
	set_variable => "55 89 E5 81 EC A8 00 00 00 8B 55 ?? 8B 45 ?? 89 5D ?? 8B 5D ?? 89 7D ??",
	get_string_table_entry => "55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??",
	call_datum_proc_by_name => "55 89 E5 57 56 53 83 EC 5C 8B 55 ?? 0F B6 45 ?? 8B 4D ?? 8B 5D ?? 89 14 24 8B 55 ?? 88 45 ?? 0F B6 F8 8B 75 ?? 8D 45 ?? 89 44 24 ?? 89 F8 89 4C 24 ?? 31 C9 C6 45 ?? 00 C7 44 24 ?? 01 00 00 00",
	dec_ref_count => "E8 ?? ?? ?? ?? 8B 4D ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 0C 24",
	inc_ref_count => "E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D",
	get_list_by_id => "E8 ?? ?? ?? ?? 85 C0 89 C7 0F 84 ?? ?? ?? ?? 8B 40 ?? 89 3C 24 83 C0 01",
	get_assoc_element => "55 89 E5 83 EC 68 89 4D ?? B9 7B 00 00 00 89 5D ?? 89 D3 89 75 ?? 89 C6",
	set_assoc_element => "55 B9 7C 00 00 00 89 E5 83 EC 58 89 7D ?? 8B 7D ?? 89 5D ?? 89 C3 8B 45 ??",
	create_list => "55 89 E5 57 56 53 83 EC 2C A1 ?? ?? ?? ?? 8B 75 ?? 85 C0 0F 84 ?? ?? ?? ??",
	append_to_list => "55 89 E5 83 EC 38 3C 54 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 76 ??",
	remove_from_list => "55 89 E5 83 EC 48 3C 54 89 5D ?? 89 C3 89 75 ?? 8B 75 ?? 89 7D ?? 8B 7D ??",
	get_length => "55 89 E5 57 56 53 83 EC 6C 8B 45 ?? 8B 5D ?? 3C 54 76 ?? 31 F6 8D 65 ??"
}

macro_rules! find_function {
	($scanner:ident, $name:ident) => {
		let $name: *const c_void;
		if let Some(ptr) = $scanner.find(SIGNATURES.$name.to_vec()) {
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
		if let Some(ptr) = $scanner.find(SIGNATURES.$name.to_vec()) {
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
	if get_init_level() == RequiredInitLevel::None {
		return Some("SUCCESS (Already initialized)".to_owned())
	}

	if get_init_level() == RequiredInitLevel::Full {
		let byondcore = match sigscan::Scanner::for_module(BYONDCORE) {
			Some(v) => v,
			None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
		};

		with_scanner! { byondcore,
			get_string_id,
			call_proc_by_id,
			get_variable,
			set_variable,
			get_string_table_entry,
			call_datum_proc_by_name,
			get_assoc_element,
			set_assoc_element,
			append_to_list,
			remove_from_list,
			get_length,
			create_list
		}

		with_scanner_by_call! { byondcore,
			get_proc_array_entry,
			dec_ref_count,
			inc_ref_count,
			get_list_by_id
		}

		unsafe {
			raw_types::funcs::call_proc_by_id_byond = call_proc_by_id;
			raw_types::funcs::call_datum_proc_by_name_byond = call_datum_proc_by_name;
			raw_types::funcs::get_proc_array_entry_byond = get_proc_array_entry;
			raw_types::funcs::get_string_id_byond = get_string_id;
			raw_types::funcs::get_variable_byond = get_variable;
			raw_types::funcs::set_variable_byond = set_variable;
			raw_types::funcs::get_string_table_entry_byond = get_string_table_entry;
			raw_types::funcs::inc_ref_count_byond = inc_ref_count;
			raw_types::funcs::dec_ref_count_byond = dec_ref_count;
			raw_types::funcs::get_list_by_id_byond = get_list_by_id;
			raw_types::funcs::get_assoc_element_byond = get_assoc_element;
			raw_types::funcs::set_assoc_element_byond = set_assoc_element;
			raw_types::funcs::create_list_byond = create_list;
			raw_types::funcs::append_to_list_byond = append_to_list;
			raw_types::funcs::remove_from_list_byond = remove_from_list;
			raw_types::funcs::get_length_byond = get_length;
		}



		if pin_dll().is_err() {
			return Some("FAILED (Could not pin the library in memory.)".to_owned());
		}

		if let Err(_) = hooks::init() {
			return Some("Failed (Couldn't initialize proc hooking)".to_owned());
		}

		set_init_level(RequiredInitLevel::Partial);
	}


	if get_init_level() == RequiredInitLevel::Partial {
		proc::populate_procs();

		for cthook in inventory::iter::<hooks::CompileTimeHook> {
			if let Err(e) = hooks::hook(cthook.proc_path, cthook.hook) {
				return Some(format!("FAILED (Could not hook proc {}: {:?})", cthook.proc_path, e));
			}
		}
		set_init_level(RequiredInitLevel::None);
	}


	Some("SUCCESS".to_owned())
} }

byond_ffi_fn! { auxtools_shutdown(_input) {
	hooks::clear_hooks();
	proc::clear_procs();

	set_init_level(RequiredInitLevel::Partial);
	Some("SUCCESS".to_owned())
} }

#[cfg(test)]
mod tests {
	#[test]
	fn test() {}
}
