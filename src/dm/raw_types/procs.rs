use super::strings;
use super::values;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct ProcRef(pub u32);

#[repr(C)]
pub struct Proc {
	path: strings::StringRef,
	name: strings::StringRef,
	unk_0: u32,
	unk_1: u32,
	unk_2: u32,
	unk_3: u32,

	// TODO:
	bytecode: u32,
	locals: u32,
	misc: u32,
}

#[repr(C)]
pub struct ProcInstance {
	proc: ProcRef,
	unk_0: u32,
	usr: values::Value,
	src: values::Value,
	context: *mut ExecutionContext,
	argslist_idx: values::ValueData,
	unk_1: u32,
	unk_2: u32,
	arg_count: u32,
	args: *mut values::Value,
	unk_3: [u8; 0x58],
	time_to_resume: u32,
}

#[repr(C)]
pub struct ExecutionContext {
	proc_instance: *mut ProcInstance,
	parent_context: *mut ExecutionContext,
	dbg_proc_file: strings::StringRef,
	dbg_current_line: u32,
	bytecode: *mut u32,
	current_opcode: u16,
	test_flag: u8,
	unk_0: u8,
	cached_datum: values::Value,
	unk_1: [u8; 0x10],
	dot: values::Value,
	local_variables: *mut values::Value,
	stack: *mut values::Value,
	local_var_count: u16,
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
