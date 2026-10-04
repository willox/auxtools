#![allow(clippy::missing_const_for_fn)]
use auxtools_impl::versioned;

use super::{misc, strings, values};

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ProcId(pub u32);

#[repr(C)]
pub struct ProcEntry {
	pub path: strings::StringId,
	pub name: strings::StringId,
	pub desc: strings::StringId,
	pub category: strings::StringId,
	// the last `/` part of `path`
	path_basename: strings::StringId,
	flags: u32,
	pub metadata: ProcMetadata
}

#[versioned(
	Pre1630 if crate::version::BYOND_VERSION_MINOR <= 1627,
	Post1630,
)]
#[repr(C)]
pub struct ProcMetadata {
	// three bytes straight out of the .dmb, meaning unknown
	#[only_in(Post1630)]
	byte_24: i8,
	#[only_in(Post1630)]
	byte_25: u8,
	#[only_in(Post1630)]
	byte_26: i8,
	pub bytecode: misc::BytecodeId,
	pub locals: misc::LocalsId,
	pub parameters: misc::ParametersId,
	// the proc `..()` goes to, 0xFFFF for none. Brings the entry to the 44 bytes
	// BYOND strides its proc table by
	#[only_in(Post1630)]
	next_override: ProcId
}

#[repr(C)]
pub struct ProcInstance {
	pub proc: ProcId,
	/// The `proc_type` this frame was called with. `..()` reads it back.
	pub flags: u8,
	/// Which override of the proc this frame runs, 0 for the most derived.
	pub override_depth: u8,
	pub mega_hack: u16,
	pub usr: values::Value,
	pub src: values::Value,
	pub context: *mut ExecutionContext,
	pub argslist_idx: values::ValueData,
	unk_1: u32,
	inner: ProcInstanceInner
}

impl ProcInstance {
	pub fn args_count(&self) -> u32 {
		*self.inner.args_count()
	}

	pub fn args(&self) -> *mut values::Value {
		*self.inner.args()
	}

	pub fn time_to_resume(&self) -> u32 {
		*self.inner.time_to_resume()
	}
}

#[versioned(
	Pre516 if crate::version::BYOND_VERSION_MAJOR < 516,
	Post516,
)]
#[repr(C)]
struct ProcInstanceInner {
	#[only_in(Pre516)]
	unk_2: u32,
	// called with the result when the proc ends, null for none
	#[only_in(Post516)]
	callback: u32,
	#[only_in(Post516)]
	callback_value: u32,
	pub args_count: u32,
	pub args: *mut values::Value,
	// room for ten values, shared by the args and the locals, so a small proc
	// doesn't have to allocate
	arg_local_store: [u8; 0x50],
	internal_arg_count: u32,
	// not 0 once the store has overflowed onto the heap
	external_arg_count: u32,
	pub time_to_resume: u32
}

// Linux is 148 bytes against Windows' 152: from `cache` onward the fields sit 4
// bytes earlier, which is the same shape Windows had before 1668. So Linux
// always takes the Pre1668 variant.
//
// The whole difference is the call flags. Windows from 1668 keeps them in a
// dword of their own, everything else packs them into the byte after
// `test_flag`.
#[versioned(
	Pre1668 if cfg!(unix) || crate::version::BYOND_VERSION_MAJOR <= 515 || (crate::version::BYOND_VERSION_MAJOR == 516 && crate::version::BYOND_VERSION_MINOR <= 1667),
	Post1668,
)]
#[repr(C)]
pub struct ExecutionContext {
	pub proc_instance: *mut ProcInstance,
	pub parent_context: *mut ExecutionContext,
	pub filename: strings::StringId,
	pub line: u32,
	pub bytecode: *mut u32,
	pub bytecode_offset: u16,
	pub test_flag: u8,
	#[only_in(Pre1668)]
	exec_call_flags: u8,
	#[only_in(Post1668)]
	exec_call_flags: u32,
	cache: values::Value,
	cache_key: values::Value,
	// two pointers into exec_proc's own stack frame
	scratch_ptr_a: u32,
	scratch_ptr_b: u32,
	pub dot: values::Value,
	pub locals: *mut values::Value,
	pub stack: *mut values::Value,
	pub locals_count: u16,
	pub stack_size: u16,
	// the iterators of the loops this one is nested in
	iterator_stack: u32,
	pub current_iterator: *mut values::Value,
	pub iterator_allocated: u32,
	pub iterator_length: u32,
	pub iterator_index: u32,
	pub iterator_filter_type: values::Value,
	pub iterator_filter_bitflags: u32,
	pub iterator_kind: u8,
	// byond-re calls this one `jnz_lchk_loop_count`
	pub infinite_loop_count: u32,
	// 0 while the proc is sleeping. There is no separate "paused" byte
	not_suspended: u8,
	caller_chain_preserve_depth: u8,
	background: u8,
	exception_state: u8,
	throw_handler: u32,
	// timevals. The last is the deadline a background proc's loops run against
	time_now: [u32; 2],
	cpu_clock_init: [u32; 2],
	cpu_clock_quantum: [u32; 2],
	loop_timeout: [u32; 2]
}

#[repr(C)]
pub struct SuspendedProcsBuffer {
	pub buffer: *mut *mut ProcInstance
}

#[cfg(windows)]
#[repr(C)]
pub struct SuspendedProcs {
	pub front: usize,
	pub back: usize,
	pub capacity: usize
}

// On Linux the compiler lays the two index globals out the other way round,
// `back` first. Whatever follows them isn't read.
#[cfg(unix)]
#[repr(C)]
pub struct SuspendedProcs {
	pub back: usize,
	pub front: usize
}

#[cfg(test)]
mod layout_tests {
	use std::mem::{offset_of, size_of};

	// The numbers below are read off the binaries, 516.1688: what BYOND
	// allocates for each struct and where its own code reaches into them.
	// The ones for older builds are from byond-re's notes.

	#[test]
	fn execution_context_linux_and_old_windows() {
		type Gen = super::ExecutionContextPre1668;
		assert_eq!(size_of::<Gen>(), 0x94);
		assert_eq!(offset_of!(Gen, bytecode_offset), 0x14);
		assert_eq!(offset_of!(Gen, dot), 0x30);
		assert_eq!(offset_of!(Gen, locals), 0x38);
		assert_eq!(offset_of!(Gen, stack_size), 0x42);
		assert_eq!(offset_of!(Gen, current_iterator), 0x48);
		assert_eq!(offset_of!(Gen, iterator_kind), 0x64);
		assert_eq!(offset_of!(Gen, infinite_loop_count), 0x68);
	}

	#[test]
	fn execution_context_windows() {
		type Gen = super::ExecutionContextPost1668;
		assert_eq!(size_of::<Gen>(), 0x98);
		assert_eq!(offset_of!(Gen, bytecode_offset), 0x14);
		assert_eq!(offset_of!(Gen, dot), 0x34);
		assert_eq!(offset_of!(Gen, locals), 0x3C);
		assert_eq!(offset_of!(Gen, stack_size), 0x46);
		assert_eq!(offset_of!(Gen, current_iterator), 0x4C);
		assert_eq!(offset_of!(Gen, iterator_kind), 0x68);
		assert_eq!(offset_of!(Gen, infinite_loop_count), 0x6C);
	}

	#[test]
	fn proc_instance() {
		type Inner = super::ProcInstanceInnerPost516;
		let inner = offset_of!(super::ProcInstance, inner);
		assert_eq!(size_of::<super::ProcInstance>(), 0x90);
		assert_eq!(offset_of!(super::ProcInstance, override_depth), 0x05);
		assert_eq!(offset_of!(super::ProcInstance, context), 0x18);
		assert_eq!(inner + offset_of!(Inner, callback), 0x24);
		assert_eq!(inner + offset_of!(Inner, args_count), 0x2C);
		assert_eq!(inner + offset_of!(Inner, args), 0x30);
		assert_eq!(inner + offset_of!(Inner, external_arg_count), 0x88);
		assert_eq!(inner + offset_of!(Inner, time_to_resume), 0x8C);
		// 515 and older have one dword less in front of the args
		assert_eq!(inner + offset_of!(super::ProcInstanceInnerPre516, args_count), 0x28);
	}

	#[test]
	fn proc_entry() {
		let metadata = offset_of!(super::ProcEntry, metadata);
		assert_eq!(size_of::<super::ProcEntry>(), 44);
		assert_eq!(metadata + offset_of!(super::ProcMetadataPost1630, bytecode), 28);
		assert_eq!(metadata + offset_of!(super::ProcMetadataPost1630, next_override), 40);
		// before 1630 the entry is 36 bytes with the bytecode id at +24
		assert_eq!(metadata + size_of::<super::ProcMetadataPre1630>(), 36);
		assert_eq!(metadata + offset_of!(super::ProcMetadataPre1630, bytecode), 24);
	}
}
