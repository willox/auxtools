use super::global_state::{State, GLOBAL_STATE};
use super::list;
use super::raw_types::values::{ValueData, ValueTag};
use super::string;
use super::value::Value;

/// The context is used to interact with global stuff. It should probably be renamed at this point.
#[allow(unused)]
pub struct DMContext<'a> {
	state: &'a State,
}

#[allow(unused)]
impl<'a> DMContext<'_> {
	/// NOTE: In order for this to work, the dm code needs to contain `global.vars["varname"]` anywhere.
	pub fn get_global<S: Into<string::StringRef>>(&self, name: S) -> Value {
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

	pub fn get_global_list<S: Into<string::StringRef>>(&self, name: S) -> Option<list::List> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_list(name) }
	}

	pub fn get_world<S: Into<string::StringRef>>(&self, name: S) -> Value<'a> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 0 }) }
	}

	pub fn new() -> Option<Self> {
		if let Some(state) = GLOBAL_STATE.get() {
			Some(Self { state: &state })
		} else {
			None
		}
	}
}
