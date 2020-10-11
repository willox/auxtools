use crate::global_state::GLOBAL_STATE;
use crate::raw_types;
use crate::raw_types::values::IntoRawValue;
use crate::string;
use crate::value;

pub struct List {
	internal: *mut raw_types::lists::List,
	me_as_value: raw_types::values::Value,
}

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

	pub fn get<I: ListKey>(&self, index: I) -> value::Value<'a> {
		unsafe {
			value::Value::from_raw((GLOBAL_STATE.get().unwrap().get_assoc_element)(
				self.me_as_value,
				index.as_list_key(),
			))
		}
	}
}

impl<'a> From<value::Value<'a>> for List {
	fn from(value: value::Value) -> Self {
		unsafe { Self::from_id(value.value.data.id) }
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
