//! For your DM code coverage """needs"""

mod codecov;

use auxtools::*;
use codecov::Tracker;
use instruction_hooking::INSTRUCTION_HOOKS;

fn with_tracker_option<F>(f: F, create: bool)
where
	F: FnOnce(&mut Tracker)
{
	unsafe {
		let hooks = INSTRUCTION_HOOKS.get_mut();

		// `type_id()` on the boxed hook itself would give the id of the trait
		// object, never `Tracker`'s, so ask the hook for its concrete
		// value as `Any` first.
		let tracker_option = hooks.iter_mut().find_map(|hook| hook.as_mut().as_any().downcast_mut::<Tracker>());

		match tracker_option {
			Some(existing_tracker) => {
				f(existing_tracker);
			}
			None => {
				if create {
					let mut created_tracker = Tracker::new();
					f(&mut created_tracker);
					hooks.push(Box::new(created_tracker));
				}
			}
		}
	}
}

// INSTRUCTION_HOOKS are cleared on shutdown so we don't need to worry about
// that.
#[hook("/proc/start_code_coverage")]
fn start_code_coverage(coverage_file: Value) {
	let coverage_file_string = coverage_file.as_string()?;

	let mut init_result = false;
	with_tracker_option(
		|tracker| {
			init_result = tracker.init_context(coverage_file_string.clone());
		},
		true
	);

	if !init_result {
		return Err(runtime!("A code coverage context for {} already exists!", coverage_file_string));
	}

	Ok(Value::NULL)
}

#[hook("/proc/stop_code_coverage")]
fn stop_code_coverage(coverage_file: Value) {
	let coverage_file_string = coverage_file.as_string()?;

	let mut result = Ok(Value::NULL);
	with_tracker_option(
		|tracker| {
			let inner_result = tracker.finalize_context(&coverage_file_string);
			result = match inner_result {
				Ok(had_entry) => {
					if !had_entry {
						Err(runtime!("A code coverage context for {} does not exist!", coverage_file_string))
					} else {
						Ok(Value::NULL)
					}
				}
				Err(error) => Err(runtime!("A error occurred while trying to save the coverage file: {}", error))
			}
		},
		false
	);

	result
}

#[allow(clippy::missing_const_for_fn)]
pub fn anti_dce_stub() {}
