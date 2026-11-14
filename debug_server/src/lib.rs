mod assemble_env;
mod ckey_override;
mod connection;
mod dmb_tables;
mod hot_reload;
mod instruction_hooking;
mod journal;
mod launched;
mod macros;
mod server;
mod server_types;
mod skin;
mod stddef;
mod unnamed;
mod waker;
mod world_resources;

#[cfg(windows)]
mod crash_handler_windows;

#[cfg(windows)]
mod mem_profiler;

#[cfg(not(windows))]
mod mem_profiler_stub;

use std::{
	cell::UnsafeCell,
	panic::{catch_unwind, AssertUnwindSafe},
	sync::atomic::{AtomicU32, Ordering}
};

pub(crate) use ::instruction_hooking::disassemble_env::DisassembleEnv;
use ::instruction_hooking::{InstructionHook, INSTRUCTION_HOOKS};
use auxtools::*;
#[cfg(not(windows))]
use mem_profiler_stub as mem_profiler;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);

/// World generation, incremented on reboot or shutdown.
pub static WORLD_ENDS: AtomicU32 = AtomicU32::new(0);

/// Abort debug server work when evaluated DM code ends the world.
/// Unwinding code must not access the destroyed server.
pub struct WorldEnded;

/// Running BYOND build used for compilation.
pub(crate) fn world_byond() -> dmasm::ByondVersion {
	let (major, build) = auxtools::version::get();
	dmasm::ByondVersion::new(major, build)
}

/// Handle panics at the BYOND callback boundary.
fn recover(payload: &(dyn std::any::Any + Send), server: &mut server::Server) {
	if !payload.is::<WorldEnded>() {
		server.recover_from_panic();
	}
}

#[shutdown]
fn debugger_shutdown() {
	WORLD_ENDS.fetch_add(1, Ordering::Relaxed);
	// INSTRUCTION_HOOKS are cleared on shutdown so we don't need to worry about
	// that.
	unsafe {
		DEBUG_SERVER.get_mut().take();
	}
}

struct DebugServerInstructionHook<'a> {
	debug_server: &'a mut UnsafeCell<Option<server::Server>>
}

impl InstructionHook for DebugServerInstructionHook<'static> {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		let Some(debug_server) = self.debug_server.get_mut() else {
			return;
		};

		// BYOND calls this through extern "C" code, and a panic that gets back
		// out there aborts the whole game. Losing the debugger is the better
		// deal.
		if let Err(payload) = catch_unwind(AssertUnwindSafe(|| debug_server.handle_instruction(ctx))) {
			recover(&*payload, debug_server);
		}
	}
}

/// Makes `server` the debug server, run on every instruction
fn install_server(server: server::Server) {
	let debug_server_instruction_hook;
	unsafe {
		*DEBUG_SERVER.get() = Some(server);
		debug_server_instruction_hook = DebugServerInstructionHook {
			debug_server: &mut *std::ptr::addr_of_mut!(DEBUG_SERVER)
		};

		INSTRUCTION_HOOKS.get_mut().push(Box::new(debug_server_instruction_hook));
	}
}
