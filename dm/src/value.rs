use super::raw_types;
use super::string;
use crate::list;
use crate::raw_types::values::IntoRawValue;
use crate::runtime;
use crate::runtime::{ConversionResult, DMResult};
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;

/// `Value` represents any value a DM variable can hold, such as numbers, strings, datums, etc.
///
/// There's a lot of lifetime shenanigans going on, the gist of it is to just not keep Values around for longer than your hook's execution.
pub struct Value<'a> {
	pub value: raw_types::values::Value,
	phantom: PhantomData<&'a raw_types::values::Value>,
}

impl<'a> Drop for Value<'a> {
	fn drop(&mut self) {
		unsafe {
			raw_types::funcs::dec_ref_count(self.into_raw_value());
		}
	}
}

#[allow(unused)]
impl<'b> Value<'b> {
	/// Creates a new value from raw tag and data.
	/// Use if you know what you are doing.
	pub unsafe fn new<'a>(
		tag: raw_types::values::ValueTag,
		data: raw_types::values::ValueData,
	) -> Value<'a> {
		let raw = raw_types::values::Value { tag, data };
		raw_types::funcs::inc_ref_count(raw);

		Value {
			value: raw,
			phantom: PhantomData {},
		}
	}

	/// Equivalent to DM's `global.vars`.
	pub fn globals() -> Value<'static> {
		return Value {
			value: raw_types::values::Value {
				tag: raw_types::values::ValueTag::Null,
				data: raw_types::values::ValueData { number: 0.0 },
			},
			phantom: PhantomData {},
		};
	}

	/// Equivalent to DM's `null`.
	pub fn null() -> Value<'static> {
		return Value {
			value: raw_types::values::Value {
				tag: raw_types::values::ValueTag::Null,
				data: raw_types::values::ValueData { number: 0.0 },
			},
			phantom: PhantomData {},
		};
	}

	fn get_by_id(&self, name_id: u32) -> DMResult<'b> {
		let mut val = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		unsafe {
			if raw_types::funcs::get_variable(
				&mut val,
				self.value,
				raw_types::strings::StringId(name_id),
			) != 1
			{
				let varname: String = string::StringRef::from_id(name_id).into();
				return runtime!("Could not read {}.{}", &self, varname);
			}

			Ok(Self::from_raw(val))
		}
	}

	fn set_by_id(
		&self,
		name_id: u32,
		new_value: raw_types::values::Value,
	) -> Result<(), runtime::Runtime> {
		unsafe {
			if raw_types::funcs::set_variable(
				self.value,
				raw_types::strings::StringId(name_id),
				new_value,
			) != 1
			{
				let varname: String = string::StringRef::from_id(name_id).into();
				return runtime!("Could not write to {}.{}", self, varname);
			}
		}
		Ok(())
	}

	/// Gets a variable by name.
	pub fn get<S: Into<string::StringRef>>(&self, name: S) -> DMResult<'b> {
		let name = name.into();
		self.get_by_id(name.get_id())
	}

	/// Gets a variable by name and safely casts it to a float.
	pub fn get_number<S: Into<string::StringRef>>(&self, name: S) -> ConversionResult<f32> {
		match self.get(name)?.as_number() {
			Some(num) => Ok(num),
			None => runtime!("Attempt to interpret non-number value as float"),
		}
	}

	/// Gets a variable by name and safely casts it to a string.
	pub fn get_string<S: Into<string::StringRef>>(&self, name: S) -> ConversionResult<String> {
		match self.get(name)?.as_string() {
			Some(s) => Ok(s),
			None => runtime!("Attempt to interpret non-string value as String"),
		}
	}

	/// Gets a variable by name and safely casts it to a [list::List].
	pub fn get_list<S: Into<string::StringRef>>(
		&self,
		name: S,
	) -> ConversionResult<list::List<'b>> {
		let var = self.get(name)?;

		match var.as_list() {
			Some(lst) => Ok(lst),
			None => runtime!("Attempt to interpret non-list value as List"),
		}
	}

	/// Sets a variable by name to a given value.
	pub fn set<S: Into<string::StringRef>, V: raw_types::values::IntoRawValue>(
		&self,
		name: S,
		new_value: V,
	) {
		unsafe {
			self.set_by_id(name.into().get_id(), new_value.into_raw_value());
		}
	}

	/// Check if the current value is a number and casts it.
	pub fn as_number(&self) -> Option<f32> {
		match self.value.tag {
			raw_types::values::ValueTag::Number => unsafe { Some(self.value.data.number) },
			_ => None,
		}
	}

	/// Check if the current value is a string and casts it.
	pub fn as_string(&self) -> Option<String> {
		match self.value.tag {
			raw_types::values::ValueTag::String => unsafe {
				Some(string::StringRef::from_id(self.value.data.id).into())
			},
			_ => None,
		}
	}

	/// Check if the current value is a list and casts it.
	pub fn as_list(&self) -> Option<list::List<'b>> {
		match self.value.tag {
			raw_types::values::ValueTag::List => unsafe {
				Some(list::List::from_id(self.value.data.id))
			},
			_ => None,
		}
	}

	/// Calls a method of the value with the given arguments.
	///
	/// # Examples:
	///
	/// This example is equivalent to `src.explode(3)` in DM.
	/// ```ignore
	/// src.call("explode", &[&Value::from(3.0)]);
	/// ```
	pub fn call<S: AsRef<str>>(&self, procname: S, args: &[&Self]) -> DMResult<'b> {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		unsafe {
			// Increment ref-count of args permenently before passing them on
			for v in args {
				raw_types::funcs::inc_ref_count(v.into_raw_value());
			}

			let procname = String::from(procname.as_ref()).replace("_", " ");
			let args: Vec<_> = args.iter().map(|e| e.into_raw_value()).collect();
			let name_ref = string::StringRef::new(&procname);

			if raw_types::funcs::call_datum_proc_by_name(
				&mut ret,
				Value::null().into_raw_value(),
				2,
				name_ref.value.value.data.string,
				self.value,
				args.as_ptr(),
				args.len(),
				0,
				0,
			) == 1
			{
				return Ok(Value::from_raw_owned(ret));
			}
		}

		runtime!("External proc call failed")
	}

	/// Creates a Value that references a byond string.
	/// Will panic if the given string contains null bytes
	///
	/// # Examples:
	/// ```ignore
	/// let my_string = Value::from_string("Testing!");
	/// ```
	pub fn from_string<S: AsRef<str>>(data: S) -> Value<'static> {
		// TODO: This should be done differently
		let string = CString::new(data.as_ref()).unwrap();

		unsafe {
			let mut id = raw_types::strings::StringId(0);

			assert_eq!(
				raw_types::funcs::get_string_id(&mut id, string.as_ptr(), 1, 0, 1),
				1
			);

			Value::new(
				raw_types::values::ValueTag::String,
				raw_types::values::ValueData { string: id },
			)
		}
	}

	/// blah blah lifetime is not verified with this so use at your peril
	pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
		Value::new(v.tag, v.data)
	}

	/// same as from_raw but does not increment the reference count (assumes we already own this reference)
	pub unsafe fn from_raw_owned<'a>(v: raw_types::values::Value) -> Value<'a> {
		Value {
			value: v,
			phantom: PhantomData {},
		}
	}
}

impl<'a> Clone for Value<'a> {
	fn clone(&self) -> Value<'a> {
		unsafe { Value::from_raw(self.into_raw_value()) }
	}
}

impl fmt::Display for Value<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

impl From<f32> for Value<'_> {
	fn from(num: f32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num },
			)
		}
	}
}

impl From<i32> for Value<'_> {
	fn from(num: i32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num as f32 },
			)
		}
	}
}

impl From<u32> for Value<'_> {
	fn from(num: u32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num as f32 },
			)
		}
	}
}

impl From<bool> for Value<'_> {
	fn from(b: bool) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData {
					number: if b { 1.0 } else { 0.0 },
				},
			)
		}
	}
}

impl raw_types::values::IntoRawValue for &Value<'_> {
	unsafe fn into_raw_value(self) -> raw_types::values::Value {
		self.value
	}
}
