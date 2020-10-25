use std::os::raw::c_char;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct StringId(pub u32);

#[repr(C)]
#[derive(Debug)]
pub struct StringEntry {
	pub data: *mut c_char,
	pub this: StringId,
	pub left: *mut StringEntry,
	pub right: *mut StringEntry,
	pub ref_count: u32,
	pub unk_1: u32,
	pub unk_2: u32,
}
