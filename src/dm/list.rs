use crate::global_state::GLOBAL_STATE;
use crate::raw_types;
use crate::raw_types::values::IntoRawValue;
use crate::string;
use crate::value;

#[allow(unused)]
pub struct List {
	internal: *mut raw_types::lists::List,
	me_as_value: raw_types::values::Value,
}

#[allow(unused)]
impl<'a> List {
	pub unsafe fn from_id(id: u32) -> Self {
		let ptr = (GLOBAL_STATE.get().unwrap().get_list_by_id)(id);
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
		(GLOBAL_STATE.get().unwrap().inc_ref_count)(value);
		let ptr = (GLOBAL_STATE.get().unwrap().get_list_by_id)(value.data.id);
		if ptr.is_null() {
			panic!("oh fuck");
		}
		Self {
			internal: ptr,
			me_as_value: value,
		}
	}

	pub fn new() -> Self {
		Self::with_size(0)
	}

	pub fn with_size(capacity: u32) -> Self {
		let id = unsafe { (GLOBAL_STATE.get().unwrap().create_list)(capacity) };
		let as_value = raw_types::values::Value {
			tag: raw_types::values::ValueTag::List,
			data: raw_types::values::ValueData { id },
		};
		unsafe {
			(GLOBAL_STATE.get().unwrap().inc_ref_count)(as_value);
		}
		Self {
			me_as_value: as_value,
			internal: unsafe { (GLOBAL_STATE.get().unwrap().get_list_by_id)(id) },
		}
	}

	pub fn get<I: ListKey>(&self, index: I) -> value::Value<'a> {
		unsafe {
			value::Value::from_raw((GLOBAL_STATE.get().unwrap().get_assoc_element)(
				self.me_as_value,
				index.as_list_key(),
			))
		}
	}

	pub fn set<I: ListKey, V: IntoRawValue>(&mut self, index: I, value: &V) {
		unsafe {
			(GLOBAL_STATE.get().unwrap().set_assoc_element)(
				self.me_as_value,
				index.as_list_key(),
				value.into_raw_value(),
			);
		}
	}

	pub fn append<V: IntoRawValue>(&mut self, value: &V) {
		unsafe {
			(GLOBAL_STATE.get().unwrap().append_to_list)(self.me_as_value, value.into_raw_value());
		}
	}

	pub fn remove<V: IntoRawValue>(&mut self, value: &V) {
		unsafe {
			(GLOBAL_STATE.get().unwrap().remove_from_list)(
				self.me_as_value,
				value.into_raw_value(),
			);
		}
	}

	pub fn len(&self) -> u32 {
		unsafe { (GLOBAL_STATE.get().unwrap().get_length)(self.me_as_value) }
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
