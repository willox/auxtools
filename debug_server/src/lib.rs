mod instruction_hooking;
mod server;
mod server_types;
mod stddef;

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


struct DisassembleEnv;
impl dmasm::disassembler::DisassembleEnv for DisassembleEnv {
	fn get_string(&mut self, index: u32) -> Option<String> {
		unsafe {
			Some(StringRef::from_id(raw_types::strings::StringId(index)).into())
		}
	}

	fn get_variable_name(&mut self, index: u32) -> Option<String> {
		unsafe {
			Some(StringRef::from_variable_id(raw_types::strings::VariableId(index)).into())
		}
	}

    fn get_proc_name(&mut self, index: u32) -> Option<String> {
		Proc::from_id(raw_types::procs::ProcId(index)).map(|x| x.path)
	}

	fn value_to_string(&mut self, tag: u32, data: u32) -> Option<String> {
		unsafe {
			let value = Value::new(std::mem::transmute(tag as u8), std::mem::transmute(data));
			value.to_string().ok()
		}
	}
}

use std::fs::File;
use std::io::Write;

#[hook("/proc/enable_debugging")]
fn enable_debugging(mode: Value, port: Value) {

	let mut file = File::create("E:/dism.txt").unwrap();

	let mut id: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(id));
		id = id + 1;

		if proc.is_none() {
			break;
		}

		let proc = proc.unwrap();

		unsafe {
			let (bytecode, len) = proc.bytecode();
			let bytecode = std::slice::from_raw_parts(bytecode, len);

			let mut env = DisassembleEnv;
			let (nodes, err) = dmasm::disassembler::disassemble(bytecode, &mut env);

			let dism = dmasm::format_disassembly(&nodes, None);

			if let Some(err) = err {
				write!(&mut file, "{}\n{}ERROR: {:?}\nBYTECODE: {:02X?}\n\n", proc.path, dism, err, bytecode).unwrap();
			} else {
				write!(&mut file, "{}\n{}\n\n", proc.path, dism).unwrap();
			}

		}
	}



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
