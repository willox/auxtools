use std::{cell::UnsafeCell, collections::HashMap, sync::Mutex};

use auxtools::*;
use instruction_hooking::{
	disassemble_env::{self, DisassembleEnv},
	InstructionHook
};
use lazy_static::lazy_static;

use crate::{
	server::Server,
	server_types::{BreakpointReason, ContinueKind},
	DEBUG_SERVER
};

// Could move these to dmasm
const OPCODE_DBGLINE: u32 = 0x85;
const OPCODE_DEBUG_BREAK: u32 = 0x1337;
const OPCODE_DEBUG_OPERAND: u32 = 0x1338;

#[shutdown]
fn instruction_hooking_shutdown() {
	unsafe {
		CURRENT_ACTION = DebuggerAction::None;
		*DEFERRED_INSTRUCTION_REPLACE.get() = None;
		*ORIGINAL_BYTECODE.lock().unwrap() = HashMap::new();
	}
}

#[derive(PartialEq, Eq, Copy, Clone)]
struct ProcInstanceRef(u16);

impl ProcInstanceRef {
	fn new(ptr: *mut raw_types::procs::ProcInstance) -> Self {
		unsafe {
			static mut PTR_REF_ID: u16 = 0x8000;
			PTR_REF_ID += 1;
			(*ptr).mega_hack = PTR_REF_ID;
			Self(PTR_REF_ID)
		}
	}

	fn is(&self, ptr: *mut raw_types::procs::ProcInstance) -> bool {
		unsafe { self.0 == (*ptr).mega_hack }
	}
}

// A lot of these store the parent ExecutionContext so we can tell if our proc
// has returned
#[derive(Copy, Clone)]
enum DebuggerAction {
	None,
	Pause,
	StepOver { target: ProcInstanceRef },
	StepInto { parent: ProcInstanceRef },
	BreakOnNext,
	StepOut { target: ProcInstanceRef }
}

static mut CURRENT_ACTION: DebuggerAction = DebuggerAction::None;

static mut DEFERRED_INSTRUCTION_REPLACE: UnsafeCell<Option<(Vec<u32>, *mut u32)>> = UnsafeCell::new(None);

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

fn is_generated_proc(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		let instance = (*ctx).proc_instance;
		if let Some(proc) = Proc::from_id((*instance).proc) {
			return proc.path.ends_with("(init)");
		}
	}

	// worst-case just pretend it is generated
	true
}

fn get_proc_ctx(stack_id: u32) -> *mut raw_types::procs::ExecutionContext {
	if stack_id == 0 {
		return unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
	}

	unsafe {
		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let procs = raw_types::funcs::SUSPENDED_PROCS;
		let front = (*procs).front;
		let back = (*procs).back;

		// bad default
		if back - front < stack_id as usize {
			return *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
		}

		let instance = *buffer.add(front + (stack_id - 1) as usize);
		(*instance).context
	}
}

fn handle_breakpoint(ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> DebuggerAction {
	let action = unsafe {
		match &mut *DEBUG_SERVER.get() {
			Some(server) => server.handle_breakpoint(ctx, reason),
			None => ContinueKind::Continue
		}
	};

	match action {
		ContinueKind::Continue => DebuggerAction::None,
		ContinueKind::StepOver { stack_id } => {
			let ctx = get_proc_ctx(stack_id);
			DebuggerAction::StepOver {
				target: ProcInstanceRef::new(unsafe { (*ctx).proc_instance })
			}
		}
		ContinueKind::StepInto { stack_id } => {
			let ctx = get_proc_ctx(stack_id);
			DebuggerAction::StepInto {
				parent: ProcInstanceRef::new(unsafe { (*ctx).proc_instance })
			}
		}
		ContinueKind::StepOut { stack_id } => {
			unsafe {
				// Just continue the code if we've got no parent
				let ctx = get_proc_ctx(stack_id);
				let parent = (*ctx).parent_context;
				if parent.is_null() {
					DebuggerAction::None
				} else {
					DebuggerAction::StepOut {
						target: ProcInstanceRef::new((*parent).proc_instance)
					}
				}
			}
		}
	}
}

fn proc_instance_is_in_stack(mut ctx: *mut raw_types::procs::ExecutionContext, proc_ref: ProcInstanceRef) -> bool {
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
		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let procs = raw_types::funcs::SUSPENDED_PROCS;
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

#[runtime_handler]
fn handle_runtime(error: &str) {
	unsafe {
		let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;

		// If this is eval code, don't catch the breakpoint
		// TODO: Could try to make this work
		if let Some(server) = &mut *DEBUG_SERVER.get() {
			if server.is_in_eval() {
				server.set_eval_error(error.into());
				return;
			}
		}

		CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Runtime(error.to_string()));
	}
}

impl InstructionHook for Server {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		// Always handle the deferred instruction replacement first - everything else
		// will depend on it
		unsafe {
			let deferred = DEFERRED_INSTRUCTION_REPLACE.get();
			if let Some((src, dst)) = &*deferred {
				std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
				*deferred = None;
			}
		}

		let server = self;
		if server.process() {
			unsafe {
				CURRENT_ACTION = DebuggerAction::Pause;
			}
		}

		let opcode_ptr = unsafe { (*ctx).bytecode.add((*ctx).bytecode_offset as usize) };
		let opcode = unsafe { *opcode_ptr };
		let is_dbgline = opcode == OPCODE_DBGLINE;

		// This lets us ignore any actual breakpoints we hit if we've already paused for
		// another reason
		let mut did_breakpoint = false;

		unsafe {
			match CURRENT_ACTION {
				DebuggerAction::None => {}

				DebuggerAction::Pause => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Pause);
					did_breakpoint = true;
				}

				DebuggerAction::BreakOnNext => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
					did_breakpoint = true;
				}

				// StepOver breaks on either of the following conditions:
				// 1) The target context has disappeared - this means it has returned or runtimed
				// 2) We're inside the target context and on a DbgLine instruction
				DebuggerAction::StepOver { target } => {
					if is_dbgline && target.is((*ctx).proc_instance) {
						CURRENT_ACTION = DebuggerAction::BreakOnNext;
					} else {
						// If the context isn't in any stacks, it has just returned. Break!
						// TODO: Don't break if the context's stack is gone (returned to C)
						if !proc_instance_is_in_stack(ctx, target) && !proc_instance_is_suspended(target) {
							CURRENT_ACTION = DebuggerAction::None;
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
					if !is_generated_proc(ctx) {
						let is_in_parent = parent.is((*ctx).proc_instance);

						if is_dbgline && is_in_parent {
							CURRENT_ACTION = DebuggerAction::BreakOnNext;
						} else if !is_in_parent {
							let in_stack = proc_instance_is_in_stack(ctx, parent);
							let is_suspended = proc_instance_is_suspended(parent);

							// If the context isn't in any stacks, it has just returned. Break!
							// TODO: Don't break if the context's stack is gone (returned to C)
							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
								CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
								did_breakpoint = true;
							} else if in_stack && is_dbgline {
								CURRENT_ACTION = DebuggerAction::BreakOnNext;
							}
						}
					}
				}

				// Just breaks the moment we're in the target instance
				DebuggerAction::StepOut { target } => {
					if !is_generated_proc(ctx) {
						if target.is((*ctx).proc_instance) {
							CURRENT_ACTION = DebuggerAction::None;
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step);
							did_breakpoint = true;
						} else {
							// If Our context disappeared, just stop the step
							let in_stack = proc_instance_is_in_stack(ctx, target);
							let is_suspended = proc_instance_is_suspended(target);

							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
							}
						}
					}
				}
			}
		}

		if opcode == OPCODE_DEBUG_BREAK {
			// We don't want to break twice when stepping on to a breakpoint
			if !did_breakpoint {
				unsafe {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint);
				}
			}

			// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been
			// removed
			let map = ORIGINAL_BYTECODE.lock().unwrap();
			if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
				unsafe {
					let deferred_replace = DEFERRED_INSTRUCTION_REPLACE.get();
					assert_eq!(*deferred_replace, None);
					*deferred_replace = Some((std::slice::from_raw_parts(opcode_ptr, original.len()).to_vec(), opcode_ptr));
					std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
				}
			}
		}
	}
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset
}

fn find_instruction<'a>(env: &'a mut DisassembleEnv, proc: &'a Proc, offset: u32) -> Option<(dmasm::Instruction, dmasm::DebugData<'a>)> {
	let bytecode = unsafe { proc.bytecode() };

	let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, env);

	for node in nodes {
		if let dmasm::Node::Instruction(ins, debug) = node {
			if debug.offset == offset {
				return Some((ins, debug));
			}
		}
	}

	None
}

pub fn hook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionHookError> {
	let mut env = disassemble_env::DisassembleEnv;
	let (_, debug) = find_instruction(&mut env, proc, offset).ok_or(InstructionHookError::InvalidOffset)?;

	let instruction_length = debug.bytecode.len();

	let bytecode;
	let opcode;
	let opcode_ptr;

	unsafe {
		bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		opcode_ptr = bytecode.as_mut_ptr().add(offset as usize);
		opcode = *opcode_ptr;
	}

	if opcode == OPCODE_DEBUG_BREAK {
		return Ok(());
	}

	unsafe {
		ORIGINAL_BYTECODE.lock().unwrap().insert(
			PtrKey::new(opcode_ptr),
			std::slice::from_raw_parts(opcode_ptr, instruction_length).to_vec()
		);
	}

	bytecode[offset as usize] = OPCODE_DEBUG_BREAK;
	for i in (offset + 1)..(offset + instruction_length as u32) {
		bytecode[i as usize] = OPCODE_DEBUG_OPERAND;
	}
	Ok(())
}

#[derive(Debug)]
pub enum InstructionUnhookError {
	InvalidOffset
}

pub fn unhook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionUnhookError> {
	let mut env = disassemble_env::DisassembleEnv;
	let (..) = find_instruction(&mut env, proc, offset).ok_or(InstructionUnhookError::InvalidOffset)?;

	let opcode_ptr = unsafe {
		let bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		bytecode.as_mut_ptr().add(offset as usize)
	};

	// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been
	// removed
	let mut map = ORIGINAL_BYTECODE.lock().unwrap();
	if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
		unsafe {
			let deferred = DEFERRED_INSTRUCTION_REPLACE.get();
			if let Some((_, dst)) = *deferred {
				if dst == opcode_ptr {
					deferred.replace(None);
				}
			}
			std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
		}

		map.remove(&PtrKey::new(opcode_ptr));
	}

	Ok(())
}

pub fn get_hooked_offsets(proc: &Proc) -> Vec<u32> {
	let bytecode = unsafe { proc.bytecode() };

	let mut env = disassemble_env::DisassembleEnv;
	let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

	let mut offsets = vec![];

	for node in nodes {
		if let dmasm::Node::Instruction(ins, debug) = node {
			if ins == dmasm::Instruction::AuxtoolsDebugBreak {
				offsets.push(debug.offset);
			}
		}
	}

	offsets
}
