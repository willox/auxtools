// This module is in control of allocating buffers for use when replacing a proc's bytecode.
// To keep things sane, we also have to return the pointers to their original value before BYOND shuts down.
// We _also_ have to check if any existing procs are still using our bytecode when we shut down and leak the memory if so.
// It may be possible to avoid the leaks but it really doesn't matter.

use crate::{debug, raw_types, *};
use std::convert::TryFrom;
use std::{
	cell::RefCell,
	collections::{HashMap, HashSet},
};

use ahash::RandomState;
use fxhash::FxBuildHasher;

thread_local! {
	static BYTECODE_ALLOCATIONS: RefCell<Option<State>> = RefCell::new(None);
}

struct State {
	allocations: HashSet<Vec<u32>, RandomState>,
	original: HashMap<raw_types::procs::ProcId, (*mut u32, u16), FxBuildHasher>,
}

pub fn init() {
	BYTECODE_ALLOCATIONS.with(|cell| {
		*cell.borrow_mut() = Some(State {
			allocations: Default::default(),
			original: Default::default(),
		});
	})
}

fn get_active_bytecode_ptrs() -> HashSet<*mut u32> {
	fn visit(dst: &mut HashSet<*mut u32>, frames: Vec<debug::StackFrame>) {
		for frame in frames {
			let ptr = unsafe { (*frame.context).bytecode };

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

	let state = BYTECODE_ALLOCATIONS.with(|cell| cell.borrow_mut().take().unwrap());

	for (id, (ptr, len)) in state.original {
		let proc = Proc::from_id(id).unwrap();

		unsafe {
			raw_types::misc::set_bytecode((*proc.entry).bytecode, ptr, len);
		}
	}

	for mut vec in state.allocations {
		// If a proc with this bytecode is still running, just leak the mrmoy
		if active_ptrs.contains(&vec.as_mut_ptr()) {
			std::mem::forget(vec);
		}
	}
}

pub fn set_bytecode(proc: &Proc, mut bytecode: Vec<u32>) {
	BYTECODE_ALLOCATIONS.with(|cell| {
		let mut state_ref = cell.borrow_mut();
		let state = state_ref.as_mut().unwrap();

		if !state.original.contains_key(&proc.id) {
			let (ptr, len) = unsafe { proc.bytecode_mut_ptr() };

			state.original.insert(proc.id, (ptr, len));
		}

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
			raw_types::misc::set_bytecode((*proc.entry).bytecode, ptr, len);
		}
	});
}
