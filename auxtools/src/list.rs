use std::iter::FromIterator;

use crate::*;

/// A wrapper around [Values](struct.Value.html) that make working with lists a
/// little easier. It can also hold an `/alist`, where numbers are keys.
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
		Self::default()
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

	/// Whether this is a DM `/alist`, where numbers are keys and not positions.
	pub fn is_alist(&self) -> bool {
		self.value.raw.tag == raw_types::values::ValueTag::Alist
	}

	/// Every key and value of an `/alist`, in the order `for (var/k in A)`
	/// gives them.
	///
	/// This is a copy, so nothing here points into BYOND's tree once it returns.
	pub fn alist_pairs(&self) -> DMResult<Vec<(Value, Value)>> {
		if !self.is_alist() {
			return Err(runtime!("attempted to read alist pairs of a non-alist"));
		}

		let id = unsafe { self.value.raw.data.id } as usize;
		let record = unsafe {
			// raw reads of BYOND's table, so a stale id has to be turned away here
			if id >= *raw_types::funcs::ALIST_TABLE_COUNT as usize {
				return Err(runtime!("alist id {} is outside the alist table", id));
			}
			// BYOND reallocates the table as it grows, so read the pointer every time
			*(*raw_types::funcs::ALIST_TABLE).add(id)
		};
		if record.is_null() {
			return Err(runtime!("alist id {} has no record", id));
		}

		let mut pairs = vec![];
		let mut node = unsafe { (*record).root };
		let mut pending = vec![];
		// left, node, right, without recursing
		while !node.is_null() || !pending.is_empty() {
			while !node.is_null() {
				pending.push(node);
				node = unsafe { (*node).left };
			}
			let entry = pending.pop().unwrap();
			unsafe {
				pairs.push((Value::from_raw((*entry).key), Value::from_raw((*entry).value)));
				node = (*entry).right;
			}
		}

		Ok(pairs)
	}

	/// Whether DM's `islist()` says yes to this value.
	///
	/// This asks BYOND instead of matching on the tag, because a tag can't
	/// answer it: a `filters` list and a single filter share one.
	pub fn is_list(value: &Value) -> bool {
		let mut is_list: u8 = 0;
		unsafe {
			assert_eq!(raw_types::funcs::value_is_list(&mut is_list, value.raw), 1);
		}
		is_list != 0
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

impl Default for List {
	fn default() -> Self {
		Self::with_size(0)
	}
}
