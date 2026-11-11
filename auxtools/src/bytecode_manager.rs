// This module is in control of allocating buffers for use when replacing a
// proc's bytecode. To keep things sane, we also have to return the pointers to
// their original value before BYOND shuts down. We _also_ have to check if any
// existing procs are still using our bytecode when we shut down and leak the
// memory if so. It may be possible to avoid the leaks but it really doesn't
// matter.

use std::{
	cell::UnsafeCell,
	collections::{HashMap, HashSet},
	convert::TryFrom
};

use crate::{debug, raw_types, *};

static mut BYTECODE_ALLOCATIONS: UnsafeCell<Option<State>> = UnsafeCell::new(None);

struct State {
	allocations: HashSet<Vec<u32>>,
	original: HashMap<raw_types::procs::ProcId, (*mut u32, u16)>,
	// Retired bytecode buffers indexed by address. Active frames continue
	// executing the buffer they started with.
	remembered: HashMap<usize, Vec<u32>>
}

pub fn init() {
	unsafe {
		let ptr = BYTECODE_ALLOCATIONS.get();
		*ptr = Some(State {
			allocations: HashSet::new(),
			original: HashMap::new(),
			remembered: HashMap::new()
		});
	}
}

fn get_active_bytecode_ptrs() -> HashSet<*mut u32> {
	fn visit(dst: &mut HashSet<*mut u32>, frames: Vec<debug::StackFrame>) {
		for frame in frames {
			let ptr = unsafe { *(*frame.context).bytecode() };

			dst.insert(ptr);
		}
	}

	let stacks = debug::CallStacks::new();

	let mut ptrs = HashSet::new();
	visit(&mut ptrs, stacks.active);
	for stack in stacks.suspended {
		visit(&mut ptrs, stack);
	}

	ptrs
}

pub fn shutdown() {
	let active_ptrs = get_active_bytecode_ptrs();

	let state = unsafe { (*BYTECODE_ALLOCATIONS.get()).take().unwrap() };

	for (id, (ptr, len)) in state.original {
		let proc = Proc::from_id(id).unwrap();

		unsafe {
			raw_types::misc::set_bytecode(*(*proc.entry()).metadata.bytecode(), ptr, len);
		}
	}

	for mut vec in state.allocations {
		// If a proc with this bytecode is still running, just leak the mrmoy
		if active_ptrs.contains(&vec.as_mut_ptr()) {
			std::mem::forget(vec);
		}
	}
}

fn state() -> &'static mut State {
	unsafe { (*BYTECODE_ALLOCATIONS.get()).as_mut().unwrap() }
}

fn remember_original(state: &mut State, proc: &Proc) {
	state.original.entry(proc.id).or_insert_with(|| unsafe { proc.bytecode_mut_ptr() });
}

/// Keeps a copy of bytecode that a proc is about to stop using, for
/// [`bytecode_at`].
pub fn remember_bytecode(ptr: *const u32, bytecode: &[u32]) {
	state().remembered.entry(ptr as usize).or_insert_with(|| bytecode.to_vec());
}

/// Copy current or retained bytecode at `ptr`. Includes original code and
/// buffers replaced by reloads, which active frames may still execute.
pub fn bytecode_at(ptr: *const u32) -> Option<Vec<u32>> {
	let state = state();

	if let Some(bytecode) = state.remembered.get(&(ptr as usize)) {
		return Some(bytecode.clone());
	}

	if let Some(&(original, len)) = state.original.values().find(|(original, _)| std::ptr::eq(*original, ptr)) {
		return Some(unsafe { std::slice::from_raw_parts(original, len as usize) }.to_vec());
	}

	state.allocations.iter().find(|bytecode| bytecode.as_ptr() == ptr).cloned()
}

pub fn set_bytecode(proc: &Proc, mut bytecode: Vec<u32>) {
	let state = state();
	remember_original(state, proc);

	let (ptr, len) = {
		let len = bytecode.len();

		let ptr = match state.allocations.get(&bytecode) {
			Some(bytecode) => {
				bytecode.as_ptr() as *mut u32 // don't @ me
			}

			None => {
				let ptr = bytecode.as_mut_ptr();
				state.allocations.insert(bytecode);
				ptr
			}
		};

		(ptr, len)
	};

	let len = u16::try_from(len).unwrap();

	unsafe {
		raw_types::misc::set_bytecode(*(*proc.entry()).metadata.bytecode(), ptr, len);
	}
}
