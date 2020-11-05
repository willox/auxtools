mod instruction_hooking;
mod server;
mod server_types;

use std::{
	cell::RefCell,
	net::{IpAddr, Ipv4Addr, SocketAddr},
};

use dm::*;

thread_local! {
	pub static DEBUG_SERVER: RefCell<Option<server::Server>> = RefCell::new(None);
}

#[hook("/proc/enable_debugging")]
fn enable_debugging(port: Value) {
	let addr = SocketAddr::new(
		IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		port.as_number()? as u16,
	);
	let server =
		server::Server::new(&addr).map_err(|e| runtime!("Couldn't create debug server {:?}", e))?;

	DEBUG_SERVER.with(|x| {
		x.replace(Some(server));
	});

	Ok(Value::from(true))
}

#[hook("/proc/install_instruction")]
fn hello_proc_hook() {
	let stacks = CallStacks::new(ctx);
	let _proc = Proc::find("/proc/test").unwrap();

	println!("{:?}", stacks.active[0].line_number);

	//hook_instruction(&proc, 11).unwrap();

	Ok(Value::from(true))
}
