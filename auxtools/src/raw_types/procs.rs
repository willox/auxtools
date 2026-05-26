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
	flags: u32,
	unk_1: u32,
	pub metadata: ProcMetadata
}

#[versioned(
	Pre1630 if crate::version::BYOND_VERSION_MINOR <= 1627,
	Post1630,
)]
#[repr(C)]
pub struct ProcMetadata {
	#[only_in(Post1630)]
	unk_2: u32,
	pub bytecode: misc::BytecodeId,
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
	pub argslist_idx: values::ValueData,
	unk_1: u32,
	unk_2: u32,
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
	#[only_in(Post516)]
	unk_pre: u32,
	pub args_count: u32,
	pub args: *mut values::Value,
	#[only_in(Pre516)]
	unk_3: [u8; 0x58],
	#[only_in(Post516)]
	unk_4: [u8; 0x58],
	pub time_to_resume: u32
}

#[versioned(
	Pre1668 if crate::version::BYOND_VERSION_MAJOR <= 515 || (crate::version::BYOND_VERSION_MAJOR == 516 && crate::version::BYOND_VERSION_MINOR <= 1667),
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
	unk_0: u8,
	cached_datum: values::Value,
	#[only_in(Pre1668)]
	unk_1_pre: [u8; 0x10],
	#[only_in(Post1668)]
	unk_1_post: [u8; 0x14],
	pub dot: values::Value,
	pub locals: *mut values::Value,
	pub stack: *mut values::Value,
	pub locals_count: u16,
	pub stack_size: u16,
	unk_2: u32,
	pub current_iterator: *mut values::Value,
	pub iterator_allocated: u32,
	pub iterator_length: u32,
	pub iterator_index: u32,
	unk_3: u32,
	unk_4: [u8; 0x03],
	pub iterator_filtered_type: u8,
	unk_5: u8,
	unk_6: u8,
	unk_7: u8,
	pub infinite_loop_count: u32,
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

#[cfg(test)]
mod layout_tests {
	use std::mem::{offset_of, size_of};

	use super::{misc, strings, values};

	// Reference structs: verbatim copies of the original hand-written variants,
	// kept here so that any accidental layout change in the macro output is caught.

	#[repr(C)]
	#[derive(Copy, Clone)]
	struct RefProcMetadataPre1630 {
		bytecode: misc::BytecodeId,
		locals: misc::LocalsId,
		parameters: misc::ParametersId
	}

	#[repr(C)]
	#[derive(Copy, Clone)]
	struct RefProcMetadataPost1630 {
		unk_2: u32,
		bytecode: misc::BytecodeId,
		locals: misc::LocalsId,
		parameters: misc::ParametersId
	}

	#[repr(C)]
	#[derive(Copy, Clone)]
	struct RefProcInstanceInnerPre516 {
		args_count: u32,
		args: *mut values::Value,
		unk_3: [u8; 0x58],
		time_to_resume: u32
	}

	#[repr(C)]
	#[derive(Copy, Clone)]
	struct RefProcInstanceInnerPost516 {
		unk_3: u32,
		args_count: u32,
		args: *mut values::Value,
		unk_4: [u8; 0x58],
		time_to_resume: u32
	}

	#[repr(C)]
	#[derive(Copy, Clone)]
	struct RefExecutionContextPre1668 {
		proc_instance: *mut super::ProcInstance,
		parent_context: *mut super::ExecutionContext,
		filename: strings::StringId,
		line: u32,
		bytecode: *mut u32,
		bytecode_offset: u16,
		test_flag: u8,
		unk_0: u8,
		cached_datum: values::Value,
		unk_1: [u8; 0x10],
		dot: values::Value,
		locals: *mut values::Value,
		stack: *mut values::Value,
		locals_count: u16,
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
	#[derive(Copy, Clone)]
	struct RefExecutionContextPost1668 {
		proc_instance: *mut super::ProcInstance,
		parent_context: *mut super::ExecutionContext,
		filename: strings::StringId,
		line: u32,
		bytecode: *mut u32,
		bytecode_offset: u16,
		test_flag: u8,
		unk_0: u8,
		cached_datum: values::Value,
		unk_1: [u8; 0x14],
		dot: values::Value,
		locals: *mut values::Value,
		stack: *mut values::Value,
		locals_count: u16,
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

	#[test]
	fn proc_metadata_pre1630() {
		type Gen = super::ProcMetadataPre1630;
		type Ref = RefProcMetadataPre1630;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, bytecode), offset_of!(Ref, bytecode));
		assert_eq!(offset_of!(Gen, locals), offset_of!(Ref, locals));
		assert_eq!(offset_of!(Gen, parameters), offset_of!(Ref, parameters));
	}

	#[test]
	fn proc_metadata_post1630() {
		type Gen = super::ProcMetadataPost1630;
		type Ref = RefProcMetadataPost1630;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, bytecode), offset_of!(Ref, bytecode));
		assert_eq!(offset_of!(Gen, locals), offset_of!(Ref, locals));
		assert_eq!(offset_of!(Gen, parameters), offset_of!(Ref, parameters));
	}

	#[test]
	fn proc_instance_inner_pre516() {
		type Gen = super::ProcInstanceInnerPre516;
		type Ref = RefProcInstanceInnerPre516;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, args_count), offset_of!(Ref, args_count));
		assert_eq!(offset_of!(Gen, args), offset_of!(Ref, args));
		assert_eq!(offset_of!(Gen, time_to_resume), offset_of!(Ref, time_to_resume));
	}

	#[test]
	fn proc_instance_inner_post516() {
		type Gen = super::ProcInstanceInnerPost516;
		type Ref = RefProcInstanceInnerPost516;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, args_count), offset_of!(Ref, args_count));
		assert_eq!(offset_of!(Gen, args), offset_of!(Ref, args));
		assert_eq!(offset_of!(Gen, time_to_resume), offset_of!(Ref, time_to_resume));
	}

	#[test]
	fn execution_context_pre1668() {
		type Gen = super::ExecutionContextPre1668;
		type Ref = RefExecutionContextPre1668;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, proc_instance), offset_of!(Ref, proc_instance));
		assert_eq!(offset_of!(Gen, filename), offset_of!(Ref, filename));
		assert_eq!(offset_of!(Gen, bytecode), offset_of!(Ref, bytecode));
		assert_eq!(offset_of!(Gen, dot), offset_of!(Ref, dot));
		assert_eq!(offset_of!(Gen, locals), offset_of!(Ref, locals));
		assert_eq!(offset_of!(Gen, locals_count), offset_of!(Ref, locals_count));
	}

	#[test]
	fn execution_context_post1668() {
		type Gen = super::ExecutionContextPost1668;
		type Ref = RefExecutionContextPost1668;
		assert_eq!(size_of::<Gen>(), size_of::<Ref>());
		assert_eq!(offset_of!(Gen, proc_instance), offset_of!(Ref, proc_instance));
		assert_eq!(offset_of!(Gen, filename), offset_of!(Ref, filename));
		assert_eq!(offset_of!(Gen, bytecode), offset_of!(Ref, bytecode));
		assert_eq!(offset_of!(Gen, dot), offset_of!(Ref, dot));
		assert_eq!(offset_of!(Gen, locals), offset_of!(Ref, locals));
		assert_eq!(offset_of!(Gen, locals_count), offset_of!(Ref, locals_count));
	}
}
