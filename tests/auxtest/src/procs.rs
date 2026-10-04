use auxtools::{raw_types::procs::ProcId, *};

// BYOND moves its whole proc table when the table grows. A `Proc` we were
// already holding has to keep working after the move.
#[hook("/proc/auxtest_procs")]
fn test_procs() {
	let table_start = || Proc::from_id(ProcId(0)).unwrap().entry();

	let before: Vec<(Proc, Vec<u32>)> = (0..)
		.map_while(|id| Proc::from_id(ProcId(id)))
		.map(|proc| {
			let bytecode = unsafe { proc.bytecode() }.to_vec();
			(proc, bytecode)
		})
		.collect();
	let old_start = table_start();

	Proc::find("/proc/auxtest_grow_proc_table")
		.ok_or_else(|| runtime!("test_procs: /proc/auxtest_grow_proc_table not defined"))?
		.call(&[])?;

	if table_start() == old_start {
		return Err(runtime!("test_procs: the proc table never moved, so this checked nothing"));
	}

	for (proc, bytecode) in before {
		if unsafe { proc.bytecode() } != bytecode {
			return Err(runtime!("test_procs: {} reads different bytecode after the proc table moved", proc.path));
		}
	}

	Ok(Value::from(true))
}
