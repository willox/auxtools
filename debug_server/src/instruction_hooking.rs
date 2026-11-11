use std::{cell::UnsafeCell, collections::HashMap, sync::Mutex};

use auxtools::*;
use instruction_hooking::{disassemble_env, InstructionHook};
use lazy_static::lazy_static;

use crate::{
	server::Server,
	server_types::{BreakpointReason, ContinueKind},
	DEBUG_SERVER
};

// Could move these to dmasm
const OPCODE_DBGLINE: u32 = 0x85;
const OPCODE_THROW: u32 = 0x12D;
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
			// Every step takes a new id, so a long session does run out of them
			PTR_REF_ID = if PTR_REF_ID == u16::MAX { 0x8001 } else { PTR_REF_ID + 1 };
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
	/// Caller frame to stop in if the stepping proc returns.
	StepOver {
		target: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	StepInto {
		parent: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	BreakOnNext,
	StepOut {
		target: ProcInstanceRef
	},
	/// Stop at `target`'s next instruction, or its caller if it returns.
	StepOverInstruction {
		target: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	/// Stop at the next instruction in `parent` or a callee,
	/// or in its caller if it returns.
	StepIntoInstruction {
		parent: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	}
}

static mut CURRENT_ACTION: DebuggerAction = DebuggerAction::None;

/// When the pause being waited for was asked for
static mut PAUSE_ASKED: Option<std::time::Instant> = None;
/// Skip debugger helpers for this long before pausing in one
/// when no world code runs.
const PAUSE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(1);

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
		let instance = *(*ctx).proc_instance();
		if let Some(proc) = Proc::from_id((*instance).proc) {
			return proc.path.ends_with("(init)");
		}
	}

	// worst-case just pretend it is generated
	true
}

/// Resolve the target frame and its caller for stepping.
/// Stack 0 is running; other stacks are sleeping procs.
fn get_proc_ctx(stack_id: u32) -> (*mut raw_types::procs::ExecutionContext, *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let running = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
		let running = (running, *(*running).parent_context());
		if stack_id == 0 {
			return running;
		}

		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let std::ops::Range { start: front, end: back } = raw_types::funcs::suspended_range();

		// bad default
		if back - front < stack_id as usize {
			return running;
		}

		// Sleeping stacks link from the outermost frame toward the innermost;
		// BYOND's `parent` field points to the callee.
		let instance = *buffer.add(front + (stack_id - 1) as usize);
		let mut caller = std::ptr::null_mut();
		let mut ctx = (*instance).context;
		loop {
			let called = *(*ctx).parent_context();
			if called.is_null() {
				return (ctx, caller);
			}
			caller = ctx;
			ctx = called;
		}
	}
}

/// Marks the proc instance of a frame, if there is one
unsafe fn mark(ctx: *mut raw_types::procs::ExecutionContext) -> Option<ProcInstanceRef> {
	if ctx.is_null() {
		None
	} else {
		Some(ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }))
	}
}

/// Mark the caller as a step target, unless it is a hidden debugger helper.
unsafe fn mark_caller(ctx: *mut raw_types::procs::ExecutionContext) -> Option<ProcInstanceRef> {
	if ctx.is_null() || Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc }).is_some_and(|proc| crate::launched::runs_spawns(&proc)) {
		return None;
	}
	unsafe { mark(ctx) }
}

/// Pause if a client requests it. Return the resume action, or `None`
/// if execution was not paused.
fn handle_breakpoint(ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<DebuggerAction> {
	let action = unsafe {
		match &mut *DEBUG_SERVER.get() {
			Some(server) => server.handle_breakpoint(ctx, reason)?,
			None => return None
		}
	};

	Some(match action {
		ContinueKind::Continue => DebuggerAction::None,
		ContinueKind::StepOver { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepOver {
				target: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepInto { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepInto {
				parent: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepOverInstruction { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepOverInstruction {
				target: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepIntoInstruction { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepIntoInstruction {
				parent: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepOut { stack_id } => {
			// Just continue the code if nothing called it
			let (_, caller) = get_proc_ctx(stack_id);
			match unsafe { mark_caller(caller) } {
				Some(target) => DebuggerAction::StepOut { target },
				None => {
					step_ended();
					DebuggerAction::None
				}
			}
		}
	})
}

/// Notify the stepping client when its stack ends without pausing,
/// so it does not wait indefinitely for a stop event.
fn step_ended() {
	unsafe {
		if let Some(server) = &mut *DEBUG_SERVER.get() {
			server.step_ended();
		}
	}
}

fn proc_instance_is_in_stack(mut ctx: *mut raw_types::procs::ExecutionContext, proc_ref: ProcInstanceRef) -> bool {
	unsafe {
		let mut found = false;

		while !ctx.is_null() {
			if proc_ref.is(*(*ctx).proc_instance()) {
				found = true;
				break;
			}
			ctx = *(*ctx).parent_context();
		}

		found
	}
}

fn proc_instance_is_suspended(proc_ref: ProcInstanceRef) -> bool {
	unsafe {
		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let mut found = false;

		for x in raw_types::funcs::suspended_range() {
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
	// BYOND calls this through extern "C" code, and a panic that gets back out
	// there aborts the whole game
	let result = std::panic::catch_unwind(|| unsafe {
		let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;

		// If this is eval code, don't catch the breakpoint
		// TODO: Could try to make this work
		if let Some(server) = &mut *DEBUG_SERVER.get() {
			if server.is_in_eval() {
				// Errors caught within the evaluated code remain its
				// responsibility.
				if !in_try_above_expression(ctx) {
					server.set_eval_error(error, ctx);
				}
				return;
			}
		}

		// Spawned code's runtime is what it gives, as well as the world's
		if let Some(server) = &mut *DEBUG_SERVER.get() {
			server.spawn_runtime(error, ctx);
		}

		// One nobody pauses for leaves a step in progress as it was
		let reason = BreakpointReason::Runtime {
			message: error.to_string(),
			caught: in_try(ctx)
		};
		if let Some(action) = handle_breakpoint(ctx, reason) {
			CURRENT_ACTION = action;
		}
	});

	if let Err(payload) = result {
		if let Some(server) = unsafe { &mut *DEBUG_SERVER.get() } {
			crate::recover(&*payload, server);
		}
	}
}

/// Whether this frame or a caller has a `try` that catches the error.
unsafe fn in_try(mut ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		while !ctx.is_null() {
			if *(*ctx).throw_handler() != 0 {
				return true;
			}
			ctx = *(*ctx).parent_context();
		}
	}
	false
}

/// Whether the evaluated code catches the error before it reaches the
/// expression stub. The paused world's surrounding `try` blocks do not apply.
unsafe fn in_try_above_expression(mut ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		while !ctx.is_null() {
			if crate::server::is_expression_frame(ctx) {
				return false;
			}
			if *(*ctx).throw_handler() != 0 {
				return true;
			}
			ctx = *(*ctx).parent_context();
		}
	}
	false
}

/// Read the pending thrown value as an error message, using an exception's
/// name when available.
unsafe fn thrown(ctx: *mut raw_types::procs::ExecutionContext) -> String {
	let size = unsafe { *(*ctx).stack_size() } as usize;
	if size == 0 {
		return "an exception".to_owned();
	}
	let value = unsafe { Value::from_raw(*(*(*ctx).stack()).add(size - 1)) };
	if let Ok(name) = value.get_string(byond_string!("name")) {
		return name;
	}
	match value.as_string() {
		Ok(text) => text,
		Err(_) => value.to_string().unwrap_or_else(|_| "an exception".to_owned())
	}
}

/// Whether the instruction about to run in a frame has a breakpoint on it
pub(crate) fn is_breakpoint(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe { *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize) == OPCODE_DEBUG_BREAK }
}

/// Whether this is a debugger helper proc, such as the keepalive loop.
unsafe fn in_helper(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	Proc::from_id(id).is_some_and(|proc| proc.path.trim_start_matches("/proc").starts_with("/auxtools_"))
}

impl InstructionHook for Server {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		// Always handle the deferred instruction replacement first - everything
		// else will depend on it
		unsafe {
			let deferred = DEFERRED_INSTRUCTION_REPLACE.get();
			if let Some((src, dst)) = &*deferred {
				std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
				*deferred = None;
			}
		}

		// An expression the debugger is evaluating runs through here too.
		// Pausing in the middle of it would start a second pause inside the
		// one that's waiting on the answer, so all it gets is its breakpoints
		// stepped over.
		let server = self;
		let in_eval = server.is_in_eval();
		// Track the frame in case evaluation is interrupted by sleep.
		if in_eval {
			server.eval_runs(ctx);
		}

		if !in_eval && server.process() {
			unsafe {
				CURRENT_ACTION = DebuggerAction::Pause;
				PAUSE_ASKED = Some(std::time::Instant::now());
			}
		}

		let opcode_ptr = unsafe { (*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize) };
		let opcode = unsafe { *opcode_ptr };
		let is_dbgline = opcode == OPCODE_DBGLINE;

		// This lets us ignore any actual breakpoints we hit if we've already
		// paused for another reason
		let mut did_breakpoint = false;

		// A watched place that changed in the instruction before this one
		if !in_eval && server.has_watches() {
			if let Some(changes) = server.watched_changes(ctx) {
				unsafe {
					// A step in progress stays as it was if nobody pauses
					if let Some(action) = handle_breakpoint(ctx, BreakpointReason::Data { changes }) {
						CURRENT_ACTION = action;
						did_breakpoint = true;
					}
				}
			}
		}

		unsafe {
			let action = if in_eval { DebuggerAction::None } else { CURRENT_ACTION };

			match action {
				DebuggerAction::None => {}

				DebuggerAction::Pause if PAUSE_ASKED.is_some_and(|asked| asked.elapsed() < PAUSE_PATIENCE) && in_helper(ctx) => {}

				DebuggerAction::Pause => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Pause).unwrap_or(DebuggerAction::None);
					did_breakpoint = true;
				}

				DebuggerAction::BreakOnNext => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
					did_breakpoint = true;
				}

				// StepOver breaks on either of the following conditions:
				// 1) The target context has disappeared (it has returned or
				//    runtimed) and we're in its caller
				// 2) We're inside the target context and on a DbgLine
				//    instruction
				DebuggerAction::StepOver { target, caller } => {
					if is_dbgline && target.is(*(*ctx).proc_instance()) {
						CURRENT_ACTION = DebuggerAction::BreakOnNext;
					} else if !proc_instance_is_in_stack(ctx, target) && !proc_instance_is_suspended(target) {
						// If the target ended, stop in its caller. If execution
						// belongs to another stack, end the step without
						// pausing.
						CURRENT_ACTION = DebuggerAction::None;
						if caller.is_some_and(|caller| caller.is(*(*ctx).proc_instance())) {
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							step_ended();
						}
					}
				}

				// StepInto breaks on any of the following conditions:
				// 1) The parent context has disappeared (it has returned or
				//    runtimed) and we're in its caller
				// 2) We're inside a context that is inside the parent context
				//    and on a DbgLine instruction
				// 3) We're inside the parent context and on a DbgLine
				//    instruction
				DebuggerAction::StepInto { parent, caller } => {
					if !is_generated_proc(ctx) {
						let is_in_parent = parent.is(*(*ctx).proc_instance());

						if is_dbgline && is_in_parent {
							CURRENT_ACTION = DebuggerAction::BreakOnNext;
						} else if !is_in_parent {
							let in_stack = proc_instance_is_in_stack(ctx, parent);
							let is_suspended = proc_instance_is_suspended(parent);

							// The target returned. Stop in its caller, or end
							// the step without pausing if this is another
							// stack.
							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
								if caller.is_some_and(|caller| caller.is(*(*ctx).proc_instance())) {
									CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
									did_breakpoint = true;
								} else {
									step_ended();
								}
							} else if in_stack && is_dbgline {
								CURRENT_ACTION = DebuggerAction::BreakOnNext;
							}
						}
					}
				}

				// Instruction stepping ignores other stacks while the target
				// sleeps. Step Into also enters callees; both modes stop in the
				// caller if the target returns.
				DebuggerAction::StepOverInstruction { target: frame, caller } | DebuggerAction::StepIntoInstruction { parent: frame, caller } => {
					let into = matches!(action, DebuggerAction::StepIntoInstruction { .. });
					let here = frame.is(*(*ctx).proc_instance()) || (into && proc_instance_is_in_stack(ctx, frame));
					let over = !here && !proc_instance_is_in_stack(ctx, frame) && !proc_instance_is_suspended(frame);
					if here || over {
						CURRENT_ACTION = DebuggerAction::None;
						if here || caller.is_some_and(|caller| caller.is(*(*ctx).proc_instance())) {
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							step_ended();
						}
					}
				}

				// Just breaks the moment we're in the target instance
				DebuggerAction::StepOut { target } => {
					if !is_generated_proc(ctx) {
						if target.is(*(*ctx).proc_instance()) {
							CURRENT_ACTION = DebuggerAction::None;
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							// If Our context disappeared, just stop the step
							let in_stack = proc_instance_is_in_stack(ctx, target);
							let is_suspended = proc_instance_is_suspended(target);

							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
								step_ended();
							}
						}
					}
				}
			}
		}

		// Intercept every `throw` before execution because BYOND does not
		// report it through the runtime error hook.
		if opcode == OPCODE_THROW && !did_breakpoint && !in_eval {
			unsafe {
				let reason = BreakpointReason::Runtime {
					message: thrown(ctx),
					caught: in_try(ctx)
				};
				if let Some(action) = handle_breakpoint(ctx, reason) {
					CURRENT_ACTION = action;
				}
			}
		}

		if opcode == OPCODE_DEBUG_BREAK {
			// We don't want to break twice when stepping on to a breakpoint
			if !did_breakpoint && !in_eval {
				unsafe {
					// A breakpoint that doesn't pause (its condition is false)
					// leaves a step in progress as it was
					let stepping = CURRENT_ACTION;
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint).unwrap_or(stepping);
				}
			}

			// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has
			// already been removed
			let map = ORIGINAL_BYTECODE.lock().unwrap();
			if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
				unsafe {
					// Execute the original instruction without re-entering the
					// hook. If it is DbgLine, notify line stepping now so it
					// does not skip the line following this breakpoint.
					if original.first() == Some(&OPCODE_DBGLINE) && !in_eval {
						match CURRENT_ACTION {
							DebuggerAction::StepOver { target: frame, .. } | DebuggerAction::StepInto { parent: frame, .. }
								if frame.is(*(*ctx).proc_instance()) =>
							{
								CURRENT_ACTION = DebuggerAction::BreakOnNext;
							}
							_ => {}
						}
					}

					// The breakpoint we stepped over last is normally back in
					// place by now. It isn't if an eval ended on that very
					// instruction, so put it back before we lose track of it.
					let deferred_replace = DEFERRED_INSTRUCTION_REPLACE.get();
					if let Some((src, dst)) = &*deferred_replace {
						std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
					}
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

/// Read instruction width at `offset` from unpatched bytecode.
/// Breakpoint operands would otherwise be misread as opcodes.
fn instruction_length(proc: &Proc, offset: u32) -> Option<usize> {
	let bytecode = original_bytecode(proc);
	let mut env = disassemble_env::DisassembleEnv;
	let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

	nodes.into_iter().find_map(|node| match node {
		dmasm::Node::Instruction(_, debug) if debug.offset == offset => Some(debug.bytecode.len()),
		_ => None
	})
}

pub fn hook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionHookError> {
	let instruction_length = instruction_length(proc, offset).ok_or(InstructionHookError::InvalidOffset)?;

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
	instruction_length(proc, offset).ok_or(InstructionUnhookError::InvalidOffset)?;

	let opcode_ptr = unsafe {
		let bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		bytecode.as_mut_ptr().add(offset as usize)
	};

	// ORIGINAL_BYTECODE won't contain an entry if this breakpoint has already
	// been removed
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

/// Takes every breakpoint back out of the bytecode and forgets any step in
/// progress.
pub fn unhook_all() {
	unsafe {
		CURRENT_ACTION = DebuggerAction::None;
		*DEFERRED_INSTRUCTION_REPLACE.get() = None;
	}

	for (opcode_ptr, original) in ORIGINAL_BYTECODE.lock().unwrap().drain() {
		unsafe {
			std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr.0 as *mut u32, original.len());
		}
	}
}

/// Where the proc's breakpoints are.
pub fn hooked_offsets(proc: &Proc) -> Vec<u32> {
	let (start, count) = unsafe { proc.bytecode_mut_ptr() };
	let start = start as usize;
	let end = start + count as usize * std::mem::size_of::<u32>();

	let mut offsets: Vec<u32> = ORIGINAL_BYTECODE
		.lock()
		.unwrap()
		.keys()
		.filter(|ptr| (start..end).contains(&ptr.0))
		.map(|ptr| ((ptr.0 - start) / std::mem::size_of::<u32>()) as u32)
		.collect();
	offsets.sort_unstable();
	offsets
}

/// The proc's bytecode without any of our breakpoints patched into it.
pub fn original_bytecode(proc: &Proc) -> Vec<u32> {
	let (start, count) = unsafe { proc.bytecode_mut_ptr() };
	let bytecode = unsafe { std::slice::from_raw_parts(start, count as usize) }.to_vec();
	unpatched(start, bytecode)
}

/// Bytecode read from `start`, without any of our breakpoints patched into it
pub fn unpatched(start: *const u32, mut bytecode: Vec<u32>) -> Vec<u32> {
	for (opcode_ptr, original) in ORIGINAL_BYTECODE.lock().unwrap().iter() {
		// The map holds every proc's breakpoints, and most aren't in this one
		let Some(offset) = opcode_ptr.0.checked_sub(start as usize).map(|bytes| bytes / std::mem::size_of::<u32>()) else {
			continue;
		};

		if let Some(hooked) = bytecode.get_mut(offset..offset + original.len()) {
			hooked.copy_from_slice(original);
		}
	}

	bytecode
}
