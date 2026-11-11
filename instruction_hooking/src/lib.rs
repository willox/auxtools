pub mod disassemble_env;

use auxtools::*;
use std::{any::Any, cell::UnsafeCell};

// stackoverflow copypasta https://old.reddit.com/r/rust/comments/kkap4e/how_to_cast_a_boxdyn_mytrait_to_an_actual_struct/
pub trait InstructionHookToAny: 'static {
	fn as_any(&mut self) -> &mut dyn Any;
}

impl<T: 'static> InstructionHookToAny for T {
	fn as_any(&mut self) -> &mut dyn Any {
		self
	}
}

pub trait InstructionHook: InstructionHookToAny {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext);
}

pub static mut INSTRUCTION_HOOKS: UnsafeCell<Vec<Box<dyn InstructionHook>>> = UnsafeCell::new(Vec::new());

#[init(full)]
fn instruction_hooking_init() -> Result<(), String> {
	auxtools::instruction_hook::set_handler(handle_instruction)
}

#[shutdown]
fn instruction_hooking_shutdown() {
	unsafe {
		INSTRUCTION_HOOKS.get_mut().clear();
	}
}

// Handles any instruction BYOND tries to execute
fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		for vec_box in &mut *INSTRUCTION_HOOKS.get() {
			vec_box.handle_instruction(ctx);
		}
	}
}
