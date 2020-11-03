use std::{cell::RefCell, ffi::c_void};

use detour::RawDetour;
use dm::*;
use lazy_static::lazy_static;
use sigscan;
use std::sync::Mutex;
use vector_map::VecMap;

#[hook("/proc/install_instruction")]
fn hello_proc_hook() {
	let proc = Proc::find("/proc/test").unwrap();

	println!("{}", "Hello");

	hook_instruction(&proc, 11, |ctx| {
		let frames = CallStacks::new(ctx).active;
		let proc_name = format!("Proc: {:?}", frames[0].proc);

		println!("{}", proc_name);
	})
	.unwrap();

	Ok(Value::from(true))
}

static mut EXECUTE_INSTRUCTION: *const c_void = std::ptr::null();

extern "C" {
	// Trampoline to the original un-hooked BYOND execute_instruction code
	static mut execute_instruction_original: *const c_void;

	// Our version of execute_instruction. It hasn't got a calling convention rust knows about, so don't call it.
	fn execute_instruction_hook();
}

#[init(full)]
fn debug_server_init(_: &DMContext) -> Result<(), String> {
	let byondcore = sigscan::Scanner::for_module(BYONDCORE).unwrap();

	if cfg!(windows) {
		let ptr = byondcore
			.find(sigscan::signature!(
				"0F B7 48 ?? 8B 78 ?? 8B F1 8B 14 ?? 81 FA 59 01 00 00 0F 87 ?? ?? ?? ??"
			))
			.ok_or_else(|| "Couldn't find EXECUTE_INSTRUCTION")?;

		unsafe {
			EXECUTE_INSTRUCTION = ptr as *const c_void;
		}
	}

	if cfg!(unix) {
		let ptr = byondcore
			.find(sigscan::signature!(
				"0F B7 47 ?? 8B 57 ?? 0F B7 D8 8B 0C ?? 81 F9 59 01 00 00 77 ?? FF 24 8D ?? ?? ?? ??"
			))
			.ok_or_else(|| "Couldn't find EXECUTE_INSTRUCTION")?;

		unsafe {
			EXECUTE_INSTRUCTION = ptr as *const c_void;
		}
	}

	unsafe {
		let hook = RawDetour::new(
			EXECUTE_INSTRUCTION as *const (),
			execute_instruction_hook as *const (),
		)
		.map_err(|_| "Couldn't detour EXECUTE_INSTRUCTION")?;

		hook.enable()
			.map_err(|_| "Couldn't enable EXECUTE_INSTRUCTION detour")?;

		execute_instruction_original = std::mem::transmute(hook.trampoline());

		// We never remove or disable the hook, so just forget about it. (atm)
		std::mem::forget(hook);
	}

	Ok(())
}

// TODO: dm's disassembler can't handle this (o dear)
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

#[derive(PartialEq, Eq)]
struct PtrKey(usize);

impl PtrKey {
	fn new(ptr: *mut u32) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}
}

lazy_static! {
	static ref ORIGINAL_BYTECODE: Mutex<VecMap<PtrKey, u32>> = Mutex::new(VecMap::new());
	static ref INSTRUCTION_HOOKS: Mutex<VecMap<PtrKey, Box<dyn Fn(&DMContext) + Send + Sync>>> =
		Mutex::new(VecMap::new());
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure to return it.
#[no_mangle]
extern "C" fn handle_instruction(
	ctx: *mut raw_types::procs::ExecutionContext,
) -> *const raw_types::procs::ExecutionContext {
	// Always handle the deferred instruction replacement first - everything else will depend on it
	unsafe {
		let mut deferred = DEFERRED_INSTRUCTION_REPLACE.borrow_mut();
		if let Some((src, dst)) = &*deferred {
			**dst = *src;
			*deferred = None;
		}
	}

	let opcode_ptr = unsafe { (*ctx).bytecode.add((*ctx).bytecode_offset as usize) };

	let opcode = unsafe { *opcode_ptr };

	if opcode == CUSTOM_OPCODE {
		// Run the hook
		let dm_ctx = DMContext {};
		let map = INSTRUCTION_HOOKS
			.lock()
			.unwrap()
			.get(&PtrKey::new(opcode_ptr))
			.unwrap()(&dm_ctx);

		// Now run the original code
		let map = ORIGINAL_BYTECODE.lock().unwrap();
		let original = map.get(&PtrKey::new(opcode_ptr)).unwrap();
		unsafe {
			let current = *opcode_ptr;
			assert_eq!(
				DEFERRED_INSTRUCTION_REPLACE.replace(Some((current, opcode_ptr))),
				None
			);
			*opcode_ptr = *original;
		}
	}

	ctx
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset,
	AlreadyHooked,
}

pub fn hook_instruction<F>(proc: &Proc, offset: u32, hook: F) -> Result<(), InstructionHookError>
where
	F: 'static,
	F: Fn(&DMContext) + Send + Sync,
{
	let dism = proc.disassemble().0;
	if !dism.iter().any(|x| x.0 == offset) {
		return Err(InstructionHookError::InvalidOffset);
	}

	let bytecode;
	let opcode;
	let opcode_ptr;

	unsafe {
		bytecode = unsafe {
			let (ptr, count) = proc.bytecode();
			std::slice::from_raw_parts_mut(ptr, count)
		};

		opcode_ptr = bytecode.as_mut_ptr().add(offset as usize);
		opcode = *opcode_ptr;
	}

	if opcode == CUSTOM_OPCODE {
		return Err(InstructionHookError::AlreadyHooked);
	}

	ORIGINAL_BYTECODE
		.lock()
		.unwrap()
		.insert(PtrKey::new(opcode_ptr), opcode);
	INSTRUCTION_HOOKS
		.lock()
		.unwrap()
		.insert(PtrKey::new(opcode_ptr), Box::new(hook));

	bytecode[offset as usize] = CUSTOM_OPCODE;
	Ok(())
}
