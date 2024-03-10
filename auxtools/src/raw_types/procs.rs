use super::misc;
use super::strings;
use super::values;

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
    pub metadata: ProcMetadata,
}

pub union ProcMetadata {
    pub pre1630: BytecodePre1630,
    pub post1630: BytecodePost1630,
}

impl ProcMetadata {
    pub fn get_bytecode(&self) -> misc::BytecodeId {
        unsafe {
            return if crate::version::BYOND_VERSION_MINOR < 1630 {
                self.pre1630.bytecode
            } else {
                self.post1630.bytecode
            };
        }
    }

    pub fn get_locals(&self) -> misc::LocalsId {
        unsafe {
            return if crate::version::BYOND_VERSION_MINOR < 1630 {
                self.pre1630.locals
            } else {
                self.post1630.locals
            };
        }
    }

    pub fn get_parameters(&self) -> misc::ParametersId {
        unsafe {
            return if crate::version::BYOND_VERSION_MINOR < 1630 {
                self.pre1630.parameters
            } else {
                self.post1630.parameters
            };
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BytecodePre1630 {
    pub bytecode: misc::BytecodeId,
    pub locals: misc::LocalsId,
    pub parameters: misc::ParametersId,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BytecodePost1630 {
    unk_2: u32,
    pub bytecode: misc::BytecodeId,
    //Bytecode moved by 4 bytes in 1630
    pub locals: misc::LocalsId,
    pub parameters: misc::ParametersId,
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
	pub args_count: u32,
	pub args: *mut values::Value,
	unk_3: [u8; 0x58],
	pub time_to_resume: u32,
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
	unk_9: [u8; 0x33],
}

#[repr(C)]
pub struct SuspendedProcsBuffer {
	pub buffer: *mut *mut ProcInstance,
}

#[repr(C)]
pub struct SuspendedProcs {
	pub front: usize,
	pub back: usize,
	pub capacity: usize,
}
