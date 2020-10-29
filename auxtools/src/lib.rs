#![deny(clippy::complexity, clippy::correctness, clippy::perf, clippy::style)]

use dm::*;

#[hook("/proc/hooked")]
fn hello_proc_hook() {

	let proc = Proc::find("/obj/item/proc/dye_item").unwrap();
	let params: Vec<String> = proc.local_names()
		.iter()
		.map(|x| String::from(x))
		.collect();

	/*
	let frames = CallStacks::new(ctx).active;

	let mut buf = String::new();

	for frame in &frames {
		let loc = String::from(frame.file_name.as_ref().unwrap());

		buf.push_str(
			format!(
				"{} @ {}:{}\n",
				frame.proc.path,
				String::from(frame.file_name.as_ref().unwrap()),
				frame.line_number.unwrap()
			)
			.as_str(),
		);

		buf.push_str("\tLocals:\n");
		for local in &frame.locals {
			buf.push_str(
				format!(
					"\t\t{} = {:?}",
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

	Ok(Value::from(2))
}
