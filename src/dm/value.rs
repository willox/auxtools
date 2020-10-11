use super::raw_types;
use super::string;
use super::GLOBAL_STATE;
use crate::list;
use crate::raw_types::values::IntoRawValue;
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;

/// `Value` represents any value a DM variable can hold, such as numbers, strings, datums, etc.
#[derive(Clone)]
pub struct Value<'a> {
	pub value: raw_types::values::Value,
	pub phantom: PhantomData<&'a raw_types::values::Value>,
}

impl<'a> Drop for Value<'a> {
	fn drop(&mut self) {
		unsafe {
			(GLOBAL_STATE.get().unwrap().dec_ref_count)(self.into_raw_value());
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
		(GLOBAL_STATE.get().unwrap().inc_ref_count)(raw);

		Value {
			value: raw,
			phantom: PhantomData {},
		}
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

	fn get_by_id(&self, name_id: u32) -> Value<'b> {
		let val = unsafe { (GLOBAL_STATE.get().unwrap().get_variable)(self.value, name_id) };
		unsafe { (GLOBAL_STATE.get().unwrap().inc_ref_count)(val) }
		unsafe { Self::from_raw(val) }
	}

	fn set_by_id(&self, name_id: u32, new_value: raw_types::values::Value) {
		unsafe { (GLOBAL_STATE.get().unwrap().set_variable)(self.value, name_id, new_value) }
	}

	/// Gets a variable by name.
	pub fn get<S: Into<string::StringRef>>(&self, name: S) -> Value<'b> {
		self.get_by_id(name.into().get_id())
	}

	/// Gets a variable by name and safely casts it to a float.
	pub fn get_number<S: Into<string::StringRef>>(&self, name: S) -> Option<f32> {
		self.get(name).as_number()
	}

	/// Gets a variable by name and safely casts it to a string.
	pub fn get_string<S: Into<string::StringRef>>(&self, name: S) -> Option<String> {
		self.get(name).as_string()
	}

	/// Gets a variable by name and safely casts it to a [list::List].
	pub fn get_list<S: Into<string::StringRef>>(&self, name: S) -> Option<list::List> {
		self.get(name).as_list()
	}

	/// Sets a variable by name to a given value.
	pub fn set<S: Into<string::StringRef>, V: raw_types::values::IntoRawValue>(
		&self,
		name: S,
		new_value: &V,
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
	pub fn as_list(&self) -> Option<list::List> {
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
	/// ```rust
	/// src.call("explode", &[&Value::from(3.0)]);
	/// ```
	pub fn call<S: AsRef<str>>(&self, procname: S, args: &[&Self]) -> Value<'b> {
		unsafe {
			let procname = String::from(procname.as_ref()).replace("_", " ");
			let args: Vec<_> = args.iter().map(|e| e.into_raw_value()).collect();
			let result = (GLOBAL_STATE.get().unwrap().call_datum_proc_by_name)(
				Value::null().into_raw_value(),
				2,
				string::StringRef::from(procname).value.value.data.string,
				self.into_raw_value(),
				args.as_ptr(),
				args.len(),
				0,
				0,
			);
			Value::from_raw(result)
		}
	}

	/// blah blah lifetime is not verified with this so use at your peril
	pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
		Value::new(v.tag, v.data)
	}
}

impl fmt::Display for Value<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

fn string_to_raw_value(string: &str) -> Option<raw_types::values::Value> {
	if let Ok(string) = CString::new(string) {
		unsafe {
			let index =
				(GLOBAL_STATE.get().unwrap().get_string_id)(string.as_ptr(), true, false, true);

			return Some(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { id: index },
			});
		}
	}
	None
}

impl From<&str> for Value<'_> {
	fn from(s: &str) -> Self {
		unsafe { Value::from_raw(string_to_raw_value(s).unwrap()) }
	}
}

impl From<String> for Value<'_> {
	fn from(s: String) -> Self {
		unsafe { Value::from_raw(string_to_raw_value(s.as_str()).unwrap()) }
	}
}

impl From<&String> for Value<'_> {
	fn from(s: &String) -> Self {
		unsafe { Value::from_raw(string_to_raw_value(s.as_str()).unwrap()) }
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

impl raw_types::values::IntoRawValue for Value<'_> {
	unsafe fn into_raw_value(&self) -> raw_types::values::Value {
		self.value
	}
}
