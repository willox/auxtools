use super::procs;
use super::strings;

pub type GetProcArrayEntry = unsafe extern "cdecl" fn(procs::ProcRef) -> *mut procs::Proc;
pub type GetStringId = unsafe extern "cdecl" fn(&str, bool, bool, bool) -> u32;
