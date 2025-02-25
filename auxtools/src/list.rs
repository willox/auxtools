use std::iter::FromIterator;

use crate::*;

/// A wrapper around [Values](struct.Value.html) that make working with lists a
/// little easier
pub struct List {
	value: Value
}

impl List {
	pub fn from_value(val: &Value) -> DMResult<Self> {
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
			data: raw_types::values::ValueData { id: id.0 }
		};
		Self {
			value: unsafe { Value::from_raw_owned(raw) }
		}
	}

	pub fn get<I: Into<Value>>(&self, index: I) -> runtime::DMResult {
		let index = index.into();

		let mut value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		// assoc funcs for everything else
		unsafe {
			if raw_types::funcs::get_assoc_element(&mut value, self.value.raw, index.raw) == 1 {
				return Ok(Value::from_raw_owned(value));
			}

			Err(runtime!("failed to get assoc list entry (probably given an invalid list or key)"))
		}
	}

	pub fn set<K: Into<Value>, V: Into<Value>>(&self, index: K, value: V) -> Result<(), runtime::Runtime> {
		let index = index.into();
		let value = value.into();

		unsafe {
			if raw_types::funcs::set_assoc_element(self.value.raw, index.raw, value.raw) == 1 {
				return Ok(());
			}

			Err(runtime!("failed to set assoc list entry (probably given an invalid list or key)"))
		}
	}

	pub fn append<V: Into<Value>>(&self, value: V) {
		let value = value.into();

		unsafe {
			assert_eq!(raw_types::funcs::append_to_list(self.value.raw, value.raw), 1);
		}
	}

	pub fn remove<V: Into<Value>>(&self, value: V) {
		let value = value.into();

		unsafe {
			assert_eq!(raw_types::funcs::remove_from_list(self.value.raw, value.raw), 1);
		}
	}

	pub fn len(&self) -> u32 {
		let mut length: u32 = 0;
		unsafe {
			assert_eq!(raw_types::funcs::get_length(&mut length, self.value.raw), 1);
		}
		length
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	pub const fn is_list(value: &Value) -> bool {
		matches!(
			value.raw.tag,
			raw_types::values::ValueTag::List
				| raw_types::values::ValueTag::ArgList
				| raw_types::values::ValueTag::MobContents
				| raw_types::values::ValueTag::TurfContents
				| raw_types::values::ValueTag::AreaContents
				| raw_types::values::ValueTag::WorldContents
				| raw_types::values::ValueTag::ObjContents
				| raw_types::values::ValueTag::MobVars
				| raw_types::values::ValueTag::ObjVars
				| raw_types::values::ValueTag::TurfVars
				| raw_types::values::ValueTag::AreaVars
				| raw_types::values::ValueTag::ClientVars
				| raw_types::values::ValueTag::Vars
				| raw_types::values::ValueTag::MobOverlays
				| raw_types::values::ValueTag::MobUnderlays
				| raw_types::values::ValueTag::ObjOverlays
				| raw_types::values::ValueTag::ObjUnderlays
				| raw_types::values::ValueTag::TurfOverlays
				| raw_types::values::ValueTag::TurfUnderlays
				| raw_types::values::ValueTag::AreaOverlays
				| raw_types::values::ValueTag::AreaUnderlays
				| raw_types::values::ValueTag::ImageOverlays
				| raw_types::values::ValueTag::ImageUnderlays
				| raw_types::values::ValueTag::ImageVars
				| raw_types::values::ValueTag::TurfVisContents
				| raw_types::values::ValueTag::ObjVisContents
				| raw_types::values::ValueTag::MobVisContents
				| raw_types::values::ValueTag::ImageVisContents
				| raw_types::values::ValueTag::TurfVisLocs
				| raw_types::values::ValueTag::ObjVisLocs
				| raw_types::values::ValueTag::MobVisLocs
				| raw_types::values::ValueTag::WorldVars
				| raw_types::values::ValueTag::GlobalVars
		)
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
