#![allow(clippy::missing_const_for_fn)]
use std::sync::OnceLock;

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
	flags: u32,
	unk_1: u32,
	pub metadata: ProcMetadata
}

#[repr(C)]
pub union ProcMetadata {
	pub pre1630: BytecodePre1630,
	pub post1630: BytecodePost1630
}

impl ProcMetadata {
	pub fn get_bytecode(&self) -> misc::BytecodeId {
		static REDIRECT: OnceLock<fn(&ProcMetadata) -> misc::BytecodeId> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MINOR {
				..=1627 => Self::get_bytecode_pre1630,
				_ => Self::get_bytecode_post1630
			}
		})(self)
	}

	#[inline(never)]
	fn get_bytecode_pre1630(this: &Self) -> misc::BytecodeId {
		unsafe { this.pre1630.bytecode }
	}

	#[inline(never)]
	fn get_bytecode_post1630(this: &Self) -> misc::BytecodeId {
		unsafe { this.post1630.bytecode }
	}

	pub fn get_locals(&self) -> misc::LocalsId {
		static REDIRECT: OnceLock<fn(&ProcMetadata) -> misc::LocalsId> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MINOR {
				..=1627 => Self::get_locals_pre1630,
				_ => Self::get_locals_post1630
			}
		})(self)
	}

	#[inline(never)]
	fn get_locals_pre1630(this: &Self) -> misc::LocalsId {
		unsafe { this.pre1630.locals }
	}

	#[inline(never)]
	fn get_locals_post1630(this: &Self) -> misc::LocalsId {
		unsafe { this.post1630.locals }
	}

	pub fn get_parameters(&self) -> misc::ParametersId {
		static REDIRECT: OnceLock<fn(&ProcMetadata) -> misc::ParametersId> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MINOR {
				..=1627 => Self::get_parameters_pre1630,
				_ => Self::get_parameters_post1630
			}
		})(self)
	}

	#[inline(never)]
	fn get_parameters_pre1630(this: &Self) -> misc::ParametersId {
		unsafe { this.pre1630.parameters }
	}

	#[inline(never)]
	fn get_parameters_post1630(this: &Self) -> misc::ParametersId {
		unsafe { this.post1630.parameters }
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BytecodePre1630 {
	pub bytecode: misc::BytecodeId,
	pub locals: misc::LocalsId,
	pub parameters: misc::ParametersId
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BytecodePost1630 {
	unk_2: u32,
	pub bytecode: misc::BytecodeId,
	// Bytecode moved by 4 bytes in 1630
	pub locals: misc::LocalsId,
	pub parameters: misc::ParametersId
}

#[repr(C)]
pub struct ProcInstance {
	pub proc: ProcId,
	pub flags: u8,
	pub mega_hack: u16,
	pub usr: values::Value,
	pub src: values::Value,
	pub context: *mut ExecutionContext,
	argslist_idx: values::ValueData,
	unk_1: u32,
	unk_2: u32,
	inner: ProcInstanceInner
}

impl ProcInstance {
	#[inline(never)]
	fn args_count_pre516(this: &Self) -> u32 {
		unsafe { this.inner.pre516.args_count }
	}

	#[inline(never)]
	fn args_count_post516(this: &Self) -> u32 {
		unsafe { this.inner.post516.args_count }
	}

	pub fn args_count(&self) -> u32 {
		static REDIRECT: OnceLock<fn(&ProcInstance) -> u32> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MAJOR {
				..516 => Self::args_count_pre516,
				_ => Self::args_count_post516
			}
		})(self)
	}

	#[inline(never)]
	fn args_pre516(this: &Self) -> *mut values::Value {
		unsafe { this.inner.pre516.args }
	}

	#[inline(never)]
	fn args_post516(this: &Self) -> *mut values::Value {
		unsafe { this.inner.post516.args }
	}

	pub fn args(&self) -> *mut values::Value {
		static REDIRECT: OnceLock<fn(&ProcInstance) -> *mut values::Value> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MAJOR {
				..516 => Self::args_pre516,
				_ => Self::args_post516
			}
		})(self)
	}

	#[inline(never)]
	fn time_to_resume_pre516(this: &Self) -> u32 {
		unsafe { this.inner.pre516.time_to_resume }
	}

	#[inline(never)]
	fn time_to_resume_post516(this: &Self) -> u32 {
		unsafe { this.inner.post516.time_to_resume }
	}

	pub fn time_to_resume(&self) -> u32 {
		static REDIRECT: OnceLock<fn(&ProcInstance) -> u32> = OnceLock::new();
		REDIRECT.get_or_init(|| unsafe {
			match crate::version::BYOND_VERSION_MAJOR {
				..516 => Self::time_to_resume_pre516,
				_ => Self::time_to_resume_post516
			}
		})(self)
	}
}
#[repr(C)]
union ProcInstanceInner {
	pre516: ProcInstanceInnerPre516,
	post516: ProcInstanceInnerPost516
}

#[repr(C)]
#[derive(Copy, Clone)]
struct ProcInstanceInnerPre516 {
	args_count: u32,
	args: *mut values::Value,
	unk_3: [u8; 0x58],
	time_to_resume: u32
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct ProcInstanceInnerPost516 {
	unk_3: u32,
	args_count: u32,
	args: *mut values::Value,
	unk_4: [u8; 0x58],
	time_to_resume: u32
}

#[repr(C)]
pub struct ExecutionContext {
	pub proc_instance: *mut ProcInstance,
	pub parent_context: *mut ExecutionContext,
	pub filename: strings::StringId,
	pub line: u32,
	pub bytecode: *mut u32,
	pub bytecode_offset: u16,
	test_flag: u8,
	unk_0: u8,
	cached_datum: values::Value,
	unk_1: [u8; 0x10],
	pub dot: values::Value,
	pub locals: *mut values::Value,
	stack: *mut values::Value,
	pub locals_count: u16,
	stack_size: u16,
	unk_2: u32,
	current_iterator: *mut values::Value,
	iterator_allocated: u32,
	iterator_length: u32,
	iterator_index: u32,
	unk_3: u32,
	unk_4: [u8; 0x03],
	iterator_filtered_type: u8,
	unk_5: u8,
	unk_6: u8,
	unk_7: u8,
	infinite_loop_count: u32,
	unk_8: [u8; 0x02],
	paused: u8,
	unk_9: [u8; 0x33]
}

#[repr(C)]
pub struct SuspendedProcsBuffer {
	pub buffer: *mut *mut ProcInstance
}

#[repr(C)]
pub struct SuspendedProcs {
	pub front: usize,
	pub back: usize,
	pub capacity: usize
}
