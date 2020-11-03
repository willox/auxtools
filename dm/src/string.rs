use super::raw_types;
use super::value::Value;
use std::ffi::CStr;
use std::fmt;

/// A wrapper around [Values](struct.Value.html) that make working with strings a little easier
pub struct StringRef {
	pub value: Value,
}

impl StringRef {
	pub fn new(string: &str) -> Self {
		StringRef {
			value: Value::from_string(string),
		}
	}

	pub fn from_value(value: Value) -> Option<Self> {
		if value.value.tag != raw_types::values::ValueTag::String {
			return None;
		}

		// Here we're going from value -> raw -> new value because to get that juicy static lifetime
		Some(StringRef {
			value: unsafe { Value::from_raw(value.value) },
		})
	}

	pub unsafe fn from_id(id: raw_types::strings::StringId) -> Self {
		StringRef {
			value: Value::from_raw(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { string: id },
			}),
		}
	}

	pub unsafe fn from_variable_id(id: raw_types::strings::VariableId) -> Self {
		let string_id = *(raw_types::funcs::VARIABLE_NAMES.add(id.0 as usize));

		StringRef {
			value: Value::from_raw(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { string: string_id },
			}),
		}
	}

	pub fn get_id(&self) -> raw_types::strings::StringId {
		unsafe { self.value.value.data.string }
	}
}

impl Clone for StringRef {
	fn clone(&self) -> Self {
		Self::from_value(self.value.clone()).unwrap()
	}
}

impl fmt::Debug for StringRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let data: String = self.clone().into();
		write!(f, "{}", data)
	}
}

impl From<&str> for StringRef {
	fn from(string: &str) -> StringRef {
		StringRef::new(string)
	}
}

impl From<&StringRef> for String {
	fn from(string: &StringRef) -> String {
		unsafe {
			let id = string.value.value.data.string;
			let mut entry: *mut raw_types::strings::StringEntry = std::ptr::null_mut();
			assert_eq!(raw_types::funcs::get_string_table_entry(&mut entry, id), 1);
			CStr::from_ptr((*entry).data).to_string_lossy().into()
		}
	}
}

impl From<StringRef> for String {
	fn from(string: StringRef) -> String {
		String::from(&string)
	}
}
