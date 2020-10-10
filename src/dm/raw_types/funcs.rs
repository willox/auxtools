use super::lists;
use super::procs;
use super::strings;
use super::values;

use std::os::raw::c_char;

pub type GetProcArrayEntry = unsafe extern "cdecl" fn(procs::ProcId) -> *mut procs::ProcEntry;
pub type GetStringId = unsafe extern "cdecl" fn(*const c_char, bool, bool, bool) -> u32;
pub type CallProcById = unsafe extern "cdecl" fn(
	values::Value,
	u32,
	procs::ProcId,
	u32,
	values::Value,
	*mut values::Value,
	usize,
	u32,
	u32,
) -> values::Value;
pub type CallDatumProcByName = unsafe extern "cdecl" fn(
	values::Value,
	u32,
	strings::StringId,
	values::Value,
	*mut values::Value,
	usize,
	u32,
	u32,
) -> values::Value;
pub type GetVariable = unsafe extern "cdecl" fn(values::Value, u32) -> values::Value;
pub type SetVariable = unsafe extern "cdecl" fn(values::Value, u32, values::Value);
pub type GetStringTableEntry = unsafe extern "cdecl" fn(u32) -> *const strings::StringEntry;
pub type IncRefCount = unsafe extern "cdecl" fn(values::Value);
pub type DecRefCount = unsafe extern "cdecl" fn(values::Value);
pub type GetListById = unsafe extern "cdecl" fn(u32) -> *mut lists::List;
pub type GetAssocElement = unsafe extern "cdecl" fn(values::Value, values::Value) -> values::Value;
