#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/auxtools_stack_trace")]
fn hello_proc_hook() {
/*
	let mut proc_id: u32 = 0;
	loop {
		//let proc = Proc::from_id(raw_types::procs::ProcId(proc_id));
		let proc = Proc::find("/world/proc/test");
		if proc.is_none() {
			break;
		}
		let proc = proc.unwrap();

		let (dism, err) = proc.disassemble();
		//if let Some(err) = err {
			let mut buf = format!("Dism for {:?}\n", proc);
			for x in &dism {
				buf.push_str(format!("\t{}-{}: {:?}\n", x.0, x.1, x.2).as_str());
			}
		
			buf.push_str(format!("\tError: {:?}", err).as_str());
			return Ok(Value::from_string(buf));
		//}

		proc_id += 1;
	}

	Ok(Value::from(true))
*/

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
