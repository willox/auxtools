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

static mut PTR_REF_ID: u16 = 0x8000;

#[derive(PartialEq, Eq, Copy, Clone)]
struct ProcInstanceRef(u16);

impl ProcInstanceRef {
	fn new(ptr: *mut raw_types::procs::ProcInstance) -> Self {
		unsafe {
			PTR_REF_ID += 1;
			(*ptr).mega_hack = PTR_REF_ID;
			Self(PTR_REF_ID)
		}
	}

	fn is(&self, ptr: *mut raw_types::procs::ProcInstance) -> bool {
		unsafe { self.0 == (*ptr).mega_hack }
	}
}

// A lot of these store the parent ExecutionContext so we can tell if our proc has returned
// TODO: line/instruction variants
#[derive(Copy, Clone)]
enum DebuggerAction {
	None,
	Pause,
	StepOver { target: ProcInstanceRef },
	StepInto { parent: ProcInstanceRef },
	BreakOnNext,
	//StepOut{target: ExecutionContext},
}

static mut CURRENT_ACTION: DebuggerAction = DebuggerAction::None;

// TODO: Clear on shutdown
static mut DEFERRED_INSTRUCTION_REPLACE: RefCell<Option<(Vec<u32>, *mut u32)>> = RefCell::new(None);

#[derive(PartialEq, Eq, Hash)]
struct PtrKey(usize);

impl PtrKey {
	fn new(ptr: *mut u32) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}
}

lazy_static! {
	static ref ORIGINAL_BYTECODE: Mutex<HashMap<PtrKey, Vec<u32>>> = Mutex::new(HashMap::new());
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
			target: ProcInstanceRef::new(unsafe { (*ctx).proc_instance }),
		},
		ContinueKind::StepInto => DebuggerAction::StepInto {
			parent: ProcInstanceRef::new(unsafe { (*ctx).proc_instance }),
		},
		ContinueKind::StepOut => {
			unsafe {
				// Just continue the code if we've got no parent
				// Otherwise, treat this as a StepOver on the parent proc
				let parent = (*ctx).parent_context;
				if parent.is_null() {
					DebuggerAction::None
				} else {
					DebuggerAction::StepOver {
						target: ProcInstanceRef::new((*parent).proc_instance),
					}
				}
			}
		}
	}
}

fn proc_instance_is_in_stack(
	mut ctx: *mut raw_types::procs::ExecutionContext,
	proc_ref: ProcInstanceRef,
) -> bool {
	unsafe {
		let mut found = false;

		while !ctx.is_null() {
			if proc_ref.is((*ctx).proc_instance) {
				found = true;
				break;
			}
			ctx = (*ctx).parent_context;
		}

		found
	}
}

fn proc_instance_is_suspended(proc_ref: ProcInstanceRef) -> bool {
	unsafe {
		let procs = raw_types::funcs::SUSPENDED_PROCS;
		let buffer = (*procs).buffer;
		let front = (*procs).front;
		let back = (*procs).back;
		let mut found = false;

		for x in front..back {
			let instance = *buffer.add(x);

			if proc_instance_is_in_stack((*instance).context, proc_ref) {
				found = true;
				break;
			}
		}

		found
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
			std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
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

	let opcode_ptr = unsafe { (*ctx).bytecode.add((*ctx).bytecode_offset as usize) };
	let opcode = unsafe { *opcode_ptr };

	// This lets us ignore any actual breakpoints we hit if we've already paused for another reason
	let mut did_breakpoint = false;

	unsafe {
		match CURRENT_ACTION {
			DebuggerAction::None => {}

			DebuggerAction::Pause => {
				CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Pause);
				did_breakpoint = true;
			}

			DebuggerAction::BreakOnNext => {
				CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint);
				did_breakpoint = true;
			}

			// StepOver breaks on either of the following conditions:
			// 1) The target context has disappeared - this means it has returned or runtimed
			// 2) We're inside the target context and on a DbgLine instruction
			DebuggerAction::StepOver { target } => {
				if opcode == (OpCode::DbgLine as u32) && target.is((*ctx).proc_instance) {
					CURRENT_ACTION = DebuggerAction::BreakOnNext;
				} else {
					// If the context isn't in any stacks, it has just returned. Break!
					// TODO: Don't break if the context's stack is gone (returned to C)
					if !proc_instance_is_in_stack(ctx, target)
						&& !proc_instance_is_suspended(target)
					{
						CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
						did_breakpoint = true;
					}
				}
			}

			// StepInto breaks on any of the following conditions:
			// 1) The parent context has disappeared - this means it has returned or runtimed
			// 2) We're inside a context that is inside the parent context and on a DbgLine instruction
			// 3) We're inside the parent context and on a DbgLine instruction
			DebuggerAction::StepInto { parent } => {
				let is_dbgline = opcode == (OpCode::DbgLine as u32);

				if is_dbgline && parent.is((*ctx).proc_instance) {
					CURRENT_ACTION = DebuggerAction::BreakOnNext;
				} else {
					let in_stack = proc_instance_is_in_stack(ctx, parent);
					let is_suspended = proc_instance_is_suspended(parent);

					// If the context isn't in any stacks, it has just returned. Break!
					// TODO: Don't break if the context's stack is gone (returned to C)
					if !in_stack && !is_suspended {
						CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
						did_breakpoint = true;
					} else if in_stack && is_dbgline {
						CURRENT_ACTION = DebuggerAction::BreakOnNext;
					}
				}
			}
		}
	}

	if opcode == DEBUG_BREAK_OPCODE {
		// We don't want to break twice when stepping on to a breakpoint
		if !did_breakpoint {
			unsafe {
				CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint);
			}
		}

		// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been removed
		let map = ORIGINAL_BYTECODE.lock().unwrap();
		if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
			unsafe {
				assert_eq!(
					DEFERRED_INSTRUCTION_REPLACE.replace(Some((
						std::slice::from_raw_parts(opcode_ptr, original.len()).to_vec(),
						opcode_ptr
					))),
					None
				);
				std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
			}
		}
	}

	ctx
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset,
}

pub fn hook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionHookError> {
	let dism = proc.disassemble().0;

	let instruction = dism.iter().find(|x| x.0 == offset);

	if instruction.is_none() {
		return Err(InstructionHookError::InvalidOffset);
	}

	let instruction_length = instruction.unwrap().1 - instruction.unwrap().0 + 1;

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

	if opcode == DEBUG_BREAK_OPCODE {
		return Ok(());
	}

	unsafe {
		ORIGINAL_BYTECODE.lock().unwrap().insert(
			PtrKey::new(opcode_ptr),
			std::slice::from_raw_parts(opcode_ptr, instruction_length as usize).to_vec(),
		);
	}

	bytecode[offset as usize] = DEBUG_BREAK_OPCODE;
	for i in (offset + 1)..(offset + instruction_length) {
		bytecode[i as usize] = DEBUG_BREAK_OPERAND;
	}
	Ok(())
}

#[derive(Debug)]
pub enum InstructionUnhookError {
	InvalidOffset,
}

pub fn unhook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionUnhookError> {
	let dism = proc.disassemble().0;

	let instruction = dism.iter().find(|x| x.0 == offset);

	if instruction.is_none() {
		return Err(InstructionUnhookError::InvalidOffset);
	}

	let opcode_ptr = unsafe {
		let bytecode = {
			let (ptr, count) = proc.bytecode();
			std::slice::from_raw_parts_mut(ptr, count)
		};

		bytecode.as_mut_ptr().add(offset as usize)
	};

	// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been removed
	let mut map = ORIGINAL_BYTECODE.lock().unwrap();
	if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
		unsafe {
			// TODO: This check could fail once we add in runtime catching
			// The solution is to just remove the replace if it is for this instruction
			assert_eq!(DEFERRED_INSTRUCTION_REPLACE.borrow().as_ref(), None);
			std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
		}

		map.remove(&PtrKey::new(opcode_ptr));
	}

	Ok(())
}
