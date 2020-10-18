use crate::raw_types;
use crate::raw_types::values::IntoRawValue;
use crate::runtime;
use crate::value::Value;

/// A wrapper around [Values](struct.Value.html) that make working with lists a little easier
#[allow(unused)]
pub struct List<'a> {
	internal: *mut raw_types::lists::List,
	value: Value<'a>,
}

#[allow(unused)]
impl<'a> List<'a> {
	pub unsafe fn from_id(id: u32) -> Self {
		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		assert_eq!(
			raw_types::funcs::get_list_by_id(&mut ptr, raw_types::lists::ListId(id)),
			1
		);

		let raw = raw_types::values::Value {
			tag: raw_types::values::ValueTag::List,
			data: raw_types::values::ValueData { id },
		};

		Self {
			internal: ptr,
			value: Value::from_raw(raw),
		}
	}

	pub unsafe fn from_raw_value(raw: raw_types::values::Value) -> Self {
		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		assert_eq!(
			raw_types::funcs::get_list_by_id(&mut ptr, raw_types::lists::ListId(raw.data.id)),
			1
		);
		if ptr.is_null() {
			panic!("oh fuck");
		}
		Self {
			internal: ptr,
			value: Value::from_raw(raw),
		}
	}

	/// Creates a new empty list.
	pub fn new() -> List<'static> {
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
	pub fn with_size(capacity: u32) -> List<'static> {
		let mut id: raw_types::lists::ListId = raw_types::lists::ListId(0);
		unsafe {
			assert_eq!(raw_types::funcs::create_list(&mut id, capacity), 1);
		}

		let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
		unsafe {
			assert_eq!(raw_types::funcs::get_list_by_id(&mut ptr, id), 1);
		}
		if ptr.is_null() {
			panic!("oh fuck");
		}

		let raw = raw_types::values::Value {
			tag: raw_types::values::ValueTag::List,
			data: raw_types::values::ValueData { id: id.0 },
		};
		List {
			internal: ptr,
			value: unsafe { Value::from_raw_owned(raw) },
		}
	}

	pub fn get<I: ListKey>(&self, index: I) -> runtime::DMResult<'a> {
		let mut value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		unsafe {
			if raw_types::funcs::get_assoc_element(
				&mut value,
				self.value.into_raw_value(),
				index.as_list_key(),
			) == 1
			{
				return Ok(Value::from_raw(value));
			}

			runtime!("failed to get assoc list entry (probably given an invalid list or key)")
		}
	}

	pub fn set<I: ListKey, V: IntoRawValue>(
		&mut self,
		index: I,
		value: V,
	) -> Result<(), runtime::Runtime> {
		unsafe {
			if raw_types::funcs::set_assoc_element(
				self.value.into_raw_value(),
				index.as_list_key(),
				value.into_raw_value(),
			) == 1
			{
				return Ok(());
			}

			runtime!("failed to set assoc list entry (probably given an invalid list or key)")
		}
	}

	pub fn append<V: IntoRawValue>(&mut self, value: V) {
		unsafe {
			raw_types::funcs::append_to_list(self.value.into_raw_value(), value.into_raw_value());
		}
	}

	pub fn remove<V: IntoRawValue>(&mut self, value: V) {
		unsafe {
			raw_types::funcs::remove_from_list(self.value.into_raw_value(), value.into_raw_value());
		}
	}

	pub fn len(&self) -> u32 {
		let mut length: u32 = 0;
		unsafe {
			assert_eq!(
				raw_types::funcs::get_length(&mut length, self.value.into_raw_value()),
				1
			);
		}
		return length;
	}
}

impl<'a> From<Value<'a>> for List<'a> {
	fn from(value: Value) -> Self {
		unsafe { Self::from_id(value.value.data.id) }
	}
}

impl<'a> raw_types::values::IntoRawValue for &List<'a> {
	unsafe fn into_raw_value(self) -> raw_types::values::Value {
		self.value.into_raw_value()
	}
}

impl<'a> From<List<'a>> for Value<'a> {
	fn from(list: List<'a>) -> Value<'a> {
		list.value.clone()
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

impl ListKey for &Value<'_> {
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
