use super::super::string;
use super::super::value;
use super::strings;
use std::fmt;

#[repr(u8)]
#[derive(PartialEq, Copy, Clone)]
#[allow(unused)]
pub enum ValueTag {
    Null,   // 0x00
    Turf,   // 0x01
    Obj,    // 0x02
    Mob,    // 0x03
    Area,   // 0x04
    Client, // 0x05
    String, // 0x06

    Number = 0x2A, // 0x2A
}

#[allow(unreachable_patterns)]
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
#[derive(Copy, Clone)]
pub union ValueData {
    pub string: strings::StringId,
    pub number: f32,
    pub id: u32,
}

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
            let content: String = string::StringRef::from_id(unsafe { self.data.id }).into();
            write!(f, "({}, {})", self.tag, content)
        } else {
            write!(f, "({}, {})", self.tag, unsafe { self.data.id })
        }
    }
}

impl From<&value::Value<'_>> for Value {
    fn from(val: &value::Value) -> Self {
        Value {
            tag: val.value.tag,
            data: val.value.data,
        }
    }
}

pub trait IntoRawValue {
    unsafe fn into_raw_value(&self) -> Value;
}
