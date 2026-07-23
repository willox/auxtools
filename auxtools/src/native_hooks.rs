use crate::{hooks, raw_types};

/// A native function that fully replaces a DM proc.
/// Returns 1 = handled (skip interpreter), 0 = fall through to interpreter.
/// When returning 0, `out` must NOT be written (interpreter will set the
/// result).
pub type NativeProcFn = unsafe extern "C" fn(
	out: *mut raw_types::values::Value,
	src: raw_types::values::Value,
	usr: raw_types::values::Value,
	args: *mut raw_types::values::Value,
	arg_count: u32
) -> u8;

/// Called when a proc call falls through to the interpreter (no native proc
/// registered). Used for call counting / auto-tier. None = disabled.
pub type UnhandledProcObserver = fn(proc_id: raw_types::procs::ProcId);
pub type ProcCallProfileBegin = fn(proc_id: raw_types::procs::ProcId) -> u64;
pub type ProcCallProfileEnd = fn(proc_id: raw_types::procs::ProcId, handled: bool, sample_token: u64, elapsed_ns: u64);

static mut NATIVE_PROC_TABLE: Vec<Option<NativeProcFn>> = Vec::new();
// Always-on per-proc count of native invocations (handled == 1, including
// deopted ones -- the deopt path re-enters exec_proc and still returns 1).
// Used as the denominator for deopt-rate demotion decisions.
static mut NATIVE_CALL_COUNTS: Vec<u32> = Vec::new();
static mut NATIVE_HOOKS_ENABLED: bool = true;
static mut UNHANDLED_PROC_OBSERVER: Option<UnhandledProcObserver> = None;
static mut PROC_CALL_PROFILE_BEGIN: Option<ProcCallProfileBegin> = None;
static mut PROC_CALL_PROFILE_END: Option<ProcCallProfileEnd> = None;

unsafe extern "C" {
	static mut call_proc_by_id_profile_enabled: u8;
}

/// Set (or clear) the observer called when a proc falls through to the
/// interpreter. Only one observer is supported; subsequent calls replace it.
pub fn set_unhandled_proc_observer(f: Option<UnhandledProcObserver>) {
	unsafe {
		UNHANDLED_PROC_OBSERVER = f;
	}
}

/// Set (or clear) per-proc call profiling callbacks. The C++ trampoline owns
/// timing so interpreter fallback is measured after Rust returns 0.
pub fn set_proc_call_profiler(begin: Option<ProcCallProfileBegin>, end: Option<ProcCallProfileEnd>) {
	unsafe {
		PROC_CALL_PROFILE_BEGIN = begin;
		PROC_CALL_PROFILE_END = end;
		call_proc_by_id_profile_enabled = u8::from(begin.is_some() && end.is_some());
	}
}

#[unsafe(no_mangle)]
extern "C" fn call_proc_by_id_profile_begin(proc_id: raw_types::procs::ProcId) -> u64 {
	unsafe { PROC_CALL_PROFILE_BEGIN.map(|begin| begin(proc_id)).unwrap_or(0) }
}

#[unsafe(no_mangle)]
extern "C" fn call_proc_by_id_profile_end(proc_id: raw_types::procs::ProcId, handled: u8, sample_token: u64, elapsed_ns: u64) {
	unsafe {
		if let Some(end) = PROC_CALL_PROFILE_END {
			end(proc_id, handled != 0, sample_token, elapsed_ns);
		}
	}
}

#[allow(clippy::too_many_arguments)]
fn intercept_proc_call(
	ret: *mut raw_types::values::Value,
	usr_raw: raw_types::values::Value,
	_proc_type: u32,
	proc_id: raw_types::procs::ProcId,
	_unknown1: u32,
	src_raw: raw_types::values::Value,
	args_ptr: *mut raw_types::values::Value,
	num_args: usize,
	_unknown2: u32,
	_unknown3: u32
) -> u8 {
	unsafe {
		if NATIVE_HOOKS_ENABLED {
			if let Some(Some(func)) = NATIVE_PROC_TABLE.get(proc_id.0 as usize) {
				let handled = func(ret, src_raw, usr_raw, args_ptr, num_args as u32);
				if handled == 1 {
					let idx = proc_id.0 as usize;
					if idx >= NATIVE_CALL_COUNTS.len() {
						NATIVE_CALL_COUNTS.resize(idx + 1, 0);
					}
					NATIVE_CALL_COUNTS[idx] += 1;
					// `call_proc_by_id` normally releases the caller-provided args
					// array (one ref per arg) when the proc completes — the same
					// release the PROC_HOOKS path performs via Value::from_raw_owned
					// + drop. The native-proc short-circuit (and the deopt path,
					// which re-enters exec_proc directly rather than through
					// call_proc_by_id) both bypass that release, so do it here.
					// Without this, every refcountable arg (datum/list/icon/string)
					// leaks one ref per JIT'd call. Numbers/null are no-ops.
					for i in 0..num_args {
						raw_types::funcs::dec_ref_count(*args_ptr.add(i));
					}
				}
				return handled;
			}
		}
		// No native proc handled it — notify observer for call counting.
		if let Some(observer) = UNHANDLED_PROC_OBSERVER {
			observer(proc_id);
		}
	}
	0
}

/// Register a native function to replace the DM proc with the given id.
/// The native function runs instead of BYOND's bytecode interpreter.
pub fn register_native_proc(proc_id: raw_types::procs::ProcId, func: NativeProcFn) {
	unsafe {
		let idx = proc_id.0 as usize;
		if idx >= NATIVE_PROC_TABLE.len() {
			NATIVE_PROC_TABLE.resize(idx + 1, None);
		}
		NATIVE_PROC_TABLE[idx] = Some(func);
	}
}

/// Total native invocations recorded for this proc (handled == 1, including
/// deopted runs). Returns 0 if the proc never ran natively.
pub fn native_call_count(proc_id: raw_types::procs::ProcId) -> u32 {
	unsafe { NATIVE_CALL_COUNTS.get(proc_id.0 as usize).copied().unwrap_or(0) }
}

/// Clear the native hook for the given proc, reverting it to the bytecode
/// interpreter. Used to permanently demote a proc that thrashes on deopt.
pub fn unregister_native_proc(proc_id: raw_types::procs::ProcId) {
	unsafe {
		let idx = proc_id.0 as usize;
		if let Some(slot) = NATIVE_PROC_TABLE.get_mut(idx) {
			*slot = None;
		}
	}
}

/// Enable or disable native proc dispatch. When disabled, all calls fall
/// through to normal DM bytecode execution regardless of the table.
pub fn set_native_hooks_enabled(enabled: bool) {
	unsafe {
		NATIVE_HOOKS_ENABLED = enabled;
	}
}

pub fn is_native_hooks_enabled() -> bool {
	unsafe { NATIVE_HOOKS_ENABLED }
}

pub(crate) fn init() {
	hooks::install_interceptor(intercept_proc_call);
}
