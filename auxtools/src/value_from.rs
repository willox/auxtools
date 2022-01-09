use crate::raw_types;
use crate::runtime::Runtime;
use crate::string;
use crate::List;
use crate::Value;
use std::collections::HashMap;
use std::convert::TryFrom;

impl From<f32> for Value {
	fn from(num: f32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num },
			)
		}
	}
}

impl From<i32> for Value {
	fn from(num: i32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num as f32 },
			)
		}
	}
}

impl From<u32> for Value {
	fn from(num: u32) -> Self {
		unsafe {
			Value::new(
				raw_types::values::ValueTag::Number,
				raw_types::values::ValueData { number: num as f32 },
			)
		}
	}
}

impl From<bool> for Value {
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

impl From<&Value> for Value {
	fn from(val: &Value) -> Self {
		val.to_owned()
	}
}

/* List-y helpers */

// This is broken due to https://github.com/rust-lang/rust/issues/50133
// The blanket implementation of TryFrom in core conflicts with -any- generics on a TryFrom trait
//
// impl<T: AsRef<str>> TryFrom<T> for Value {
// 	type Error = Runtime;
// 	fn try_from(value: T) -> Result<Self, Self::Error> {
// 		Value::from_string(value.as_ref())
// 	}
// }

// Specialized for ease-of-use due to the above not being possible
impl<T: Into<Value> + Clone> TryFrom<&HashMap<String, T>> for Value {
	type Error = Runtime;
	fn try_from(hashmap: &HashMap<String, T>) -> Result<Self, Self::Error> {
		let res = List::new();

		for (k, v) in hashmap {
			let string = string::StringRef::new(k)?;
			res.set(string, v.clone())?;
		}

		Ok(res.into())
	}
}

impl<A: Into<Value> + Clone, B: Into<Value> + Clone> TryFrom<&HashMap<A, B>> for Value {
	type Error = Runtime;
	fn try_from(hashmap: &HashMap<A, B>) -> Result<Self, Self::Error> {
		let res = List::new();

		for (k, v) in hashmap {
			// This can fail for basically any reason that BYOND decides,
			// because in the end this just ends up calling into BYOND with the Value's.
			res.set(k.clone(), v.clone())?;
		}

		Ok(res.into())
	}
}

impl<T: Into<Value> + Clone> From<&Vec<T>> for Value {
	fn from(vec: &Vec<T>) -> Self {
		let res = List::new();
		for val in vec {
			res.append(val.clone());
		}
		res.into()
	}
}
