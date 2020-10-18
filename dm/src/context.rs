use super::list;
use super::raw_types::values::{ValueData, ValueTag};
use super::string;
use super::value::Value;
use crate::runtime;
use runtime::{ConversionResult, DMResult};
use std::marker::PhantomData;

/// Used to interact with global variables.
///
/// You don't need to make a context yourself, instead use the magical `ctx` value that is available to hooks.
///
/// ## Note
/// In order for global getters/setters to work, the DM code needs to contain usage of `global.vars["varname"]` somewhere.
#[allow(unused)]
pub struct DMContext<'a> {
	phantom: PhantomData<&'a ()>,
}

#[allow(unused)]
impl<'a> DMContext<'_> {
	/// Fetch a global variable from BYOND. Will return a runtime if the variable does not exist.
	///
	/// # Example
	/// ```ignore
	/// #[hook("/proc/my_proc")]
	/// fn my_proc_hook() {
	/// 	let glob_var = ctx.get_global("slime_count")?;
	/// 	Ok(glob_var)
	/// }
	/// ```
	pub fn get_global<S: Into<string::StringRef>>(&self, name: S) -> DMResult<'a> {
		unsafe {
			Value::new(ValueTag::World, ValueData { id: 1 }).get(name)
			// Tag World with value 1 means Global
		}
	}

	/// Fetch a numeric global variable from BYOND. Will return a runtime if the variable does not exist or is not a number.
	///
	/// See [get_global](#method.get_global)
	pub fn get_global_number<S: Into<string::StringRef>>(&self, name: S) -> ConversionResult<f32> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_number(name) }
	}

	/// Fetch a string global variable from BYOND. Will return a runtime if the variable does not exist or is not a string.
	///
	/// See [get_global](#method.get_global)
	pub fn get_global_string<S: Into<string::StringRef>>(
		&self,
		name: S,
	) -> ConversionResult<String> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 1 }).get_string(name) }
	}

	/// Fetch a list global variable from BYOND. Will return a runtime if the variable does not exist or is not a list.
	///
	/// See [get_global](#method.get_global)
	pub fn get_global_list<S: Into<string::StringRef>>(
		&self,
		name: S,
	) -> ConversionResult<list::List> {
		let globals = Value::globals();
		let list = globals.get_list(name)?;
		Ok(list)
	}

	/// Returns a [Value](struct.Value.html) representing the world object. It's the same as `world` in DM.
	pub fn get_world<S: Into<string::StringRef>>(&self, name: S) -> Value<'a> {
		unsafe { Value::new(ValueTag::World, ValueData { id: 0 }) }
	}

	pub unsafe fn new() -> Self {
		// Pretty dumb way to set the lifetime but im not changing it now
		Self {
			phantom: PhantomData,
		}
	}
}
