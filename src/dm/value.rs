use super::raw_types::strings::StringRef;
use super::raw_types::values::{RawValue, ValueData, ValueTag};
use super::GLOBAL_STATE;
use std::fmt;
use std::marker::PhantomData;

pub struct Value<'a> {
	pub value: RawValue,
	pub phantom: PhantomData<&'a RawValue>,
}

impl fmt::Display for Value<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

fn create_value<'a>(tag: ValueTag, data: ValueData) -> Value<'a> {
	Value {
		value: RawValue { tag, data },
		phantom: PhantomData {},
	}
}

fn value_from_string<'a>(s: &String) -> Value<'a> {
	let mut s = s.clone();
	s.push(0x00 as char);
	let id = unsafe { (GLOBAL_STATE.get().unwrap().get_string_id)(s.as_str(), true, false, true) };
	create_value(
		ValueTag::String,
		ValueData {
			string: StringRef(id),
		},
	)
}

impl From<&String> for Value<'_> {
	fn from(s: &String) -> Self {
		value_from_string(s)
	}
}

impl From<&str> for Value<'_> {
	fn from(s: &str) -> Self {
		value_from_string(&s.to_owned())
	}
}

impl From<f32> for Value<'_> {
	fn from(num: f32) -> Self {
		create_value(ValueTag::Number, ValueData { number: num })
	}
}

impl From<i32> for Value<'_> {
	fn from(num: i32) -> Self {
		create_value(ValueTag::Number, ValueData { number: num as f32 })
	}
}

impl From<u32> for Value<'_> {
	fn from(num: u32) -> Self {
		create_value(ValueTag::Number, ValueData { number: num as f32 })
	}
}

impl From<bool> for Value<'_> {
	fn from(b: bool) -> Self {
		create_value(
			ValueTag::Number,
			ValueData {
				number: if b { 1.0 } else { 0.0 },
			},
		)
	}
}
