use auxtools::*;
use std::{cell::UnsafeCell, collections::HashMap};

#[derive(PartialEq, Eq, Hash)]
struct RawRef {
	tag: u32,
	data: u32,
}

impl RawRef {
	fn new(value: &raw_types::values::Value) -> Self {
		Self {
			tag: value.tag as u32,
			data: unsafe { value.data.id } as u32,
		}
	}
}

struct Allocation {
	proc: Proc,
	line: Option<u32>,
}

struct State {
	allocations: HashMap<RawRef, Allocation>,
}

static mut STATE: UnsafeCell<Option<State>> = UnsafeCell::new(None);

#[init(partial)]
fn init() -> Result<(), String> {
	unsafe {
		*STATE.get_mut() = Some(State {
			allocations: HashMap::new(),
		});
	}

	Ok(())
}

#[shutdown]
fn shutdown() {
	unsafe {
		STATE.get_mut().take();
	}
}

pub fn on_allocated(list: &raw_types::values::Value) {
	let state = unsafe {
		STATE.get_mut().as_mut()
	};

	if let Some(state) = state {
		let stack = debug::CallStacks::new();
		let frame = stack.active.first().unwrap();

		state.allocations.insert(RawRef::new(list), Allocation {
			proc: frame.proc.clone(),
			line: frame.line_number,
		});
	}
}

#[hook("/proc/get_list_origin")]
pub fn get_origin(list: &Value) {
	let state = unsafe {
		STATE.get_mut().as_mut()
	};

	let state = state.ok_or_else(|| runtime!("list tracking not active"))?;

	if let Some(entry) = state.allocations.get(&RawRef::new(&list.raw)) {
		let res = format!("{:?}:{}", entry.proc.path, entry.line.unwrap_or(0));
		return Ok(Value::from_string(res)?);
	}

	Ok(Value::null())
}
