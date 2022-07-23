mod assemble_env;
mod ckey_override;
mod disassemble_env;
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

#[cfg(not(windows))]
use mem_profiler_stub as mem_profiler;

pub(crate) use disassemble_env::DisassembleEnv;

use std::{
	cell::UnsafeCell,
	net::{IpAddr, Ipv4Addr, SocketAddr},
};

use auxtools::*;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);

#[shutdown]
fn debugger_shutdown() {
	unsafe {
		*DEBUG_SERVER.get() = None;
	}
}

fn get_default_mode() -> String {
	match std::env::var("AUXTOOLS_DEBUG_MODE") {
		Ok(val) => val,
		Err(_) => "NONE".into(),
	}
}

fn get_default_port() -> u16 {
	match std::env::var("AUXTOOLS_DEBUG_PORT") {
		Ok(val) => val.parse::<u16>().unwrap_or(server_types::DEFAULT_PORT),
		Err(_) => server_types::DEFAULT_PORT,
	}
}

#[hook("/proc/enable_debugging")]
fn enable_debugging(mode: Value, port: Value) {
	let mode = mode.as_string().unwrap_or_else(|_| get_default_mode());
	let port = port
		.as_number()
		.map(|x| x as u16)
		.unwrap_or_else(|_| get_default_port());

	let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

	let server = match mode.as_str() {
		"NONE" => {
			return Ok(Value::null());
		}

		"LAUNCHED" => server::Server::connect(&addr)
			.map_err(|e| runtime!("Couldn't create debug server: {}", e))?,

		"BACKGROUND" => server::Server::listen(&addr)
			.map_err(|e| runtime!("Couldn't create debug server: {}", e))?,

		"BLOCK" => {
			let mut server = server::Server::listen(&addr)
				.map_err(|e| runtime!("Couldn't create debug server: {}", e))?;
			server.process_until_configured(); // might never return 😳
			server
		}

		_ => {
			return Err(runtime!("invalid debugging mode: {:?}", mode));
		}
	};

	unsafe {
		*DEBUG_SERVER.get() = Some(server);
	}

	Ok(Value::null())
}
