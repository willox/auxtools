use super::procs;

pub type GetProcArrayEntry = unsafe extern "cdecl" fn(procs::ProcRef) -> *mut procs::Proc;
