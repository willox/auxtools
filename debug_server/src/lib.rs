mod instruction_hooking;
mod profiler;
mod server;
mod server_types;
mod stddef;

use std::{
	cell::UnsafeCell,
	net::{IpAddr, Ipv4Addr, SocketAddr},
};

use auxtools::*;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);
pub static mut PROFILER: UnsafeCell<Option<profiler::Profiler>> = UnsafeCell::new(None);

#[hook("/proc/profile_begin")]
fn profile_begin() {
	unsafe {
		*PROFILER.get() = Some(profiler::Profiler::new());
	}
	Ok(Value::from(true))
}

#[hook("/proc/profile_end")]
fn profile_end() {
	unsafe {
		if let Some(mut profiler) = (*PROFILER.get()).take() {
			profiler.finish();
		}
	}
	Ok(Value::from(true))
}

#[shutdown]
fn debugger_shutdown() {
	unsafe {
		*DEBUG_SERVER.get() = None;
		*PROFILER.get() = None;
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
