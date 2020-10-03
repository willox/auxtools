use super::strings;
use std::fmt;
use std::marker::PhantomData;

#[repr(u8)]
#[derive(PartialEq)]
pub enum ValueTag {
	Null,   // 0x00
	Turf,   // 0x01
	Obj,    // 0x02
	Mob,    // 0x03
	Area,   // 0x04
	Client, // 0x05
	String, // 0x06

	Number, // 0x2A
}

impl fmt::Display for ValueTag {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			ValueTag::Null => write!(f, "Null"),
			ValueTag::Turf => write!(f, "Turf"),
			ValueTag::Obj => write!(f, "Obj"),
			ValueTag::Mob => write!(f, "Mob"),
			ValueTag::Area => write!(f, "Area"),
			ValueTag::Client => write!(f, "Client"),
			ValueTag::String => write!(f, "String"),
			ValueTag::Number => write!(f, "Number"),
			_ => write!(f, "Unknown-type"),
		}
	}
}

#[repr(C)]
pub union ValueData {
	pub string: strings::StringRef,
	pub number: f32,
	pub id: u32,
}

#[repr(C)]
pub struct Value {
	pub tag: ValueTag,
	pub data: ValueData,
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.tag == ValueTag::Number {
			write!(f, "({}, {})", self.tag, unsafe { self.data.number })
		} else {
			write!(f, "({}, {})", self.tag, unsafe { self.data.id })
		}
	}
}
