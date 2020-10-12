use super::raw_types;
use super::value::Value;
use std::ffi::CStr;
use std::fmt;

pub struct StringRef {
	pub value: Value<'static>,
}

impl StringRef {
	pub fn new(string: &str) -> Self {
		StringRef {
			value: Value::from(string),
		}
	}

	pub fn from_value(value: Value) -> Option<Self> {
		// TODO: Check type with a nice api
		if value.value.tag != raw_types::values::ValueTag::String {
			return None;
		}

		// Here we're going from value -> raw -> new value because to get that juicy static lifetime
		Some(StringRef {
			value: unsafe { Value::from_raw(value.value) },
		})
	}

	pub unsafe fn from_id(id: u32) -> Self {
		// TODO: Could check the string id is valid
		StringRef {
			value: Value::from_raw(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { id: id },
			}),
		}
	}

	pub fn get_id(&self) -> u32 {
		return unsafe { self.value.value.data.id };
	}
}

impl Clone for StringRef {
	fn clone(&self) -> Self {
		Self::from_value(self.value.clone()).unwrap()
	}
}

impl fmt::Debug for StringRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// TODO: Show ref count? Escape special chars?
		let data: String = self.clone().into();
		write!(f, "{}", data)
	}
}

impl From<&str> for StringRef {
	fn from(s: &str) -> Self {
		StringRef::new(s)
	}
}

impl From<String> for StringRef {
	fn from(s: String) -> Self {
		StringRef::new(s.as_str())
	}
}

impl From<&String> for StringRef {
	fn from(s: &String) -> Self {
		StringRef::new(s.as_str())
	}
}

impl Into<String> for StringRef {
	fn into(self) -> String {
		unsafe {
			let id = self.value.value.data.string;
			let mut entry: *mut raw_types::strings::StringEntry = std::ptr::null_mut();
			assert_eq!(raw_types::funcs::get_string_table_entry(&mut entry, id), 1);
			CStr::from_ptr((*entry).data).to_string_lossy().into()
		}
	}
}
