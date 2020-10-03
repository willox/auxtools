use super::procs;
use super::strings;
use std::os::raw::c_char;

pub type GetProcArrayEntry = unsafe extern "cdecl" fn(procs::ProcRef) -> *mut procs::Proc;
pub type GetStringId = unsafe extern "cdecl" fn(*const c_char, bool, bool, bool) -> u32;
