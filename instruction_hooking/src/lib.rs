pub mod disassemble_env;

use auxtools::{
	byond_scan::{Anchor, Extract, Recipe, SignatureTreatment, VersionRange},
	*
};
use retour::RawDetour;
use std::{any::Any, cell::UnsafeCell, ffi::c_void};

// Where the interpreter reads the next opcode, inside exec_proc. Copied from
// `byond_catalog` in the byond-re repo, which checks it against every local
// build.
//
// The running proc's frame sits in EDI here and in ESI on Linux, so each
// platform has its own assembly in execute_instruction_hook.*.
#[cfg(windows)]
const DISPATCH_MASK: &str = "0F B7 47 ?? 8B 4F ?? 8B F0 8B 14 ?? 89 95 ?? ?? ?? ?? 81 FA ?? 01 00 00";
#[cfg(unix)]
const DISPATCH_MASK: &str = "0F B7 46 ?? 8B 56 ?? 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? C1 E0 02 89 85 ?? ?? ?? ?? 01 D0 8B 18 81 FB ?? 01 00 00 \
                             0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";

const EXECUTE_INSTRUCTION: Recipe = Recipe {
	name: "execute_instruction",
	versions: VersionRange { min: 1659, max: 1688 },
	anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK),
	hops: &[],
	extract: Extract::Entry
};

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

extern "C" {
	// Trampoline to the original un-hooked BYOND execute_instruction code
	static mut execute_instruction_original: *const c_void;

	// Our version of execute_instruction. It hasn't got a calling convention
	// rust knows about, so don't call it.
	fn execute_instruction_hook();

	// The 514 version of the instruction hook.
	#[cfg(windows)]
	fn execute_instruction_hook_514();
}

#[init(full)]
fn instruction_hooking_init() -> Result<(), String> {
	let execute_instruction = find_recipe(&EXECUTE_INSTRUCTION)?;

	#[cfg(windows)]
	let versioned_hook = if auxtools::version::get().0 == 514 {
		execute_instruction_hook_514 as *const ()
	} else {
		execute_instruction_hook as *const ()
	};
	#[cfg(unix)]
	let versioned_hook = execute_instruction_hook as *const ();

	unsafe {
		let hook = RawDetour::new(execute_instruction as *const (), versioned_hook).map_err(|_| "Couldn't detour execute_instruction")?;

		hook.enable().map_err(|_| "Couldn't enable execute_instruction detour")?;

		execute_instruction_original = hook.trampoline() as *const () as *const c_void;

		// We never remove or disable the hook, so just forget about it.
		std::mem::forget(hook);
	}

	Ok(())
}

#[shutdown]
fn instruction_hooking_shutdown() {
	unsafe {
		INSTRUCTION_HOOKS.get_mut().clear();
	}
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure
// to return it.
#[no_mangle]
extern "C" fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) -> *const raw_types::procs::ExecutionContext {
	unsafe {
		for vec_box in &mut *INSTRUCTION_HOOKS.get() {
			vec_box.handle_instruction(ctx);
		}
	}

	ctx
}
