use super::procs;
use super::values;

use std::os::raw::c_char;

pub type GetProcArrayEntry = unsafe extern "cdecl" fn(procs::ProcRef) -> *mut procs::Proc;
pub type GetStringId = unsafe extern "cdecl" fn(*const c_char, bool, bool, bool) -> u32;
pub type CallGlobalProc = unsafe extern "cdecl" fn(
	values::Value,
	u32,
	u32,
	u32,
	values::Value,
	*mut values::Value,
	usize,
	u32,
	u32,
) -> values::Value;
