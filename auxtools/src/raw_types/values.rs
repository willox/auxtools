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

	// Lists
	List = 0x0F,
	ArgList = 0x10,
	MobVerbs = 0x11,
	ObjVerbs = 0x12,
	TurfVerbs = 0x13,
	AreaVerbs = 0x14,
	ClientVerbs = 0x15,
	SaveFileDir = 0x16,
	MobContents = 0x17,
	TurfContents = 0x18,
	AreaContents = 0x19,
	WorldContents = 0x1A,
	MobGroup = 0x1B,
	ObjContents = 0x1C,

	DatumTypepath = 0x20,
	Datum = 0x21,
	SaveFile = 0x23,
	SaveFileTypepath = 0x24,
	ProcRef = 0x26,
	File = 0x27,
	// what `var/list/L = new()` pushes as its type. islist() says no to it
	ListTypepath = 0x28,
	// a typepath plus var overrides, applied after `new`. extools' name; the
	// binary never names it
	Prefab = 0x29,
	Number = 0x2A,
	// what `F["key"]` evaluates to: a savefile id plus the key's string id
	SaveFileEntry = 0x2B,
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
	ClientTypepath = 0x3B,
	Pointer = 0x3C,
	// internal: the recorder `optimize_obj_type_init_procs` writes var inits
	// into. DM code never sees it
	InitVarRecorder = 0x3D,
	// internal: a name constant that only DM-compiler code builds
	CompilerNameConstant = 0x3E,
	// also what an appearance's `type` comes back as
	ImageTypepath = 0x3F,
	ImageOverlays = 0x40,
	ImageUnderlays = 0x41,
	ImageVars = 0x42,
	ImageVerbs = 0x43,
	ImageContents = 0x44,
	// What `load_ext()` hands back: a reference to a resolved function in an
	// external library. It is really any native-object handle: /icon and /regex
	// objects are this tag too, told apart by a kind at `+8`.
	ExtFunc = 0x45,
	ClientImages = 0x46,
	ClientScreen = 0x47,
	// BYOND's own inc/dec_ref_count do nothing with this one
	SaveFileVars = 0x48,
	// the vars of a /list or /alist. The data is a string id, so it is
	// refcounted like a string
	ListVars = 0x49,
	// internal, and no DM-visible name anywhere: an entry in a per-client
	// resource set
	Unnamed4A = 0x4A,
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
	// the typepath of the 516 builtins: /alist, /pixloc, /vector, /callee
	BuiltinTypepath = 0x59
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
