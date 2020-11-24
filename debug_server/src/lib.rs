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

#[hook("/proc/enable_debugging")]
fn enable_debugging(port: Value) {
	let addr = SocketAddr::new(
		IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		port.as_number()? as u16,
	);
	let server = server::Server::listen(&addr)
		.map_err(|e| runtime!("Couldn't create debug server {:?}", e))?;

	unsafe {
		*DEBUG_SERVER.get() = Some(server);
	}

	Ok(Value::from(true))
}
