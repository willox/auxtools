use super::values;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ListId(pub u32);

#[repr(C)]
#[allow(unused)]
enum Color {
	Red = 0,
	Black = 1,
}

#[repr(C)]
pub struct AssociativeListEntry {
	key: values::Value,
	value: values::Value,
	color: Color,
	left: *mut AssociativeListEntry,
	right: *mut AssociativeListEntry,
}

#[repr(C)]
pub struct List {
	pub vector_part: *mut values::Value,
	pub assoc_part: *mut AssociativeListEntry,
	pub allocated: u32,
	pub length: u32,
	pub refcount: u32,
	unknown: u32,
}
