use std::{cell::RefCell, ffi::c_void};

use crate::server_types::{BreakpointReason, ContinueKind};
use crate::DEBUG_SERVER;
use detour::RawDetour;
use dm::*;
use lazy_static::lazy_static;
use sigscan;
use std::collections::HashMap;
use std::sync::Mutex;

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

#[derive(PartialEq, Eq, Copy, Clone)]
struct ExecutionContextRef(usize);

impl ExecutionContextRef {
	fn new(ptr: *mut raw_types::procs::ExecutionContext) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}

	fn get(&self) -> *mut raw_types::procs::ExecutionContext {
		return self.0 as *mut _;
	}
}

// A lot of these store the parent ExecutionContext so we can tell if our proc has returned
// TODO: line/instruction variants
#[derive(Copy, Clone)]
enum DebuggerAction {
	None,
	Pause,
	//StepInto{parent: ExecutionContextRef},
	StepOver { target: ExecutionContextRef },
	//StepOut{target: ExecutionContext},
}

static mut CURRENT_ACTION: DebuggerAction = DebuggerAction::None;

// TODO: dm's disassembler can't handle this (o dear)
static CUSTOM_OPCODE: u32 = 0x1337;
// static CUSTOM_OPCODE_WITH_OPERANDS: u32 = 0x1338;
// static CUSTOM_OPCODE_WITH_OPERAND_MARKER: u32 = 0x1339;

// TODO: Clear on shutdown
static mut DEFERRED_INSTRUCTION_REPLACE: RefCell<Option<(u32, *mut u32)>> = RefCell::new(None);

#[derive(PartialEq, Eq, Hash)]
struct PtrKey(usize);

impl PtrKey {
	fn new(ptr: *mut u32) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}
}

lazy_static! {
	static ref ORIGINAL_BYTECODE: Mutex<HashMap<PtrKey, u32>> = Mutex::new(HashMap::new());
}

fn handle_breakpoint(
	ctx: *mut raw_types::procs::ExecutionContext,
	reason: BreakpointReason,
) -> DebuggerAction {
	let action = DEBUG_SERVER.with(|x| {
		let mut server = x.borrow_mut();
		server.as_mut().unwrap().handle_breakpoint(ctx, reason)
	});

	match action {
		ContinueKind::Continue => DebuggerAction::None,
		ContinueKind::StepOver => DebuggerAction::StepOver {
			target: ExecutionContextRef::new(ctx),
		},
	}
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

	DEBUG_SERVER.with(|x| {
		let mut server = x.borrow_mut();
		if let Some(server) = server.as_mut() {
			if server.process() {
				unsafe {
					CURRENT_ACTION = DebuggerAction::Pause;
				}
			}
		}
	});

	// This lets us ignore any actual breakpoints we hit if we've already paused for another reason
	let mut did_breakpoint = false;

	unsafe {
		match CURRENT_ACTION {
			DebuggerAction::None => {}

			DebuggerAction::Pause => {
				CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Pause);
				did_breakpoint = true;
			}

			DebuggerAction::StepOver { target } => {
				// If we're in the context, break!
				if ctx == target.get() {
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
					did_breakpoint = true;
				} else {
					// If the context isn't in any stacks, it has just returned. Break!
					let context_in_stack = {
						let mut current = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
						let mut found = false;

						while !current.is_null() {
							if current == target.get() {
								found = true;
								break;
							}
							current = (*current).parent_context;
						}

						found
					};

					let context_suspended = {
						let procs = raw_types::funcs::SUSPENDED_PROCS;
						let buffer = (*procs).buffer;
						let front = (*procs).front;
						let back = (*procs).back;
						let mut found = false;

						'outer: for x in front..back {
							let instance = *buffer.add(x);
							let mut current = (*instance).context;

							while !current.is_null() {
								if current == target.get() {
									found = true;
									break 'outer;
								}
								current = (*current).parent_context;
							}
						}

						found
					};

					// TODO: Detect break when we've returned outside of our context
					if !context_in_stack && !context_suspended {
						CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
						did_breakpoint = true;
					}
				}
			}
		}
	}

	let opcode_ptr = unsafe { (*ctx).bytecode.add((*ctx).bytecode_offset as usize) };

	let opcode = unsafe { *opcode_ptr };

	if opcode == CUSTOM_OPCODE {
		// We don't want to break twice when stepping on to a breakpoint
		if !did_breakpoint {
			unsafe {
				CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint);
			}
		}

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

pub fn hook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionHookError> {
	let dism = proc.disassemble().0;
	if !dism.iter().any(|x| x.0 == offset) {
		return Err(InstructionHookError::InvalidOffset);
	}

	let bytecode;
	let opcode;
	let opcode_ptr;

	unsafe {
		bytecode = {
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

	bytecode[offset as usize] = CUSTOM_OPCODE;
	Ok(())
}

#[derive(Debug)]
pub enum InstructionUnhookError {
	InvalidOffset,
}

// TODO: this won't work until the disassembler can handle our custom bytecode
pub fn unhook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionUnhookError> {
	let dism = proc.disassemble().0;
	if !dism.iter().any(|x| x.0 == offset) {
		return Err(InstructionUnhookError::InvalidOffset);
	}

	Ok(())
}
