use crate::raw_types;
use crate::raw_types::values::{IntoRawValue, ValueTag};
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
		Self {
			value: unsafe { Value::from_raw_owned(raw) },
		}
	}

	pub fn get<I: ListKey>(&self, index: I) -> runtime::DMResult {
		let mut value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 },
		};

		let index = index.into_list_key();

		// assoc funcs for everything else
		unsafe {
			if raw_types::funcs::get_assoc_element(&mut value, self.value.into_raw_value(), index)
				== 1
			{
				return Ok(Value::from_raw_owned(value));
			}

			Err(runtime!(
				"failed to get assoc list entry (probably given an invalid list or key)"
			))
		}
	}

	pub fn set<I: ListKey, V: IntoRawValue>(
		&self,
		index: I,
		value: V,
	) -> Result<(), runtime::Runtime> {
		let index = index.into_list_key();
		let value = unsafe { value.into_raw_value() };

		unsafe {
			if raw_types::funcs::set_assoc_element(
				self.value.into_raw_value(),
				index.into_list_key(),
				value,
			) == 1
			{
				return Ok(());
			}

			Err(runtime!(
				"failed to set assoc list entry (probably given an invalid list or key)"
			))
		}
	}

	pub fn append<V: IntoRawValue>(&self, value: V) {
		unsafe {
			raw_types::funcs::append_to_list(self.value.into_raw_value(), value.into_raw_value());
		}
	}

	pub fn remove<V: IntoRawValue>(&self, value: V) {
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
		length
	}

	/// Copies the List's vector part (values accessible by numeric indices) into a Vec<Value>.
	pub fn to_vec(self) -> Vec<Value> {
		unsafe {
			let mut ptr: *mut raw_types::lists::List = std::ptr::null_mut();
			assert_eq!(
				raw_types::funcs::get_list_by_id(&mut ptr, self.value.value.data.list),
				1
			);
			std::slice::from_raw_parts((*ptr).vector_part as *const _, self.len() as usize).to_vec()
		}
	}

	pub fn is_list(value: &Value) -> bool {
		match value.value.tag {
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

		// TODO: This is probably a performane bottleneck.
		for val in it {
			res.append(&val);
		}

		res
	}
}

impl raw_types::values::IntoRawValue for &List {
	unsafe fn into_raw_value(self) -> raw_types::values::Value {
		self.value.into_raw_value()
	}
}

impl From<List> for Value {
	fn from(list: List) -> Value {
		list.value.clone()
	}
}

pub trait ListKey {
	fn into_list_key(self) -> raw_types::values::Value;
}

impl ListKey for &raw_types::values::Value {
	fn into_list_key(self) -> raw_types::values::Value {
		*self
	}
}

impl ListKey for &Value {
	fn into_list_key(self) -> raw_types::values::Value {
		unsafe { self.into_raw_value() }
	}
}

impl ListKey for u32 {
	fn into_list_key(self) -> raw_types::values::Value {
		raw_types::values::Value {
			tag: raw_types::values::ValueTag::Number,
			data: raw_types::values::ValueData {
				number: self as f32,
			},
		}
	}
}
