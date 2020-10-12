use crate::raw_types;
use crate::raw_types::values::IntoRawValue;
use crate::string;
use crate::value;

/// Represents a DM `list`.
#[allow(unused)]
pub struct List {
	internal: *mut raw_types::lists::List,
	me_as_value: raw_types::values::Value,
}

#[allow(unused)]
impl<'a> List {
	pub unsafe fn from_id(id: u32) -> Self {
		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		assert_eq!(
			raw_types::funcs::get_list_by_id(&mut ptr, raw_types::lists::ListId(id)),
			1
		);
		Self {
			internal: ptr,
			me_as_value: raw_types::values::Value {
				tag: raw_types::values::ValueTag::List,
				data: raw_types::values::ValueData { id },
			},
		}
	}

	pub unsafe fn from_raw_value<V: IntoRawValue>(value: V) -> Self {
		let value = value.into_raw_value();

		// TODO: This leaks!
		raw_types::funcs::inc_ref_count(value);

		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		assert_eq!(
			raw_types::funcs::get_list_by_id(&mut ptr, raw_types::lists::ListId(value.data.id)),
			1
		);
		if ptr.is_null() {
			panic!("oh fuck");
		}
		Self {
			internal: ptr,
			me_as_value: value,
		}
	}

	/// Creates a new empty list.
	pub fn new() -> Self {
		Self::with_size(0)
	}

	/// Creates a new empty list, with enough memory reserved to contain `capacity` elements.
	/// NOTE: UNTESTED, BYOND MAY RESIZE IT BACK DOWN!
	pub fn with_capacity(capacity: u32) -> Self {
		let res = Self::with_size(capacity);
		unsafe {
			(*res.internal).length = 0;
		}
		res
	}

	/// Creates a new list filled with `capacity` nulls.
	pub fn with_size(capacity: u32) -> Self {
		let mut id: raw_types::lists::ListId = raw_types::lists::ListId(0);
		unsafe {
			assert_eq!(raw_types::funcs::create_list(&mut id, capacity), 1);
		}
		let as_value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::List,
			data: raw_types::values::ValueData { id: id.0 },
		};

		// TODO: This leaks!
		unsafe {
			raw_types::funcs::inc_ref_count(as_value);
		}

		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		unsafe {
			assert_eq!(raw_types::funcs::get_list_by_id(&mut ptr, id), 1);
		}
		if ptr.is_null() {
			panic!("oh fuck");
		}

		Self {
			me_as_value: as_value,
			internal: ptr,
		}
	}

	pub fn get<I: ListKey>(&self, index: I) -> value::Value<'a> {
		let mut value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		// TODO: Should handle error
		unsafe {
			assert_eq!(
				raw_types::funcs::get_assoc_element(
					&mut value,
					self.me_as_value,
					index.as_list_key()
				),
				1
			);
			value::Value::from_raw(value)
		}
	}

	pub fn set<I: ListKey, V: IntoRawValue>(&mut self, index: I, value: &V) {
		// TODO: Should handle error
		unsafe {
			raw_types::funcs::set_assoc_element(
				self.me_as_value,
				index.as_list_key(),
				value.into_raw_value(),
			);
		}
	}

	pub fn append<V: IntoRawValue>(&mut self, value: &V) {
		unsafe {
			raw_types::funcs::append_to_list(self.me_as_value, value.into_raw_value());
		}
	}

	pub fn remove<V: IntoRawValue>(&mut self, value: &V) {
		unsafe {
			raw_types::funcs::remove_from_list(self.me_as_value, value.into_raw_value());
		}
	}

	pub fn len(&self) -> u32 {
		let mut length: u32 = 0;
		unsafe {
			assert_eq!(
				raw_types::funcs::get_length(&mut length, self.me_as_value),
				1
			);
		}
		return length;
	}
}

impl<'a> From<value::Value<'a>> for List {
	fn from(value: value::Value) -> Self {
		unsafe { Self::from_id(value.value.data.id) }
	}
}

impl raw_types::values::IntoRawValue for List {
	unsafe fn into_raw_value(&self) -> raw_types::values::Value {
		self.me_as_value
	}
}

impl From<List> for value::Value<'_> {
	fn from(list: List) -> Self {
		unsafe { Self::from_raw(list.me_as_value) }
	}
}

pub trait ListKey {
	fn as_list_key(self) -> raw_types::values::Value;
}

impl ListKey for &raw_types::values::Value {
	fn as_list_key(self) -> raw_types::values::Value {
		*self
	}
}

impl ListKey for &value::Value<'_> {
	fn as_list_key(self) -> raw_types::values::Value {
		unsafe { self.into_raw_value() }
	}
}

impl ListKey for u32 {
	fn as_list_key(self) -> raw_types::values::Value {
		raw_types::values::Value {
			tag: raw_types::values::ValueTag::Number,
			data: raw_types::values::ValueData {
				number: self as f32,
			},
		}
	}
}

fn str_to_listkey<S: Into<String>>(s: S) -> raw_types::values::Value {
	unsafe {
		string::StringRef::new(s.into().as_str())
			.value
			.into_raw_value()
	}
}

impl ListKey for &str {
	fn as_list_key(self) -> raw_types::values::Value {
		str_to_listkey(self)
	}
}

impl ListKey for &String {
	fn as_list_key(self) -> raw_types::values::Value {
		str_to_listkey(self)
	}
}
