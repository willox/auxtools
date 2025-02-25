use std::os::raw::c_char;

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub struct StringId(pub u32);

impl StringId {
	pub const fn valid(&self) -> bool {
		self.0 != 0xFFFF
	}
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VariableId(pub u32);

#[repr(C)]
#[derive(Debug)]
pub struct StringEntry {
	pub data: *mut c_char,
	pub this: StringId,
	pub left: *mut StringEntry,
	pub right: *mut StringEntry,
	pub ref_count: u32,
	pub unk_1: u32,
	pub unk_2: u32
}
