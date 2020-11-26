#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use std::fs::File;
use std::io::Write;

use dm::*;

#[hook("/proc/dump_bytecode")]
fn hello_proc_hook() {
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

	Ok(Value::from(1337))
}

#[hook("/proc/typepaths")]
fn typepaths(something: Value) {
	Ok(Value::from(something.is_type("/datum")))
}
