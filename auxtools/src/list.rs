use crate::raw_types;
use crate::raw_types::values::ValueTag;
use crate::runtime;
use crate::runtime::ConversionResult;
use crate::value::Value;
use std::iter::FromIterator;

/// A wrapper around [Values](struct.Value.html) that make working with lists a little easier
#[allow(unused)]
pub struct List {
	value: Value,
}

#[allow(unused)]
impl List {
	pub fn from_value(val: &Value) -> ConversionResult<Self> {
		if !Self::is_list(val) {
			return Err(runtime!("attempted to create List from non-list value"));
		}

		Ok(Self { value: val.clone() })
	}

	/// Creates a new empty list.
	pub fn new() -> Self {
		Self::with_size(0)
	}

	/// Creates a new list filled with `capacity` nulls.
	pub fn with_size(capacity: u32) -> Self {
		let mut id: raw_types::lists::ListId = raw_types::lists::ListId(0);
		unsafe {
			assert_eq!(raw_types::funcs::create_list(&mut id, capacity), 1);
		}

		let raw = raw_types::values::Value {
			tag: raw_types::values::ValueTag::List,
			data: raw_types::values::ValueData { id: id.0 },
		};
		Self {
			value: unsafe { Value::from_raw_owned(raw) },
		}
	}

	pub fn get<I: Into<Value>>(&self, index: I) -> runtime::DMResult {
		let index = index.into();

		let mut value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		// assoc funcs for everything else
		unsafe {
			if raw_types::funcs::get_assoc_element(&mut value, self.value.raw, index.raw) == 1 {
				return Ok(Value::from_raw_owned(value));
			}

			Err(runtime!(
				"failed to get assoc list entry (probably given an invalid list or key)"
			))
		}
	}

	pub fn set<V: Into<Value>>(&self, index: V, value: V) -> Result<(), runtime::Runtime> {
		let index = index.into();
		let value = value.into();

		unsafe {
			if raw_types::funcs::set_assoc_element(self.value.raw, index.raw, value.raw) == 1 {
				return Ok(());
			}

			Err(runtime!(
				"failed to set assoc list entry (probably given an invalid list or key)"
			))
		}
	}

	pub fn append<V: Into<Value>>(&self, value: V) {
		let value = value.into();

		unsafe {
			assert_eq!(
				raw_types::funcs::append_to_list(self.value.raw, value.raw),
				1
			);
		}
	}

	pub fn remove<V: Into<Value>>(&self, value: V) {
		let value = value.into();

		unsafe {
			assert_eq!(
				raw_types::funcs::remove_from_list(self.value.raw, value.raw),
				1
			);
		}
	}

	pub fn len(&self) -> u32 {
		let mut length: u32 = 0;
		unsafe {
			assert_eq!(raw_types::funcs::get_length(&mut length, self.value.raw), 1);
		}
		length
	}

	pub fn is_list(value: &Value) -> bool {
		match value.raw.tag {
			ValueTag::List
			| ValueTag::MobVars
			| ValueTag::ObjVars
			| ValueTag::TurfVars
			| ValueTag::AreaVars
			| ValueTag::ClientVars
			| ValueTag::Vars
			| ValueTag::MobOverlays
			| ValueTag::MobUnderlays
			| ValueTag::ObjOverlays
			| ValueTag::ObjUnderlays
			| ValueTag::TurfOverlays
			| ValueTag::TurfUnderlays
			| ValueTag::AreaOverlays
			| ValueTag::AreaUnderlays
			| ValueTag::ImageVars
			| ValueTag::WorldVars
			| ValueTag::GlobalVars => true,
			_ => false,
		}
	}
}

impl FromIterator<Value> for List {
	fn from_iter<I: IntoIterator<Item = Value>>(it: I) -> Self {
		let res = Self::new();

		for val in it {
			res.append(val);
		}

		res
	}
}

impl From<List> for Value {
	fn from(list: List) -> Self {
		list.value
	}
}

impl From<&List> for Value {
	fn from(list: &List) -> Self {
		list.value.clone()
	}
}
