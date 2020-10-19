use super::super::string;
use super::strings;
use std::fmt;

#[repr(u8)]
#[derive(PartialEq, Copy, Clone, Debug)]
#[allow(unused)]
#[non_exhaustive]
pub enum ValueTag {
	Null,   // 0x00
	Turf,   // 0x01
	Obj,    // 0x02
	Mob,    // 0x03
	Area,   // 0x04
	Client, // 0x05
	String, // 0x06

	MobTypepath = 0x08, // 0x08
	ObjTypepath,        // 0x09
	TurfTypepath,       // 0x0A
	AreaTypepath,       // 0x0B
	Resource,           // 0x0C
	Image,              // 0x0D
	World,              // 0x0E
	List,               // 0x0F

	Number = 0x2A, // 0x2A
}

impl fmt::Display for ValueTag {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		//write!(f, "{:?}", self)
		write!(f, "TODO")
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union ValueData {
	pub string: strings::StringId,
	pub number: f32,
	pub id: u32,
}

/// Internal thing used when interfacing with BYOND. You shouldn't need to use this.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Value {
	pub tag: ValueTag,
	pub data: ValueData,
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.tag == ValueTag::Number {
			write!(f, "({}, {})", self.tag, unsafe { self.data.number })
		} else if self.tag == ValueTag::String {
			write!(f, "TODO")
			//let content: String = unsafe { string::StringRef::from_id(self.data.id).into() };
			//write!(f, "({}, {})", self.tag, content)
		} else {
			write!(f, "({}, {})", self.tag, unsafe { self.data.id })
		}
	}
}

pub trait IntoRawValue {
	unsafe fn into_raw_value(self) -> Value;
}

impl IntoRawValue for f32 {
	unsafe fn into_raw_value(self) -> Value {
		Value {
			tag: ValueTag::Number,
			data: ValueData { number: self },
		}
	}
}
