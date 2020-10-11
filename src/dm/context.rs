use super::global_state::{State, GLOBAL_STATE};
use super::raw_types::values::{ValueData, ValueTag};
use super::string;
use super::value::Value;

pub struct DMContext<'a> {
	state: &'a State,
}

impl<'a> DMContext<'_> {
	//NOTE: In order for this to work, the dm code needs to contain `global.vars["varname"]` anywhere.
	pub fn get_global<S: Into<string::StringRef>>(&self, name: S) -> Option<Value> {
		unsafe {
			Value::new(ValueTag::World, ValueData { id: 1 }).get(name)
			// Tag World with value 1 means Global
		}
	}

	pub fn get_global_number<S: Into<string::StringRef>>(&self, name: S) -> Option<f32> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_number(name) }
	}

	pub fn get_global_string<S: Into<string::StringRef>>(&self, name: S) -> Option<String> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_string(name) }
	}

	pub fn new() -> Option<Self> {
		if let Some(state) = GLOBAL_STATE.get() {
			Some(Self { state: &state })
		} else {
			None
		}
	}
}
