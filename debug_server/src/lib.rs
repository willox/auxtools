mod instruction_hooking;
mod server;
mod server_types;

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
/*
	use std::fs::File;
	use std::io::Write;
	
	let mut file = File::create("E:/bytecode.txt").unwrap();

	let mut proc_id: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(proc_id));
		//let proc = Proc::find("/proc/test");
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();
		let (dism, err) = proc.disassemble();
		writeln!(&mut file, "Dism for {:?}", proc).unwrap();
		for x in &dism {
			writeln!(&mut file, "\t{}-{}: {:?}", x.0, x.1, x.2).unwrap();
		}

		if let Some(err) = err {
			writeln!(&mut file, "\n\tError: {:?}", err).unwrap();
		}
		writeln!(&mut file, "").unwrap();
		proc_id += 1;
	}
*/

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
			server.wait_for_connection(); // might never return 😳
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
