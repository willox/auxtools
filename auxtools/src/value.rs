use super::{raw_types, string};
use crate::{list, runtime, runtime::DMResult};
use std::{ffi::CString, fmt, marker::PhantomData};

/// `Value` represents any value a DM variable can hold, such as numbers,
/// strings, datums, etc.
///
/// There's a lot of lifetime shenanigans going on, the gist of it is to just
/// not keep Values around for longer than your hook's execution.
pub struct Value {
	pub raw: raw_types::values::Value,
	phantom: PhantomData<*mut ()>
}

impl PartialEq for Value {
	fn eq(&self, other: &Self) -> bool {
		unsafe { self.raw.tag == other.raw.tag && self.raw.data.id == other.raw.data.id }
	}
}

impl Eq for Value {}

impl std::hash::Hash for Value {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		unsafe {
			self.raw.tag.hash(state);
			self.raw.data.id.hash(state);
		}
	}
}

impl Drop for Value {
	fn drop(&mut self) {
		unsafe {
			raw_types::funcs::dec_ref_count(self.raw);
		}
	}
}

impl Value {
	/// Equivalent to DM's `global.vars`.
	pub const GLOBAL: Self = Self {
		raw: raw_types::values::Value {
			tag: raw_types::values::ValueTag::World,
			data: raw_types::values::ValueData { id: 1 }
		},
		phantom: PhantomData {}
	};
	/// Equivalent to DM's null.
	pub const NULL: Self = Self {
		raw: raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { number: 0.0 }
		},
		phantom: PhantomData {}
	};
	/// Equivalent to DM's `world`.
	pub const WORLD: Self = Self {
		raw: raw_types::values::Value {
			tag: raw_types::values::ValueTag::World,
			data: raw_types::values::ValueData { id: 0 }
		},
		phantom: PhantomData {}
	};

	/// Creates a new value from raw tag and data.
	/// Use if you know what you are doing.
	pub unsafe fn new(tag: raw_types::values::ValueTag, data: raw_types::values::ValueData) -> Value {
		let raw = raw_types::values::Value { tag, data };
		raw_types::funcs::inc_ref_count(raw);

		Value {
			raw,
			phantom: PhantomData {}
		}
	}

	/// Equivalent to DM's `global.vars`.
	#[deprecated(note = "please use the `GLOBAL` const instead")]
	pub const fn globals() -> Value {
		Self::GLOBAL
	}

	/// Equivalent to DM's `world`.
	#[deprecated(note = "please use the `WORLD` const instead")]
	pub const fn world() -> Value {
		Self::WORLD
	}

	/// Equivalent to DM's `null`.
	#[deprecated(note = "please use the `NULL` const instead")]
	pub const fn null() -> Value {
		Self::NULL
	}

	/// Gets a turf by ID, without bounds checking. Use turf_by_id if you're not
	/// sure about how to check the bounds.
	pub const unsafe fn turf_by_id_unchecked(id: u32) -> Value {
		Value {
			raw: raw_types::values::Value {
				tag: raw_types::values::ValueTag::Turf,
				data: raw_types::values::ValueData { id }
			},
			phantom: PhantomData {}
		}
	}

	/// Gets a turf by ID, with bounds checking.
	pub fn turf_by_id(id: u32) -> DMResult {
		let world = Self::WORLD;
		let max_x = world.get_number(crate::byond_string!("maxx"))? as u32;
		let max_y = world.get_number(crate::byond_string!("maxy"))? as u32;
		let max_z = world.get_number(crate::byond_string!("maxz"))? as u32;
		if (0..max_x * max_y * max_z).contains(&(id - 1)) {
			Ok(unsafe { Value::turf_by_id_unchecked(id) })
		} else {
			Err(runtime!("Attempted to get tile with invalid ID {}", id))
		}
	}

	/// Gets a turf by coordinates.
	pub fn turf(x: u32, y: u32, z: u32) -> DMResult {
		let world = Self::WORLD;
		let max_x = world.get_number(crate::byond_string!("maxx"))? as u32;
		let max_y = world.get_number(crate::byond_string!("maxy"))? as u32;
		let max_z = world.get_number(crate::byond_string!("maxz"))? as u32;
		let x = x - 1; // thanks byond
		let y = y - 1;
		let z = z - 1;
		if (0..max_x).contains(&x) && (0..max_y).contains(&y) && (0..max_z).contains(&z) {
			Ok(unsafe { Value::turf_by_id_unchecked(x + y * max_x + z * max_x * max_y) })
		} else {
			Err(runtime!("Attempted to get out-of-range tile at coords {} {} {}", x + 1, y + 1, z + 1))
		}
	}

	fn get_by_id(&self, name_id: raw_types::strings::StringId) -> DMResult {
		let mut val = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			if raw_types::funcs::get_variable(&mut val, self.raw, name_id) != 1 {
				let varname: String = string::StringRef::from_id(name_id).into();
				return Err(runtime!("Could not read {}.{}", &self, varname));
			}

			Ok(Self::from_raw(val))
		}
	}

	fn set_by_id(&self, name_id: raw_types::strings::StringId, new_value: raw_types::values::Value) -> Result<(), runtime::Runtime> {
		unsafe {
			if raw_types::funcs::set_variable(self.raw, name_id, new_value) != 1 {
				let varname: String = string::StringRef::from_id(name_id).into();
				return Err(runtime!("Could not write to {}.{}", self, varname));
			}
		}
		Ok(())
	}

	/// Gets a variable by name.
	pub fn get<S: Into<string::StringRef>>(&self, name: S) -> DMResult {
		let name = name.into();
		self.get_by_id(name.get_id())
	}

	/// Gets a variable by name and safely casts it to a float.
	pub fn get_number<S: Into<string::StringRef>>(&self, name: S) -> DMResult<f32> {
		self.get(name)?.as_number()
	}

	/// Gets a variable by name and safely casts it to a string.
	pub fn get_string<S: Into<string::StringRef>>(&self, name: S) -> DMResult<String> {
		self.get(name)?.as_string()
	}

	/// Gets a variable by name and safely casts it to a [list::List].
	pub fn get_list<S: Into<string::StringRef>>(&self, name: S) -> DMResult<list::List> {
		let var = self.get(name)?;
		var.as_list()
	}

	/// Sets a variable by name to a given value.
	pub fn set<S: Into<string::StringRef>, V: Into<Value>>(&self, name: S, value: V) -> DMResult<()> {
		let value = value.into();

		self.set_by_id(name.into().get_id(), value.raw)?;
		Ok(())
	}

	/// Check if the current value is a number and casts it.
	pub fn as_number(&self) -> DMResult<f32> {
		match self.raw.tag {
			raw_types::values::ValueTag::Number => unsafe { Ok(self.raw.data.number) },
			_ => Err(runtime!("Attempt to interpret non-number value as number"))
		}
	}

	/// Check if the current value is a string and casts it.
	pub fn as_string(&self) -> DMResult<String> {
		match self.raw.tag {
			raw_types::values::ValueTag::String => unsafe { Ok(string::StringRef::from_id(self.raw.data.string).into()) },
			_ => Err(runtime!("Attempt to interpret non-string value as String"))
		}
	}

	/// Check if the current value is a list and casts it.
	pub fn as_list(&self) -> DMResult<list::List> {
		list::List::from_value(self)
	}

	/// Calls a method of the value with the given arguments.
	///
	/// # Examples:
	///
	/// This example is equivalent to `src.explode(3)` in DM.
	/// ```ignore
	/// src.call("explode", &[&Value::from(3.0)]);
	/// ```
	pub fn call<S: AsRef<str>>(&self, procname: S, args: &[&Value]) -> DMResult {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			// Increment ref-count of args permenently before passing them on
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			let procname = String::from(procname.as_ref()).replace("_", " ");
			let mut args: Vec<_> = args.iter().map(|e| e.raw).collect();
			let name_ref = string::StringRef::new(&procname)?;

			if raw_types::funcs::call_datum_proc_by_name(
				&mut ret,
				Value::NULL.raw,
				2,
				name_ref.value.raw.data.string,
				self.raw,
				args.as_mut_ptr(),
				args.len(),
				0,
				0
			) == 1
			{
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	// ugh
	pub fn to_dmstring(&self) -> DMResult<string::StringRef> {
		match self.raw.tag {
			raw_types::values::ValueTag::Null | raw_types::values::ValueTag::Number | raw_types::values::ValueTag::String => {
				return string::StringRef::new(format!("{}", self.raw).as_str())
			}

			_ => {}
		}

		let mut id = raw_types::strings::StringId(0);

		unsafe {
			if raw_types::funcs::to_string(&mut id, self.raw) != 1 {
				return Err(runtime!("to_string failed on {:?}", self));
			}
			Ok(string::StringRef::from_id(id))
		}
	}

	pub fn to_string(&self) -> DMResult<String> {
		match self.raw.tag {
			raw_types::values::ValueTag::Null | raw_types::values::ValueTag::Number | raw_types::values::ValueTag::String => {
				return Ok(format!("{}", self.raw))
			}

			_ => {}
		}

		let mut id = raw_types::strings::StringId(0);

		unsafe {
			if raw_types::funcs::to_string(&mut id, self.raw) != 1 {
				return Err(runtime!("to_string failed on {:?}", self));
			}
			Ok(String::from(string::StringRef::from_id(id)))
		}
	}

	/// Gets the type of the Value as a string
	pub fn get_type(&self) -> Result<String, runtime::Runtime> {
		self.get(crate::byond_string!("type"))?.to_string()
	}

	/// Checks whether this Value's type is equal to `typepath`.
	pub fn is_exact_type<S: AsRef<str>>(&self, typepath: S) -> bool {
		match self.get_type() {
			Err(_) => false,
			Ok(my_type) => my_type == typepath.as_ref()
		}
	}

	pub fn is_truthy(&self) -> bool {
		match self.raw.tag {
			raw_types::values::ValueTag::Null => false,
			raw_types::values::ValueTag::Number => unsafe { self.raw.data.number != 0.0 },

			_ => true
		}
	}

	/// Creates a Value that references a byond string.
	/// Will panic if the given string contains null bytes
	///
	/// # Examples:
	/// ```ignore
	/// let my_string = Value::from_string("Testing!");
	/// ```
	pub fn from_string<S: AsRef<str>>(data: S) -> DMResult {
		let string = CString::new(data.as_ref()).map_err(|_| runtime!("tried to create string containing NUL"))?;

		unsafe {
			let mut id = raw_types::strings::StringId(0);

			assert_eq!(raw_types::funcs::get_string_id(&mut id, string.as_ptr()), 1);

			Ok(Value::new(raw_types::values::ValueTag::String, raw_types::values::ValueData {
				string: id
			}))
		}
	}

	pub fn from_string_raw(data: &[u8]) -> DMResult {
		let string = CString::new(data).map_err(|_| runtime!("tried to create string containing NUL"))?;

		unsafe {
			let mut id = raw_types::strings::StringId(0);

			assert_eq!(raw_types::funcs::get_string_id(&mut id, string.as_ptr()), 1);

			Ok(Value::new(raw_types::values::ValueTag::String, raw_types::values::ValueData {
				string: id
			}))
		}
	}

	/// blah blah lifetime is not verified with this so use at your peril
	pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
		Value::new(v.tag, v.data)
	}

	/// same as from_raw but does not increment the reference count (assumes we
	/// already own this reference)
	pub const unsafe fn from_raw_owned(v: raw_types::values::Value) -> Value {
		Value {
			raw: v,
			phantom: PhantomData {}
		}
	}
}

impl Clone for Value {
	fn clone(&self) -> Value {
		unsafe { Value::from_raw(self.raw) }
	}
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.raw)
	}
}

impl fmt::Debug for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:?}", self.raw)
	}
}
