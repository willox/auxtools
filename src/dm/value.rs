use super::raw_types;
use super::GLOBAL_STATE;
use std::fmt;
use std::marker::PhantomData;

pub struct Value<'a> {
	pub value: raw_types::values::Value,
	pub phantom: PhantomData<&'a raw_types::values::Value>,
}

impl fmt::Display for Value<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

fn create_value<'a>(tag: raw_types::values::ValueTag, data: raw_types::values::ValueData) -> Value<'a> {
	Value {
		value: raw_types::values::Value { tag, data },
		phantom: PhantomData {},
	}
}

fn value_from_string<'a>(s: &String) -> Value<'a> {
	let mut s = s.clone();
	s.push(0x00 as char);
	let id = unsafe { (GLOBAL_STATE.get().unwrap().get_string_id)(s.as_str(), true, false, true) };
	create_value(
		raw_types::values::ValueTag::String,
		raw_types::values::ValueData {
			string: raw_types::strings::StringRef(id),
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
		create_value(raw_types::values::ValueTag::Number, raw_types::values::ValueData { number: num })
	}
}

impl From<i32> for Value<'_> {
	fn from(num: i32) -> Self {
		create_value(raw_types::values::ValueTag::Number, raw_types::values::ValueData { number: num as f32 })
	}
}

impl From<u32> for Value<'_> {
	fn from(num: u32) -> Self {
		create_value(raw_types::values::ValueTag::Number, raw_types::values::ValueData { number: num as f32 })
	}
}

impl From<bool> for Value<'_> {
	fn from(b: bool) -> Self {
		create_value(
			raw_types::values::ValueTag::Number,
			raw_types::values::ValueData {
				number: if b { 1.0 } else { 0.0 },
			},
		)
	}
}
