#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use std::fs::File;
use std::io::Write;
use dm::*;

#[hook("/proc/install_instruction")]
fn hello_proc_hook() {

	let proc = Proc::find("/proc/test").unwrap();
	let bytecode = unsafe {
		let (ptr, count) = proc.bytecode();
		std::slice::from_raw_parts_mut(ptr, count)
	};

	bytecode[0] = 1337;
	bytecode[1] = 1337;
	bytecode[2] = 1337;
	bytecode[3] = 1337;
	bytecode[4] = 1337;
	bytecode[5] = 1337;
	bytecode[6] = 1337;
	bytecode[7] = 1337;
	Ok(Value::from(true))

	/*
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

	Ok(Value::from(true))
	*/

/*
	let mut success = 0;
	let mut total = 0;
	let mut proc_id: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(proc_id));
		//let proc = Proc::find("/world/proc/SDQL_var");
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();

		let (dism, err) = proc.disassemble();
		if err.is_none() {
			success += 1;
		}

		total += 1;
		proc_id += 1;
	}

	Ok(Value::from_string(format!("{}/{}", success, total)))
*/

	/*
	let frames = CallStacks::new(ctx).active;
	let mut buf = String::new();

	for frame in &frames {
		buf.push_str(
			format!(
				"{} @ {}:{}\n",
				frame.proc.path,
				String::from(frame.file_name.as_ref().unwrap()),
				frame.line_number.unwrap()
			)
			.as_str(),
		);

		buf.push_str("\tArguments:\n");
		for local in &frame.args {
			let name = match &local.0 {
				Some(n) => String::from(n),
				None => "<no name>".to_string(),
			};
			buf.push_str(
				format!(
					"\t\t{} = {:?}\n",
					name,
					local.1
				)
				.as_str()
			)
		}
		buf.push('\n');

		buf.push_str("\tLocals:\n");
		for local in &frame.locals {
			buf.push_str(
				format!(
					"\t\t{} = {:?}\n",
					String::from(&local.0),
					local.1
				)
				.as_str()
			)
		}
		buf.push('\n');
	}

	Ok(Value::from_string(buf))
	*/
}
