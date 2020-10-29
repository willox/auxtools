#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/auxtools_stack_trace")]
fn hello_proc_hook() {
	let proc = Proc::find("/datum/x/proc/wew").unwrap();
	let dism = proc.disassemble().unwrap();

	let mut buf = String::new();
	for x in &dism {
		buf.push_str(format!("{}-{}: {:?}\n", x.0, x.1, x.2).as_str());
	}

	Ok(Value::from_string(buf))

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
