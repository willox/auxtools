use super::lists;
use super::procs;
use super::strings;
use super::values;

use std::ffi::c_void;
use std::os::raw::c_char;

// TODO: Doesn't belong here at all
pub static mut IS_INITIALIZED: bool = false;

// Function pointers exported by C++ but set by Rust
// Rust shouldn't call these so we're going to treat them as void ptrs for simplicity
extern "C" {
	pub static mut call_proc_by_id_byond: *const c_void;
	pub static mut call_datum_proc_by_name_byond: *const c_void;
	pub static mut get_proc_array_entry_byond: *const c_void;
	pub static mut get_string_id_byond: *const c_void;
	pub static mut get_variable_byond: *const c_void;
	pub static mut set_variable_byond: *const c_void;
	pub static mut get_string_table_entry_byond: *const c_void;
	pub static mut inc_ref_count_byond: *const c_void;
	pub static mut dec_ref_count_byond: *const c_void;
	pub static mut get_list_by_id_byond: *const c_void;
	pub static mut get_assoc_element_byond: *const c_void;
	pub static mut set_assoc_element_byond: *const c_void;
	pub static mut create_list_byond: *const c_void;
	pub static mut append_to_list_byond: *const c_void;
	pub static mut remove_from_list_byond: *const c_void;
	pub static mut get_length_byond: *const c_void;
}

// Functions exported by our C++ for Rust to call.
extern "C" {
	pub fn call_proc_by_id(
		out: *mut values::Value,
		usr: values::Value,
		proc_type: u32,
		proc_id: procs::ProcId,
		unk_0: u32,
		src: values::Value,
		args: *const values::Value,
		args_countL: usize,
		unk_1: u32,
		unk_2: u32,
	) -> u8;
	pub fn call_datum_proc_by_name(
		out: *mut values::Value,
		usr: values::Value,
		proc_type: u32,
		proc_name: strings::StringId,
		src: values::Value,
		args: *const values::Value,
		args_countL: usize,
		unk_0: u32,
		unk_1: u32,
	) -> u8;
	pub fn get_proc_array_entry(out: *mut *mut procs::ProcEntry, id: procs::ProcId) -> u8;
	pub fn get_string_id(
		out: *mut strings::StringId,
		string: *const c_char,
		a: u8,
		b: u8,
		c: u8,
	) -> u8;
	pub fn get_variable(
		out: *mut values::Value,
		datum: values::Value,
		index: strings::StringId,
	) -> u8;
	pub fn set_variable(datum: values::Value, index: strings::StringId, value: values::Value)
		-> u8;
	pub fn get_string_table_entry(
		out: *mut *mut strings::StringEntry,
		index: strings::StringId,
	) -> u8;
	pub fn inc_ref_count(value: values::Value) -> u8;
	pub fn dec_ref_count(value: values::Value) -> u8;
	pub fn get_list_by_id(out: *mut *mut lists::List, index: lists::ListId) -> u8;
	pub fn get_assoc_element(
		out: *mut values::Value,
		datum: values::Value,
		index: values::Value,
	) -> u8;
	pub fn set_assoc_element(
		datum: values::Value,
		index: values::Value,
		value: values::Value,
	) -> u8;
	pub fn create_list(out: *mut lists::ListId, reserve_capacity: u32) -> u8;
	pub fn append_to_list(list: values::Value, value: values::Value) -> u8;
	pub fn remove_from_list(list: values::Value, value: values::Value) -> u8;
	pub fn get_length(out: *mut u32, value: values::Value) -> u8;
}
