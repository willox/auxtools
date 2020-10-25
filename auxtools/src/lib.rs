use dm::*;

#[hook("/datum/gas_mixture/proc/react")]
fn hello_proc_hook() {
	let world = ctx.get_world();
	let maxx = world.get_number("maxx")?;

	Ok(Value::null())
}
