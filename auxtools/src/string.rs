use crate::*;
use std::{ffi::CStr, fmt};

/// A wrapper around [Values](struct.Value.html) that make working with strings
/// a little easier
pub struct StringRef {
	pub value: Value
}

impl StringRef {
	pub fn new(string: &str) -> DMResult<Self> {
		Ok(StringRef {
			value: Value::from_string(string)?
		})
	}

	pub fn from_raw(data: &[u8]) -> DMResult<Self> {
		Ok(StringRef {
			value: Value::from_string_raw(data)?
		})
	}

	pub fn from_value(value: Value) -> Option<Self> {
		if value.raw.tag != raw_types::values::ValueTag::String {
			return None;
		}

		// Here we're going from value -> raw -> new value because to get that juicy
		// static lifetime
		Some(StringRef {
			value: unsafe { Value::from_raw(value.raw) }
		})
	}

	pub unsafe fn from_id(id: raw_types::strings::StringId) -> Self {
		StringRef {
			value: Value::from_raw(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { string: id }
			})
		}
	}

	pub unsafe fn from_variable_id(id: raw_types::strings::VariableId) -> Self {
		let string_id = *((*raw_types::funcs::VARIABLE_NAMES).entries.add(id.0 as usize));

		StringRef {
			value: Value::from_raw(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { string: string_id }
			})
		}
	}

	pub const fn get_id(&self) -> raw_types::strings::StringId {
		unsafe { self.value.raw.data.string }
	}

	pub fn data(&self) -> &[u8] {
		unsafe {
			let id = self.value.raw.data.string;
			let mut entry: *mut raw_types::strings::StringEntry = std::ptr::null_mut();
			assert_eq!(raw_types::funcs::get_string_table_entry(&mut entry, id), 1);
			CStr::from_ptr((*entry).data).to_bytes()
		}
	}
}

impl Clone for StringRef {
	fn clone(&self) -> Self {
		Self::from_value(self.value.clone()).unwrap()
	}
}

impl fmt::Display for StringRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:?}", self)
	}
}

impl fmt::Debug for StringRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut format = vec![];

		let mut iter = self.data().iter();

		while let Some(&byte) = iter.next() {
			if byte == 0xFF {
				// NOTE: Doesn't hold state for formatting, so some strings relying on are a
				// little off
				format.extend_from_slice(match iter.next() {
					None => break,
					Some(1) | Some(2) | Some(3) | Some(4) => b"[]",
					Some(5) => b"[]\\th",
					Some(6) => b"\\a",
					Some(7) => b"\\A",
					Some(8) => b"\\the",
					Some(9) => b"\\The",
					Some(10) => b"\\he",
					Some(11) => b"\\He",
					Some(12) => b"\\his",
					Some(13) => b"\\His",
					Some(14) => b"\\hers",
					Some(15) => b"\\Hers",
					Some(16) => b"\\him ",
					Some(17) => b"\\himself",
					Some(18) => b"\\... ",
					Some(19) => b"\\n",
					Some(20) => b"\\s ",
					Some(21) => b"\\proper ",
					Some(22) => b"\\improper ",
					Some(23) => b"\\bold ",
					Some(24) => b"\\italic ",
					Some(25) => b"\\underline ",
					Some(26) => b"\\strike ",
					Some(27) => b"\\font",
					Some(28) => b"\\color",
					Some(29) => b"\\font",
					Some(30) => b"\\color",
					Some(31) => b"\\red ",
					Some(32) => b"\\green ",
					Some(33) => b"\\blue ",
					Some(34) => b"\\black ",
					Some(35) => b"\\white ",
					Some(36) => b"\\yellow ",
					Some(37) => b"\\cyan ",
					Some(38) => b"\\magenta ",
					Some(39) => b"\\beep ",
					Some(40) => b"\\link",
					Some(41) => b" \\link",
					Some(42) => b"\\ref[]",
					Some(43) => b"\\icon[]",
					Some(44) => b"\\roman[]",
					Some(45) => b"\\Roman[]",
					Some(_) => b"[UNKNONWN FORMAT SPECIFIER]"
				});
				continue;
			}

			if byte == b'\n' {
				format.extend_from_slice(b"\\n");
				continue;
			}

			if byte == b'\r' {
				format.extend_from_slice(b"\\r");
				continue;
			}

			// Escape \[]"" chars
			if byte == b'\\' || byte == b'[' || byte == b']' || byte == b'"' {
				format.push(b'\\');
			}

			format.push(byte);
		}

		write!(f, "\"{}\"", String::from_utf8_lossy(&format))
	}
}

impl From<&StringRef> for StringRef {
	fn from(string: &StringRef) -> StringRef {
		string.to_owned()
	}
}

impl From<&StringRef> for String {
	fn from(string: &StringRef) -> String {
		unsafe {
			let id = string.value.raw.data.string;
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

impl From<StringRef> for Value {
	fn from(string: StringRef) -> Self {
		string.value
	}
}

impl From<&StringRef> for Value {
	fn from(string: &StringRef) -> Self {
		string.value.clone()
	}
}
