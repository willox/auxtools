use super::funcs;
use super::lists;
use super::strings;
use std::ffi::CStr;
use std::fmt;

#[repr(u8)]
#[derive(PartialEq, Copy, Clone, Debug, Hash)]
#[non_exhaustive]
pub enum ValueTag {
	Null = 0x00,
	Turf = 0x01,
	Obj = 0x02,
	Mob = 0x03,
	Area = 0x04,
	Client = 0x05,
	String = 0x06,

	MobTypepath = 0x08,
	ObjTypepath = 0x09,
	TurfTypepath = 0x0A,
	AreaTypepath = 0x0B,
	Resource = 0x0C,
	Image = 0x0D,
	World = 0x0E,

	// Lists
	List = 0x0F,
	MobVars = 0x2C,
	ObjVars = 0x2D,
	TurfVars = 0x2E,
	AreaVars = 0x2F,
	ClientVars = 0x30,
	Vars = 0x31,
	MobOverlays = 0x32,
	MobUnderlays = 0x33,
	ObjOverlays = 0x34,
	ObjUnderlays = 0x35,
	TurfOverlays = 0x36,
	TurfUnderlays = 0x37,
	AreaOverlays = 0x38,
	AreaUnderlays = 0x39,
	ImageVars = 0x42,
	WorldVars = 0x51,
	GlobalVars = 0x52,

	Datum = 0x21,
	SaveFile = 0x23,

	Number = 0x2A,
	Appearance = 0x3A,
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		unsafe {
			match self.tag {
				ValueTag::Null => write!(f, "{}", "null"),
				ValueTag::Number => write!(f, "{}", self.data.number),
				ValueTag::String => {
					let id = self.data.string;
					let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
					assert_eq!(funcs::get_string_table_entry(&mut entry, id), 1);
					write!(f, "{:?}", CStr::from_ptr((*entry).data).to_string_lossy())
				}
				_ => write!(f, "Value({}, {})", self.tag as u8, self.data.id),
			}
		}
	}
}

impl fmt::Debug for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		unsafe {
			match self.tag {
				ValueTag::Null => write!(f, "{}", "null"),
				ValueTag::Number => write!(f, "{:?}", self.data.number),
				ValueTag::String => {
					let id = self.data.string;
					let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
					assert_eq!(funcs::get_string_table_entry(&mut entry, id), 1);
					write!(f, "{:?}", CStr::from_ptr((*entry).data).to_string_lossy())
				}
				_ => write!(f, "Value({}, {})", self.tag as u8, self.data.id),
			}
		}
	}
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
	pub list: lists::ListId,
}

/// Internal thing used when interfacing with BYOND. You shouldn't need to use this.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Value {
	pub tag: ValueTag,
	pub data: ValueData,
}
