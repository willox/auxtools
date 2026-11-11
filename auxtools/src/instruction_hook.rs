//! Intercept interpreter instructions for debugger callbacks.
//! Proc hooks also require this detour on builds where interpreter calls
//! bypass call_proc_by_id; see `hooks::calls_bypass_detour`.

use std::ffi::c_void;

use byond_scan::{Anchor, Extract, Recipe, SignatureTreatment, VersionRange};
use retour::RawDetour;

use crate::raw_types;

// Where the interpreter reads the next opcode, inside exec_proc.
//
// The running proc's frame sits in EDI here and in ESI on Linux, so each
// platform has its own assembly in execute_instruction_hook.*.
#[cfg(windows)]
const DISPATCH_MASK: &str = "0F B7 47 ?? 8B 4F ?? 8B F0 8B 14 ?? 89 95 ?? ?? ?? ?? 81 FA ?? 01 00 00";
// Linux 515 before 515.1644: the frame is in EDI
#[cfg(unix)]
const DISPATCH_MASK_EARLY_515: &str = "0F B7 57 14 8B 47 10 0F B7 DA 8B 0C 98 81 F9 ?? 01 00 00 77 ?? FF 24 8D ?? ?? ?? ??";
#[cfg(unix)]
const DISPATCH_MASK_515: &str = "0F B7 46 14 8B 56 10 8D 3C 85 00 00 00 00 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? 8D 04 3A 8B 18 81 FB ?? 01 00 00 \
                                 0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";
#[cfg(unix)]
const DISPATCH_MASK: &str = "0F B7 46 ?? 8B 56 ?? 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? C1 E0 02 89 85 ?? ?? ?? ?? 01 D0 8B 18 81 FB ?? 01 00 00 \
                             0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";

#[cfg(windows)]
const CHECKED: VersionRange = VersionRange { min: 1637, max: 1688 };
#[cfg(unix)]
const CHECKED: VersionRange = VersionRange { min: 1648, max: 1688 };

const EXECUTE_INSTRUCTION: Recipe = Recipe {
	name: "execute_instruction",
	versions: CHECKED,
	anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK),
	hops: &[],
	extract: Extract::Entry
};

// The frame is in ESI from 515.1644, as on 516; in EDI before
#[cfg(unix)]
const EXECUTE_INSTRUCTIONS: &[Recipe] = &[
	EXECUTE_INSTRUCTION,
	Recipe {
		name: "execute_instruction",
		versions: VersionRange { min: 1644, max: 1647 },
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: VersionRange { min: 1637, max: 1643 },
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_EARLY_515),
		hops: &[],
		extract: Extract::Entry
	}
];
#[cfg(windows)]
const EXECUTE_INSTRUCTIONS: &[Recipe] = &[EXECUTE_INSTRUCTION];

extern "C" {
	// Trampoline to the original un-hooked BYOND execute_instruction code
	static mut execute_instruction_original: *const c_void;

	// Assembly entry point with a nonstandard calling convention.
	// Do not call directly from Rust.
	fn execute_instruction_hook();

	// The 514 version of the instruction hook.
	#[cfg(windows)]
	fn execute_instruction_hook_514();

	// For Linux 515 before 515.1644, where the frame is in EDI
	#[cfg(unix)]
	fn execute_instruction_hook_edi();
}

type Handler = fn(*mut raw_types::procs::ExecutionContext);

// Read on every instruction
static mut HANDLER: Option<Handler> = None;
static mut CALLS_BYPASS_DETOUR: bool = false;
// The detour stays for the life of the process, through reboots
static mut DETOURED: bool = false;

/// Install a handler called before each instruction with its running frame.
/// Replaces the previous handler.
pub fn set_handler(handler: Handler) -> Result<(), String> {
	detour()?;
	unsafe {
		HANDLER = Some(handler);
	}
	Ok(())
}

/// Called when auxtools starts
pub(crate) fn init() -> Result<(), String> {
	unsafe {
		CALLS_BYPASS_DETOUR = crate::hooks::calls_bypass_detour();
		if CALLS_BYPASS_DETOUR {
			detour()?;
		}
	}
	Ok(())
}

fn detour() -> Result<(), String> {
	unsafe {
		if DETOURED {
			return Ok(());
		}
	}
	let execute_instruction = crate::find_recipe_of(EXECUTE_INSTRUCTIONS)?;

	#[cfg(windows)]
	let versioned_hook = if crate::version::get().0 == 514 {
		execute_instruction_hook_514 as *const ()
	} else {
		execute_instruction_hook as *const ()
	};
	#[cfg(unix)]
	let versioned_hook = if crate::version::get().1 < 1644 {
		execute_instruction_hook_edi as *const ()
	} else {
		execute_instruction_hook as *const ()
	};

	unsafe {
		let hook = RawDetour::new(execute_instruction as *const (), versioned_hook).map_err(|_| "Couldn't detour execute_instruction")?;

		hook.enable().map_err(|_| "Couldn't enable execute_instruction detour")?;

		execute_instruction_original = hook.trampoline() as *const () as *const c_void;

		// The hook remains installed for the process lifetime.
		std::mem::forget(hook);
		DETOURED = true;
	}
	Ok(())
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure
// to return it.
#[no_mangle]
extern "C" fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) -> *const raw_types::procs::ExecutionContext {
	unsafe {
		// Dispatch proc hooks and load initialization for builds that bypass
		// the call_proc_by_id detour.
		if CALLS_BYPASS_DETOUR && *(*ctx).bytecode_offset() == 0 {
			crate::hooks::proc_starting(ctx);
		}
		if let Some(handler) = HANDLER {
			handler(ctx);
		}
	}

	ctx
}
