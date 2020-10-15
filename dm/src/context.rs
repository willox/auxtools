use super::list;
use super::raw_types::values::{ValueData, ValueTag};
use super::string;
use super::value::Value;
use crate::runtime;
use runtime::{ConversionResult, DMResult};
use std::marker::PhantomData;

/// The context is used to interact with global stuff. It should probably be renamed at this point.
#[allow(unused)]
pub struct DMContext<'a> {
	phantom: PhantomData<&'a ()>,
}

#[allow(unused)]
impl<'a> DMContext<'_> {
	/// NOTE: In order for this to work, the dm code needs to contain `global.vars["varname"]` anywhere.
	pub fn get_global<S: Into<string::StringRef>>(&self, name: S) -> DMResult<'a> {
		unsafe {
			Value::new(ValueTag::World, ValueData { id: 1 }).get(name)
			// Tag World with value 1 means Global
		}
	}

	pub fn get_global_number<S: Into<string::StringRef>>(&self, name: S) -> ConversionResult<f32> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_number(name) }
	}

	pub fn get_global_string<S: Into<string::StringRef>>(
		&self,
		name: S,
	) -> ConversionResult<String> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_string(name) }
	}

	pub fn get_global_list<S: Into<string::StringRef>>(
		&self,
		name: S,
	) -> ConversionResult<list::List> {
		let globals = Value::globals();
		let list = globals.get_list(name)?;
		Ok(list)
	}

	pub fn get_world<S: Into<string::StringRef>>(&self, name: S) -> Value<'a> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 0 }) }
	}

	pub fn new() -> Option<Self> {
		// Pretty dumb way to set the lifetime but im not changing it now
		Some(Self {
			phantom: PhantomData,
		})
	}
}
