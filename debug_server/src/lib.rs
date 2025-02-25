mod assemble_env;
mod ckey_override;
mod instruction_hooking;
mod server;
mod server_types;
mod stddef;

#[cfg(windows)]
mod crash_handler_windows;

#[cfg(windows)]
mod mem_profiler;

#[cfg(not(windows))]
mod mem_profiler_stub;

use std::{
	cell::UnsafeCell,
	net::{IpAddr, Ipv4Addr, SocketAddr}
};

pub(crate) use ::instruction_hooking::disassemble_env::DisassembleEnv;
use ::instruction_hooking::{InstructionHook, INSTRUCTION_HOOKS};
use auxtools::*;
#[cfg(not(windows))]
use mem_profiler_stub as mem_profiler;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);

#[shutdown]
fn debugger_shutdown() {
	// INSTRUCTION_HOOKS are cleared on shutdown so we don't need to worry about
	// that.
	unsafe {
		DEBUG_SERVER.get_mut().take();
	}
}

fn get_default_mode() -> String {
	match std::env::var("AUXTOOLS_DEBUG_MODE") {
		Ok(val) => val,
		Err(_) => "NONE".into()
	}
}

fn get_default_port() -> u16 {
	match std::env::var("AUXTOOLS_DEBUG_PORT") {
		Ok(val) => val.parse::<u16>().unwrap_or(server_types::DEFAULT_PORT),
		Err(_) => server_types::DEFAULT_PORT
	}
}

struct DebugServerInstructionHook<'a> {
	debug_server: &'a mut UnsafeCell<Option<server::Server>>
}

impl InstructionHook for DebugServerInstructionHook<'static> {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		if let Some(debug_server) = self.debug_server.get_mut() {
			debug_server.handle_instruction(ctx);
		}
	}
}

#[hook("/proc/enable_debugging")]
fn enable_debugging(mode: Value, port: Value) {
	let mode = mode.as_string().unwrap_or_else(|_| get_default_mode());
	let port = port.as_number().map(|x| x as u16).unwrap_or_else(|_| get_default_port());

	let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

	let server = match mode.as_str() {
		"NONE" => {
			return Ok(Value::NULL);
		}

		"LAUNCHED" => server::Server::connect(&addr).map_err(|e| runtime!("Couldn't create debug server: {}", e))?,

		"BACKGROUND" => server::Server::listen(&addr).map_err(|e| runtime!("Couldn't create debug server: {}", e))?,

		"BLOCK" => {
			let mut server = server::Server::listen(&addr).map_err(|e| runtime!("Couldn't create debug server: {}", e))?;
			server.process_until_configured(); // might never return 😳
			server
		}

		_ => {
			return Err(runtime!("invalid debugging mode: {:?}", mode));
		}
	};

	let debug_server_instruction_hook;
	unsafe {
		*DEBUG_SERVER.get() = Some(server);
		debug_server_instruction_hook = DebugServerInstructionHook {
			debug_server: &mut *std::ptr::addr_of_mut!(DEBUG_SERVER)
		};

		INSTRUCTION_HOOKS.get_mut().push(Box::new(debug_server_instruction_hook));
	}

	Ok(Value::NULL)
}
