//! For when BYOND is not enough. Probably often.

//#[cfg(not(target_pointer_width = "32"))]
// compile_error!("Auxtools must be compiled for a 32-bit target");

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
mod value_from;
pub mod version;
mod weak_value;

use std::{
	ffi::c_void,
	sync::atomic::{AtomicBool, Ordering}
};

pub use auxtools_impl::{full_shutdown, hook, init, pin_dll, runtime_handler, shutdown};
/// Used by the [pin_dll] macro to set dll pinning
pub use ctor;
pub use hooks::{CompileTimeHook, RuntimeErrorHook};
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
pub use value::Value;
pub use weak_value::WeakValue;

// We need winapi to call GetModuleHandleExW which lets us prevent our DLL from
// unloading.
#[cfg(windows)]
extern crate winapi;

#[cfg(windows)]
pub const BYONDCORE: &str = "byondcore.dll";
#[cfg(windows)]
signatures! {
	get_proc_array_entry => version_dependent_signature!(
		1630.. => (call, "E8 ?? ?? ?? ?? 8B 4D 0C 8B D0 83 C4 04 89 55 DC 8D 46 10 F6 C1 01"),
		..1630 => (call, "E8 ?? ?? ?? ?? 8B C8 8D 45 ?? 6A 01 50 FF 76 ?? 8A 46 ?? FF 76 ?? FE C0")
	),
	get_string_id => universal_signature!("55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35 ?? ?? ?? ?? 57 85 C0 75 ?? 68 ?? ?? ?? ??"),
	call_proc_by_id => version_dependent_signature!(
		1648.. => "55 8B EC 81 EC 9C 00 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45 ??",
		1602..1648 => "55 8B EC 81 EC 98 00 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 FC 8B 55",
		..1602 => (call, "E8 ?? ?? ?? ?? 83 C4 2C 89 45 F4 89 55 F8 8B 45 F4 8B 55 F8 5F 5E 5B 8B E5 5D C3 CC 55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF")
	),
	get_variable => version_dependent_signature!(
		1648.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 8B 75 ??",
		1615..1648 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 28 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4 64 A3 00 00 00 00 8B 5D",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D",
		..1602 => "55 8B EC 8B 4D ?? 0F B6 C1 48 83 F8 ?? 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 ?? FF 75 ?? E8"
	),
	set_variable => version_dependent_signature!(
		1648.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 24 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ??",
		1615..1648 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 08 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D",
		..1602 => "55 8B EC 8B 4D 08 0F B6 C1 48 57 8B 7D 10 83 F8 53 0F ?? ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 18 FF 75 14 57 FF 75 0C E8 ?? ?? ?? ?? 83 C4 10 5F 5D C3"),
	get_string_table_entry => universal_signature!("55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 10 A1"),
	call_datum_proc_by_name => version_dependent_signature!(
		1615.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75 1C 8D 45 F3 8B 7D 18 8B 5D 10 6A 00",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 18 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75 14 8D 45 F3 8B 5D 10 6A 01",
		..1602 => "55 8B EC 83 EC 0C 53 8B 5D 10 8D 45 FF 56 8B 75 14 57 6A 01 50 FF 75 1C C6 45 FF 00 FF 75 18 6A 00 56"
	),
	/* ..1614 "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74 ?? 57 FF 75 ??"
	   1615.. "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 13 85 FF 74 0F 8B 4D 14 57 53 56" */
	dec_ref_count => universal_signature!(call, "E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74"),
	/* ..1614 "E8 ?? ?? ?? ?? FF 77 ?? FF 77 ?? E8 ?? ?? ?? ?? 8D 77 ?? 56 E8 ?? ?? ?? ??"
	   1615.. "E8 ?? ?? ?? ?? FF 73 14 FF 73 10 E8 ?? ?? ?? ?? 8D 73 30 56 E8" */
	inc_ref_count => universal_signature!(call, "E8 ?? ?? ?? ?? FF ?? ?? FF ?? ?? E8 ?? ?? ?? ?? 8D ?? ?? 56 E8"),
	get_assoc_element => version_dependent_signature!(
		1614.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 51 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 10 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D",
		..1602 => "55 8B EC 51 8B 4D 08 C6 45 FF 00 80 F9 05 76 11 80 F9 21 74 10 80 F9 0D 74 0B 80 F9 0E 75 65 EB 04 84 C9 74 5F 6A 00 8D 45 FF 50 FF 75 0C 51 6A 00 6A 7B"
	),
	set_assoc_element => version_dependent_signature!(
		1648.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 1C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 80 FB 0F",
		1615..1648 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 14 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D",
		..1602 => "55 8B EC 83 EC 14 8B 4D 08 C6 45 FF 00 80 F9 05 76 15 80 F9 21 74 14 80 F9 0D 74 0F 80 F9 0E 0F 85 ?? ?? ?? ?? EB 04 84 C9 74 7A 6A 00"
	),
	create_list => version_dependent_signature!(
		1615.. => "55 8B EC 8B 0D ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 0D ?? ?? ?? ?? 8B 34 88 81 FE FF FF 00 00 0F 85 EC 00 00 00 8B 35 ?? ?? ?? ?? 81 FE FF FF FF 00 75 1B 56 68 00 01 00 00 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C B8 FF FF 00 00 5E 5D C3",
		..1614 => "55 8B EC 8B ?? ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 ?? ?? ?? ?? ?? 8B 34 88 81 FE ?? ?? ?? ?? 0F 85 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 8B F1 81 F9 ?? ?? ?? ?? 75 1B 51 68 ?? ?? ?? ?? 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C B8 ?? ?? ?? ?? 5E 5D C3"
	),
	append_to_list => version_dependent_signature!(
		1648.. => "55 8B EC 8B 4D ?? 0F B6 C1 48 56 57 83 F8 54 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ??",
		1615..1648 => "55 8B EC 8B 4D 08 0F B6 C1 48 56 57 83 F8 53 0F 87 AA 00 00 00 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 83 C4 04 85 F6 0F 84 94 00 00 00 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 7E 0C 83 C4 08 85",
		..1614 => "55 8B EC 8B 4D 08 0F B6 C1 48 56 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? FF 24 ?? ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 83 C4 04 85 F6 0F 84 ?? ?? ?? ?? 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 56 0C 83 C4 08 85 D2"
	),
	remove_from_list => version_dependent_signature!(
		1648.. => "55 8B EC 83 EC 08 53 8B 5D ?? 0F B6 C3 48 56 57 83 F8 54 0F 87 ?? ?? ?? ??",
		1615..1648 => "55 8B EC 83 EC 08 53 8B 5D 08 0F B6 C3 48 56 57 83 F8 53 0F 87 23 01 00 00 0F B6 80 ?? ?? ?? ?? 8B 4D 10 FF 24 85 ?? ?? ?? ?? 8B 7D 0C 6A 0F 57 53 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 02 01 00 00 8B 08 8B 40 0C 85 C0 0F 84 F5 00 00 00 8B 75 14 8B 5D 10",
		..1614 => "55 8B EC 8B 4D 08 83 EC 0C 0F B6 C1 48 53 83 F8 53 0F 87 ?? ?? ?? ?? 0F B6 ?? ?? ?? ?? ?? 8B 55 10 FF 24 ?? ?? ?? ?? ?? 6A 0F FF 75 0C 51 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 ?? ?? ?? ?? 8B 48 0C 8B 10 85 C9 0F 84 ?? ?? ?? ?? 8B 45 14 8B 5D 10"
	),
	get_length => universal_signature!("55 8B EC 8B 4D ?? 83 EC ?? 0F B6 C1 48 53 56 57 83 F8 ?? 0F 87 ?? ?? ?? ??"
	),
	get_misc_by_id => version_dependent_signature!(
		1615.. => (call, "E8 ?? ?? ?? ?? 83 C4 04 85 C0 74 08 0F B7 38 8B 70 08 EB 04 33 FF 33 F6 0F B7 C7 50 89 45 F8 E8 ?? ?? ?? ??"),
		..1614 => (call, "E8 ?? ?? ?? ?? 83 C4 04 85 C0 75 ?? FF 75 ?? E8 ?? ?? ?? ?? FF 30 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? A1 ?? ?? ?? ??")
	),
	runtime => universal_signature!(call, "E8 ?? ?? ?? ?? 83 C4 04 8B 85 ?? ?? ?? ?? 0F B6 C0 51 66 0F 6E C0 0F 5B C0"),
	suspended_procs => version_dependent_signature!(
		1615.. => (2, "8B 1D ?? ?? ?? ?? 56 8B 75 ?? 57 8B 3D ?? ?? ?? ?? 89 7D ?? 8B 86"),
		..1614 => (1, "A1 ?? ?? ?? ?? 8B D8 89 45 ?? 89 75 ?? 3B DA 73 ?? 8D 0C ?? D1 E9 8B 04 ??")
	),
	suspended_procs_buffer => version_dependent_signature!(
		1615.. => (2, "8B 3D ?? ?? ?? ?? 89 7D ?? 8B 86 ?? ?? ?? ?? 89 45 ?? A1 ?? ?? ?? ?? 8B"),
		..1614 => (2, "8B 35 ?? ?? ?? ?? 8B 80 ?? ?? ?? ?? 57 8B 3D ?? ?? ?? ?? 8B D7 89 45 ??")
	),
	to_string => version_dependent_signature!(
		1648.. => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 38 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 75 ?? 8B 5D ??",
		1615..1648 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 24 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75 ?? 8B 5D ?? BF 00 90 00 00 0F B6 C3 48 83 F8 53 0F 87",
		1602..1614 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 3C 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75",
		1585..1602 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 1D ?? ?? ?? ??",
		1561..1585 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 18 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ?? 0F B6 C1",
		1543..1561 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ??",
		..1543 => "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 10 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 0F B6 C3"),

	current_execution_context => version_dependent_signature!(
		1615.. => (1, "A1 ?? ?? ?? ?? 56 53 6A 00 8B 00 57 6A ?? 89 4D ?? FF 70 ?? 8B 4D ?? FF"),
		..1614 => (1, "A1 ?? ?? ?? ?? FF 75 ?? 89 4D ?? 8B 4D ?? 8B 00 6A 00 52 6A 12 FF 70 ??")
	),
	variable_names => version_dependent_signature!(
		1615.. => (2, "8B 0D ?? ?? ?? ?? 0F B7 C2 89 45 08 8B 04 87 39 34 81 74 35 0F B7 F3 66 3B DA 76 25 0F 1F 84 00 00 00 00 00"),
		..1614 => (2, "8B 1D ?? ?? ?? ?? 2B 0C ?? 8B 5D ?? 74 ?? 85 C9 79 ?? 0F B7 D0 EB ?? 83 C0 02")
	)
}

#[cfg(unix)]
pub const BYONDCORE: &str = "libbyond.so";
#[cfg(unix)]
signatures! {
	get_proc_array_entry => version_dependent_signature!(
		1584.. => (call, "E8 ?? ?? ?? ?? 0F B7 F6 89 C7 89 B5 ?? ?? ?? ?? 89 34 24 E8 ?? ?? ?? ??"),
		..1584 => (call, "E8 ?? ?? ?? ?? 8B 00 89 04 24 E8 ?? ?? ?? ?? 8B 00 89 44 24 ?? 8D 45 ??")
	),
	get_string_id => universal_signature!("55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??"),
	call_proc_by_id => universal_signature!(call, "E8 ?? ?? ?? ?? 8B 45 ?? 8B 55 ?? 89 45 ?? 89 55 ?? 8B 55 ?? 8B 4D ?? 8B 5D ??"),
	get_variable => universal_signature!("55 89 E5 81 EC C8 00 00 00 8B 55 ?? 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ??"),
	set_variable => version_dependent_signature!(
		1560.. => (call, "E8 ?? ?? ?? ?? 8B 45 ?? 8D 65 ?? 5B 5E 5F 5D C3 8D B4 26 00 00 00 00 8B 40 ??"),
		1543..1560 => "55 89 E5 81 EC A8 00 00 00 8B 55 ?? 89 5D ?? 8B 4D ?? 89 7D ?? 8B 5D ??",
		..1543 => "55 89 E5 81 EC A8 00 00 00 8B 55 ?? 8B 45 ?? 89 5D ?? 8B 5D ?? 89 7D ??"
	),

	get_string_table_entry => universal_signature!("55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??"),
	call_datum_proc_by_name => version_dependent_signature!(
		1606.. => "55 89 E5 57 56 89 CE 53 89 D3 83 EC ?? 0F B6 55 ?? 89 45 ?? 8B 45 ?? 8B 7D ?? C6 45 E7 ?? 0F B6 CA 89 45 B0 8D 45 ?? 89 44 24 ?? 8B 45 ?? 89 ?? BC 31 C9 88 ?? BB 8B 55 ?? C7 44 24 ?? 01 00 00 00",
		..1606 => "55 89 E5 57 56 53 83 EC 5C 8B 55 ?? 0F B6 45 ?? 8B 4D ?? 8B 5D ?? 89 14 24 8B 55 ?? 88 45 ?? 0F B6 F8 8B 75 ?? 8D 45 ?? 89 44 24 ?? 89 F8 89 4C 24 ?? 31 C9 C6 45 ?? 00 C7 44 24 ?? 01 00 00 00"
	),

	dec_ref_count => version_dependent_signature!(
		1543.. => (call, "E8 ?? ?? ?? ?? C7 06 00 00 00 00 C7 46 ?? 00 00 00 00 A1 ?? ?? ?? ?? 0F B7 50 ??"),
		..1543 => (call, "E8 ?? ?? ?? ?? 8B 4D ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 0C 24")
	),
	inc_ref_count => universal_signature!(call, "E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D"),
	get_assoc_element => version_dependent_signature!(
		1602.. => "55 89 E5 83 EC ?? ?? ?? ?? ?? 5D F4 89 D3 89 75 F8 89 D6 89 7D FC 89 CF 89 45 B4 0F 84 B7 00 00 ??",
		..1602 => "55 89 E5 83 EC 68 89 4D ?? B9 7B 00 00 00 89 5D ?? 89 D3 89 75 ?? 89 C6"
	),

	set_assoc_element => version_dependent_signature!(
		1602.. => "55 89 E5 83 EC 68 89 75 F8 8B 75 08 89 5D F4 89 C3 8B 45 0C 89 7D FC 80 FB 3C 89 D7 88 5D BF 89 ??",
		..1602 => "55 B9 7C 00 00 00 89 E5 83 EC 58 89 7D ?? 8B 7D ?? 89 5D ?? 89 C3 8B 45 ??"
	),

	create_list => universal_signature!("55 89 E5 57 56 53 83 EC 2C A1 ?? ?? ?? ?? 8B 75 ?? 85 C0 0F 84 ?? ?? ?? ??"),
	append_to_list => universal_signature!("55 89 E5 83 EC 38 3C 54 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 76 ??"),
	remove_from_list => universal_signature!("55 89 E5 83 EC 48 3C 54 89 5D ?? 89 C3 89 75 ?? 8B 75 ?? 89 7D ?? 8B 7D ??"),
	get_length => universal_signature!("55 89 E5 57 56 53 83 EC 6C 8B 45 ?? 8B 5D ?? 3C 54 76 ?? 31 F6 8D 65 ??"),
	get_misc_by_id => universal_signature!(call, "E8 ?? ?? ?? ?? 0F B7 55 ?? 03 1F 0F B7 4B ?? 89 8D ?? ?? ?? ?? 0F B7 5B ??"),
	runtime => universal_signature!(call, "E8 ?? ?? ?? ?? 31 C0 8D B4 26 00 00 00 00 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC"),
	suspended_procs => universal_signature!(1, "A3 ?? ?? ?? ?? 8D 14 ?? 73 ?? 8D 74 26 00 83 C0 01 8B 14 ?? 39 C3 89 54 ?? ??"),
	suspended_procs_buffer => universal_signature!(2, "89 35 ?? ?? ?? ?? C7 04 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 45 ?? 83 C0 08"),
	to_string => version_dependent_signature!(
		1602.. => "55 89 E5 83 ?? ?? 89 5D F4 8D ?? ?? 89 75 F8 89 7D FC 80 ?? ?? ?? ?? ?? B8",
		1560..1602 => (call, "E8 ?? ?? ?? ?? 89 04 24 E8 ?? ?? ?? ?? 8B 00 8D 4D ?? 89 0C 24"),
		1543..1560 => "55 89 E5 83 EC 68 A1 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 8B 0D ?? ?? ?? ?? 89 5D ??",
		..1543 => "55 89 E5 83 EC 58 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 80 FB 54"
	),
	current_execution_context => universal_signature!(1, "A1 ?? ?? ?? ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 74 24"),
	variable_names => version_dependent_signature!(
		1543.. => (1, "A1 ?? ?? ?? ?? 8B 13 8B 39 8B 75 ?? 8B 14 ?? 89 7D ?? 8B 3C ?? 83 EE 02"),
		..1543 => (2, "8B 35 ?? ?? ?? ?? 89 5D ?? 0F B7 08 89 75 ?? 66 C7 45 ?? 00 00 89 7D ??")
	)
}
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

		find_signatures! { byondcore,
			(current_execution_context as *mut *mut raw_types::procs::ExecutionContext),
			(suspended_procs as *mut raw_types::procs::SuspendedProcs),
			(suspended_procs_buffer as *mut raw_types::procs::SuspendedProcsBuffer),
			call_proc_by_id,
			call_datum_proc_by_name,
			get_proc_array_entry,
			get_string_id,
			get_variable,
			set_variable,
			get_string_table_entry,
			inc_ref_count,
			dec_ref_count,
			get_assoc_element,
			set_assoc_element,
			create_list,
			append_to_list,
			remove_from_list,
			get_length,
			get_misc_by_id,
			to_string,
			runtime
		}

		unsafe {
			raw_types::funcs::CURRENT_EXECUTION_CONTEXT = current_execution_context;
			raw_types::funcs::SUSPENDED_PROCS = suspended_procs;
			raw_types::funcs::SUSPENDED_PROCS_BUFFER = suspended_procs_buffer;
			raw_types::funcs::call_proc_by_id_byond = call_proc_by_id;
			raw_types::funcs::call_datum_proc_by_name_byond = call_datum_proc_by_name;
			raw_types::funcs::get_proc_array_entry_byond = get_proc_array_entry;
			raw_types::funcs::get_string_id_byond = get_string_id;
			raw_types::funcs::get_variable_byond = get_variable;
			raw_types::funcs::set_variable_byond = set_variable;
			raw_types::funcs::get_string_table_entry_byond = get_string_table_entry;
			raw_types::funcs::inc_ref_count_byond = inc_ref_count;
			raw_types::funcs::dec_ref_count_byond = dec_ref_count;
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

		if hooks::init().is_err() {
			return Some("Failed (Couldn't initialize proc hooking)".to_owned());
		}

		set_init_level(InitLevel::Partial);
	}


	if get_init_level() == InitLevel::Partial {
		did_partial = true;

		// This is a heap ptr so fetch it on partial loads
		find_signature! { byondcore, variable_names as *mut VariableNameIdTable }

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
	let byondcore = match sigscan::Scanner::for_module(BYONDCORE) {
		Some(v) => v,
		None => return Some("FAILED (Couldn't create scanner for byondcore.dll)".to_owned())
	};
	if let Err(e) = version::init() {
		return Some(format!("FAILED ({})", e));
	}
	let mut missing = Vec::<&'static str>::new();
	for (name, found) in SIGNATURES0.check_all(&byondcore) {
		if !found {
			missing.push(name);
		}
	}
	if missing.is_empty() {
		Some("SUCCESS".to_owned())
	} else {
		Some(format!("MISSING: {}", missing.join(", ")))
	}
} }
