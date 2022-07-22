use std::ffi::c_void;

use crate::disassemble_env::DisassembleEnv;
use crate::server_types::{BreakpointReason, ContinueKind};
use crate::CURRENT_EXECUTION_CONTEXT;
use crate::DEBUG_SERVER;
use crate::SUSPENDED_PROCS;
use crate::SUSPENDED_PROCS_BUFFER;
use auxtools::*;
use detour::RawDetour;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

// Could move these to dmasm
const OPCODE_DBGLINE: u32 = 0x85;
const OPCODE_DEBUG_BREAK: u32 = 0x1337;
const OPCODE_DEBUG_OPERAND: u32 = 0x1338;

static mut EXECUTE_INSTRUCTION: *const c_void = std::ptr::null();

extern "C" {
	// Trampoline to the original un-hooked BYOND execute_instruction code
	static mut execute_instruction_original: *const c_void;

	// Our version of execute_instruction. It hasn't got a calling convention rust knows about, so don't call it.
	fn execute_instruction_hook();
}

#[init(full)]
fn instruction_hooking_init() -> Result<(), String> {
	let byondcore = sigscan::Scanner::for_module(BYONDCORE).unwrap();

	if cfg!(windows) {
		let ptr = byondcore
			.find(signature!(
				"0F B7 48 ?? 8B 78 ?? 8B F1 8B 14 ?? 81 FA ?? ?? 00 00 0F 87 ?? ?? ?? ??"
			))
			.ok_or_else(|| "Couldn't find EXECUTE_INSTRUCTION")?;

		unsafe {
			EXECUTE_INSTRUCTION = ptr as *const c_void;
		}
	}

	if cfg!(unix) {
		let ptr = byondcore
			.find(signature!(
				"0F B7 47 ?? 8B 57 ?? 0F B7 D8 8B 0C ?? 81 F9 ?? ?? 00 00 77 ?? FF 24 8D ?? ?? ?? ??"
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

		// We never remove or disable the hook, so just forget about it.
		std::mem::forget(hook);
	}

	Ok(())
}

#[shutdown]
fn instruction_hooking_shutdown() {
	CURRENT_ACTION.with(|cell| cell.set(DebuggerAction::None));
	DEFERRED_INSTRUCTION_REPLACE.with(|cell| *cell.borrow_mut() = None);
	ORIGINAL_BYTECODE.with(|cell| cell.take());
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

// A lot of these store the parent ExecutionContext so we can tell if our proc has returned
#[derive(Copy, Clone)]
enum DebuggerAction {
	None,
	Pause,
	StepOver { target: ProcInstanceRef },
	StepInto { parent: ProcInstanceRef },
	BreakOnNext,
	StepOut { target: ProcInstanceRef },
}
thread_local! {
	static CURRENT_ACTION: Cell<DebuggerAction> = Cell::new(DebuggerAction::None);

	static DEFERRED_INSTRUCTION_REPLACE: RefCell<Option<(Vec<u32>, *mut u32)>> =
		RefCell::new(None);
}

#[derive(PartialEq, Eq, Hash)]
struct PtrKey(usize);

impl PtrKey {
	fn new(ptr: *mut u32) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}
}

thread_local! {
	static ORIGINAL_BYTECODE: RefCell<HashMap<PtrKey, Vec<u32>, fxhash::FxBuildHasher>> = Default::default();
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
		return unsafe { *CURRENT_EXECUTION_CONTEXT.with(|cell| cell.get()) };
	}

	unsafe {
		let buffer = (*SUSPENDED_PROCS_BUFFER.with(|cell| cell.get())).buffer;
		let procs = SUSPENDED_PROCS.with(|cell| cell.get());
		let front = (*procs).front;
		let back = (*procs).back;

		// bad default
		if back - front < stack_id as usize {
			return *CURRENT_EXECUTION_CONTEXT.with(|cell| cell.get());
		}

		let instance = *buffer.add(front + (stack_id - 1) as usize);
		(*instance).context
	}
}

fn handle_breakpoint(
	ctx: *mut raw_types::procs::ExecutionContext,
	reason: BreakpointReason,
) -> DebuggerAction {
	let action = DEBUG_SERVER.with(|cell| {
		let mut serber = cell.borrow_mut();
		match serber.as_mut() {
			Some(server) => server.handle_breakpoint(ctx, reason),
			None => ContinueKind::Continue,
		}
	});

	match action {
		ContinueKind::Continue => DebuggerAction::None,
		ContinueKind::StepOver { stack_id } => {
			let ctx = get_proc_ctx(stack_id);
			DebuggerAction::StepOver {
				target: ProcInstanceRef::new(unsafe { (*ctx).proc_instance }),
			}
		}
		ContinueKind::StepInto { stack_id } => {
			let ctx = get_proc_ctx(stack_id);
			DebuggerAction::StepInto {
				parent: ProcInstanceRef::new(unsafe { (*ctx).proc_instance }),
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
		let buffer = (*SUSPENDED_PROCS_BUFFER.with(|cell| cell.get())).buffer;
		let procs = SUSPENDED_PROCS.with(|cell| cell.get());
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
		let ctx = *CURRENT_EXECUTION_CONTEXT.with(|cell| cell.get());

		// If this is eval code, don't catch the breakpoint
		// TODO: Could try to make this work
		let is_eval_err = DEBUG_SERVER.with(|cell| -> Result<(), ()> {
			let mut serber = cell.borrow_mut();
			if let Some(server) = serber.as_mut() {
				if server.is_in_eval() {
					server.set_eval_error(error.into());
					return Err(());
				}
			}
			return Ok(());
		});

		if is_eval_err.is_err() {
			return;
		}

		CURRENT_ACTION.with(|cell| {
			cell.set(handle_breakpoint(
				ctx,
				BreakpointReason::Runtime(error.to_string()),
			))
		});
	}
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure to return it.
#[no_mangle]
extern "C" fn handle_instruction(
	ctx: *mut raw_types::procs::ExecutionContext,
) -> *const raw_types::procs::ExecutionContext {
	// Always handle the deferred instruction replacement first - everything else will depend on it
	DEFERRED_INSTRUCTION_REPLACE.with(|cell| {
		let mut deferred = cell.borrow_mut();
		if let Some((src, dst)) = &*deferred {
			unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len()) };
			*deferred = None;
		}
	});

	DEBUG_SERVER.with(|cell| {
		let mut serber = cell.borrow_mut();
		if let Some(server) = serber.as_mut() {
			if server.process() {
				CURRENT_ACTION.with(|cell| cell.set(DebuggerAction::Pause))
			}
		}
	});

	let opcode_ptr = unsafe { (*ctx).bytecode.add((*ctx).bytecode_offset as usize) };
	let opcode = unsafe { *opcode_ptr };

	// This lets us ignore any actual breakpoints we hit if we've already paused for another reason
	let mut did_breakpoint = false;

	CURRENT_ACTION.with(|cell| {
		match cell.get() {
			DebuggerAction::None => {}

			DebuggerAction::Pause => {
				cell.set(DebuggerAction::None);
				cell.set(handle_breakpoint(ctx, BreakpointReason::Pause));
				did_breakpoint = true;
			}

			DebuggerAction::BreakOnNext => {
				cell.set(DebuggerAction::None);
				cell.set(handle_breakpoint(ctx, BreakpointReason::Step));
				did_breakpoint = true;
			}

			// StepOver breaks on either of the following conditions:
			// 1) The target context has disappeared - this means it has returned or runtimed
			// 2) We're inside the target context and on a DbgLine instruction
			DebuggerAction::StepOver { target } => {
				if opcode == OPCODE_DBGLINE && unsafe { target.is((*ctx).proc_instance) } {
					cell.set(DebuggerAction::BreakOnNext);
				} else {
					// If the context isn't in any stacks, it has just returned. Break!
					// TODO: Don't break if the context's stack is gone (returned to C)
					if !proc_instance_is_in_stack(ctx, target)
						&& !proc_instance_is_suspended(target)
					{
						cell.set(DebuggerAction::None);
						cell.set(handle_breakpoint(ctx, BreakpointReason::Step));
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
					let is_dbgline = opcode == OPCODE_DBGLINE;
					let is_in_parent = unsafe { parent.is((*ctx).proc_instance) };

					if is_dbgline && is_in_parent {
						cell.set(DebuggerAction::BreakOnNext);
					} else if !is_in_parent {
						let in_stack = proc_instance_is_in_stack(ctx, parent);
						let is_suspended = proc_instance_is_suspended(parent);

						// If the context isn't in any stacks, it has just returned. Break!
						// TODO: Don't break if the context's stack is gone (returned to C)
						if !in_stack && !is_suspended {
							cell.set(DebuggerAction::None);
							cell.set(handle_breakpoint(ctx, BreakpointReason::Step));
							did_breakpoint = true;
						} else if in_stack && is_dbgline {
							cell.set(DebuggerAction::BreakOnNext);
						}
					}
				}
			}

			// Just breaks the moment we're in the target instance
			DebuggerAction::StepOut { target } => {
				if !is_generated_proc(ctx) {
					if unsafe { target.is((*ctx).proc_instance) } {
						cell.set(DebuggerAction::None);
						cell.set(handle_breakpoint(ctx, BreakpointReason::Step));
						did_breakpoint = true;
					} else {
						// If Our context disappeared, just stop the step
						let in_stack = proc_instance_is_in_stack(ctx, target);
						let is_suspended = proc_instance_is_suspended(target);

						if !in_stack && !is_suspended {
							cell.set(DebuggerAction::None);
						}
					}
				}
			}
		}

		if opcode == OPCODE_DEBUG_BREAK {
			// We don't want to break twice when stepping on to a breakpoint
			if !did_breakpoint {
				cell.set(DebuggerAction::None);
				cell.set(handle_breakpoint(ctx, BreakpointReason::Breakpoint));
			}

			// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been removed
			ORIGINAL_BYTECODE.with(|cell| {
				let map = cell.borrow_mut();
				if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
					DEFERRED_INSTRUCTION_REPLACE.with(|cell| {
						let mut deferred_replace = cell.borrow_mut();
						assert_eq!(*deferred_replace, None);
						unsafe {
							*deferred_replace = Some((
								std::slice::from_raw_parts(opcode_ptr, original.len()).to_vec(),
								opcode_ptr,
							));
							std::ptr::copy_nonoverlapping(
								original.as_ptr(),
								opcode_ptr,
								original.len(),
							);
						}
					});
				}
			});
		}
	});
	ctx
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset,
}

fn find_instruction<'a>(
	env: &'a mut DisassembleEnv,
	proc: &'a Proc,
	offset: u32,
) -> Option<(dmasm::Instruction, dmasm::DebugData<'a>)> {
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
	let mut env = crate::disassemble_env::DisassembleEnv;
	let (_, debug) =
		find_instruction(&mut env, proc, offset).ok_or(InstructionHookError::InvalidOffset)?;

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

	ORIGINAL_BYTECODE.with(|cell| {
		let mut map = cell.borrow_mut();
		map.insert(
			PtrKey::new(opcode_ptr),
			unsafe { std::slice::from_raw_parts(opcode_ptr, instruction_length as usize) }.to_vec(),
		);
	});

	bytecode[offset as usize] = OPCODE_DEBUG_BREAK;
	for i in (offset + 1)..(offset + instruction_length as u32) {
		bytecode[i as usize] = OPCODE_DEBUG_OPERAND;
	}
	Ok(())
}

#[derive(Debug)]
pub enum InstructionUnhookError {
	InvalidOffset,
}

pub fn unhook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionUnhookError> {
	let mut env = crate::disassemble_env::DisassembleEnv;
	let (_, _) =
		find_instruction(&mut env, proc, offset).ok_or(InstructionUnhookError::InvalidOffset)?;

	let opcode_ptr = unsafe {
		let bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		bytecode.as_mut_ptr().add(offset as usize)
	};

	// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already been removed
	ORIGINAL_BYTECODE.with(|map_cell| {
		let mut map = map_cell.borrow_mut();
		if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
			DEFERRED_INSTRUCTION_REPLACE.with(|cell| {
				let mut deferred = cell.borrow_mut();
				if let Some((_, dst)) = *deferred {
					if dst == opcode_ptr {
						*deferred = None;
					}
				}
				unsafe {
					std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len())
				};
			});

			map.remove(&PtrKey::new(opcode_ptr));
		}
	});

	Ok(())
}

pub fn get_hooked_offsets(proc: &Proc) -> Vec<u32> {
	let bytecode = unsafe { proc.bytecode() };

	let mut env = crate::disassemble_env::DisassembleEnv;
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
