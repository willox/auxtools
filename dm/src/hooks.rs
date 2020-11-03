use super::proc::Proc;
use super::raw_types;
use super::value::Value;
use super::DMContext;
use crate::raw_types::values::IntoRawValue;
use crate::runtime::DMResult;
use detour::RawDetour;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_void;

#[doc(hidden)]
pub struct CompileTimeHook {
	pub proc_path: &'static str,
	pub hook: ProcHook,
}

impl CompileTimeHook {
	pub fn new(proc_path: &'static str, hook: ProcHook) -> Self {
		CompileTimeHook { proc_path, hook }
	}
}

inventory::collect!(CompileTimeHook);

extern "C" {

	static mut execute_instruction_original: *const c_void;
	static mut call_proc_by_id_original: *const c_void;

	// Rust does not know the calling convention of this method (it has none), so don't call it!
	fn execute_instruction_hook();

	fn call_proc_by_id_original_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		unk_0: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_count_l: usize,
		unk_1: u32,
		unk_2: u32,
	) -> raw_types::values::Value;

	fn call_proc_by_id_hook_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		unk_0: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_count_l: usize,
		unk_1: u32,
		unk_2: u32,
	) -> raw_types::values::Value;
}

pub enum HookFailure {
	NotInitialized,
	ProcNotFound,
	AlreadyHooked,
	UnknownFailure,
}

impl std::fmt::Debug for HookFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInitialized => write!(f, "Library not initialized"),
			Self::ProcNotFound => write!(f, "Proc not found"),
			Self::AlreadyHooked => write!(f, "Proc is already hooked"),
			Self::UnknownFailure => write!(f, "Unknown failure"),
		}
	}
}

pub fn init() -> Result<(), String> {
	unsafe {
		let ins_hook = RawDetour::new(
			raw_types::funcs::EXECUTE_INSTRUCTION as *const (),
			execute_instruction_hook as *const (),
		)
		.unwrap();
		ins_hook.enable().unwrap();
		execute_instruction_original = std::mem::transmute(ins_hook.trampoline());
		std::mem::forget(ins_hook);

		//

		let call_hook = RawDetour::new(
			raw_types::funcs::call_proc_by_id_byond as *const (),
			call_proc_by_id_hook_trampoline as *const (),
		)
		.unwrap();

		call_hook.enable().unwrap();
		call_proc_by_id_original = std::mem::transmute(call_hook.trampoline());
		std::mem::forget(call_hook);
	}
	Ok(())
}

pub type ProcHook = fn(&DMContext, &Value, &Value, &mut Vec<Value>) -> DMResult;

thread_local! {
	static PROC_HOOKS: RefCell<HashMap<raw_types::procs::ProcId, ProcHook>> = RefCell::new(HashMap::new());
}

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook) -> Result<(), HookFailure> {
	PROC_HOOKS.with(|h| {
		let mut map = h.borrow_mut();
		match map.entry(id) {
			Entry::Vacant(v) => {
				v.insert(hook);
				Ok(())
			}
			Entry::Occupied(_) => Err(HookFailure::AlreadyHooked),
		}
	})
}

pub fn clear_hooks() {
	PROC_HOOKS.with(|h| h.borrow_mut().clear());
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook),
		None => Err(HookFailure::ProcNotFound),
	}
}

impl Proc {
	#[allow(unused)]
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		hook_by_id(self.id, func)
	}
}

static CUSTOM_OPCODE: u32 = 0x1337;

pub enum InstructionHandlerReturn {
	// Begin execution of the next instruction.
	Continue,

	// Run the given bytecode before the next.
	// Will panic if the bytecode is larger than the instruction being handled.
	Execute(Vec<u8>),
}

pub type InstructionHook = fn(&DMContext);

// TODO: Clear on shutdown
static mut DEFERRED_INSTRUCTION_REPLACE: RefCell<Option<(u32, *mut u32)>> = RefCell::new(None);

thread_local! {
	static ORIGINAL_BYTECODE: RefCell<HashMap<*mut u32, u32>> = RefCell::new(HashMap::new());
	static INSTRUCTION_HOOKS: RefCell<HashMap<*mut u32, Box<dyn Fn(&DMContext)>>> = RefCell::new(HashMap::new());
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure to return it.
#[no_mangle]
extern "C" fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) -> *const raw_types::procs::ExecutionContext {
	// Always handle the deferred instruction replacement first - everything else will depend on it
	unsafe {
		let mut deferred = DEFERRED_INSTRUCTION_REPLACE.borrow_mut();
		if let Some((src, dst)) = &*deferred {
			**dst = *src;
			*deferred = None;
		}
	}

	let opcode_ptr = unsafe {
		(*ctx).bytecode.add((*ctx).bytecode_offset as usize)
	};
	
	let opcode = unsafe {
		*opcode_ptr
	};

	if opcode == CUSTOM_OPCODE {
		// Run the hook
		INSTRUCTION_HOOKS.with(|x| {
			let map = x.borrow();
			let dm_ctx = DMContext {};
			map.get(&opcode_ptr).unwrap()(&dm_ctx);
		});

		// Now run the original code
		ORIGINAL_BYTECODE.with(|x| {
			let map = x.borrow();
			let original = map.get(&opcode_ptr).unwrap();
			unsafe {
				let current = *opcode_ptr;
				assert_eq!(DEFERRED_INSTRUCTION_REPLACE.replace(Some((current, opcode_ptr))), None);
				*opcode_ptr = *original;		
			}
		});
	}

	ctx
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset,
	AlreadyHooked
}

pub unsafe fn hook_instruction(proc: &Proc, offset: u32, hook: Box<dyn Fn(&DMContext)>) -> Result<u32, InstructionHookError> {
	let dism = proc.disassemble().0;
	if !dism.iter().any(|x| {
		x.0 == offset
	}) {
		return Err(InstructionHookError::InvalidOffset)
	}
	
	let bytecode = {
		let (ptr, count) = proc.bytecode();
		std::slice::from_raw_parts_mut(ptr, count)
	};

	let opcode_ptr = bytecode.as_mut_ptr().add(offset as usize);

	if *opcode_ptr == CUSTOM_OPCODE {
		return Err(InstructionHookError::AlreadyHooked);
	}

	ORIGINAL_BYTECODE.with(|x| {
		let mut map = x.borrow_mut();
		map.insert(opcode_ptr, *opcode_ptr);
	});

	INSTRUCTION_HOOKS.with(|x| {
		let mut map = x.borrow_mut();
		map.insert(opcode_ptr, hook);
	});

	bytecode[offset as usize] = CUSTOM_OPCODE;
	Ok(31)
}

#[no_mangle]
extern "C" fn call_proc_by_id_hook(
	usr_raw: raw_types::values::Value,
	proc_type: u32,
	proc_id: raw_types::procs::ProcId,
	unknown1: u32,
	src_raw: raw_types::values::Value,
	args_ptr: *mut raw_types::values::Value,
	num_args: usize,
	unknown2: u32,
	unknown3: u32,
) -> raw_types::values::Value {
	match PROC_HOOKS.with(|h| match h.borrow().get(&proc_id) {
		Some(hook) => {
			let ctx = unsafe { DMContext::new() };
			let src;
			let usr;
			let mut args: Vec<Value>;

			unsafe {
				src = Value::from_raw(src_raw);
				usr = Value::from_raw(usr_raw);

				// Taking ownership of args here
				args = std::slice::from_raw_parts(args_ptr, num_args)
					.iter()
					.map(|v| Value::from_raw_owned(*v))
					.collect();
			}

			let result = hook(&ctx, &src, &usr, &mut args);

			match result {
				Ok(r) => {
					let result_raw = unsafe { (&r).into_raw_value() };
					// Stealing our reference out of the Value
					std::mem::forget(r);
					Some(result_raw)
				}
				Err(e) => {
					// TODO: Some info about the hook would be useful (as the hook is never part of byond's stack, the runtime won't show it.)
					src.call("stack_trace", &[&Value::from_string(e.message.as_str())])
						.unwrap();
					unsafe { Some(Value::null().into_raw_value()) }
				}
			}
		}
		None => None,
	}) {
		Some(result) => result,
		None => unsafe {
			call_proc_by_id_original_trampoline(
				usr_raw, proc_type, proc_id, unknown1, src_raw, args_ptr, num_args, unknown2,
				unknown3,
			)
		},
	}
}
