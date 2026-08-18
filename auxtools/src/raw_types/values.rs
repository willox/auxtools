use super::{funcs, lists, strings};
use std::{ffi::CStr, fmt};

macro_rules! value_tags {
	(
		$($variant:ident = $val:expr),* $(,)?
	) => {
		#[repr(u8)]
		#[derive(PartialEq, Copy, Clone, Debug, Hash)]
		#[non_exhaustive]
		pub enum ValueTag {
			$($variant = $val),*
		}

		impl ValueTag {
			pub const fn from_u8(value: u8) -> Option<Self> {
				match value {
					$($val => Some(Self::$variant),)*
					_ => None,
				}
			}
		}
	};
}

value_tags! {
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

	List = 0x0F,
	// Lists
	ArgList = 0x10,
	Unk0 = 0x11,
	Unk1 = 0x12,
	Unk2 = 0x13,
	Unk3 = 0x14,
	Unk4 = 0x15,
	Unk5 = 0x16,

	MobContents = 0x17,
	TurfContents = 0x18,
	AreaContents = 0x19,
	WorldContents = 0x1A,
	ObjContents = 0x1C,

	DatumTypepath = 0x20,
	Unk6 = 0x22,
	Unk7 = 0x24,
	Unk8 = 0x25,

	Unk9 = 0x28,
	Unk10 = 0x29,

	Datum = 0x21,
	SaveFile = 0x23,
	ProcRef = 0x26,
	File = 0x27,
	Number = 0x2A,
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
	Appearance = 0x3A,
	Unk11 = 0x3B,
	Pointer = 0x3C,
	Unk12 = 0x3D,
	Unk13  = 0x3E,
	Unk14  = 0x3F,

	ImageOverlays = 0x40,
	ImageUnderlays = 0x41,
	ImageVars = 0x42,
	TurfVisContents = 0x4B,
	ObjVisContents = 0x4C,
	MobVisContents = 0x4D,
	TurfVisLocs = 0x4E,
	ObjVisLocs = 0x4F,
	MobVisLocs = 0x50,
	WorldVars = 0x51,
	GlobalVars = 0x52,
	Filters = 0x53,
	ImageVisContents = 0x54,
	Alist = 0x55,
	PixLoc = 0x56,
	Vector = 0x57,
	Callee = 0x58,
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		unsafe {
			match self.tag {
				ValueTag::Null => write!(f, "null"),
				ValueTag::Number => write!(f, "{}", self.data.number),
				ValueTag::String => {
					let id = self.data.string;
					let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
					assert_eq!(funcs::get_string_table_entry(&mut entry, id), 1);
					write!(f, "{:?}", CStr::from_ptr((*entry).data).to_string_lossy())
				}
				_ => write!(f, "Value({}, {})", self.tag as u8, self.data.id)
			}
		}
	}
}

impl fmt::Debug for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		unsafe {
			match self.tag {
				ValueTag::Null => write!(f, "null"),
				ValueTag::Number => write!(f, "{:?}", self.data.number),
				ValueTag::String => {
					let id = self.data.string;
					let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
					assert_eq!(funcs::get_string_table_entry(&mut entry, id), 1);
					write!(f, "{:?}", CStr::from_ptr((*entry).data).to_string_lossy())
				}
				_ => write!(f, "Value({}, {})", self.tag as u8, self.data.id)
			}
		}
	}
}

impl fmt::Display for ValueTag {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// write!(f, "{:?}", self)
		write!(f, "TODO")
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union ValueData {
	pub string: strings::StringId,
	pub number: f32,
	pub id: u32,
	pub list: lists::ListId
}

/// Internal thing used when interfacing with BYOND. You shouldn't need to use
/// this.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Value {
	pub tag: ValueTag,
	pub data: ValueData
}
