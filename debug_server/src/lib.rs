mod instruction_hooking;
mod server;
mod server_types;

use std::{
	cell::UnsafeCell,
	net::{IpAddr, Ipv4Addr, SocketAddr},
};

use dm::*;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);

#[shutdown]
fn debugger_shutdown() {
	unsafe {
		*DEBUG_SERVER.get() = None;
	}
}

fn get_default_mode() -> String {
	static DEFAULT: &'static str = "NONE";

	match std::env::var("AUXTOOLS_DEBUG_MODE") {
		Ok(val) => val,
		Err(_) => DEFAULT.into(),
	}
}

fn get_default_port() -> u16 {
	static DEFAULT: u16 = 2448;

	match std::env::var("AUXTOOLS_DEBUG_PORT") {
		Ok(val) => val.parse::<u16>().unwrap_or(DEFAULT),
		Err(_) => DEFAULT,
	}
}

#[hook("/proc/enable_debugging")]
fn enable_debugging(mode: Value, port: Value) {
	let mode = mode.as_string().unwrap_or_else(|_| get_default_mode());
	let port = port
		.as_number()
		.map(|x| x as u16)
		.unwrap_or_else(|_| get_default_port());

	let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);

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
			server.wait_for_connection();
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
