use super::values;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ListId(pub u32);

/// One key->value pair of an associative list, as a red-black tree node.
///
/// 32 bytes. Matches BYOND's own tree lookup (516.1669), which reads the key at
/// `+0` and the children at `+16` and `+20`. This used to declare `color` at
/// `+16`, which put both child pointers 4 bytes past where BYOND writes them.
/// Nothing read the children, so it never showed.
#[repr(C)]
pub struct AssociativeListEntry {
	/// Non-owning; the same ref also occupies a `vector_part` slot.
	pub key: values::Value,
	/// Owning; released when the tree is torn down.
	pub value: values::Value,
	pub left: *mut AssociativeListEntry,
	pub right: *mut AssociativeListEntry,
	pub parent: *mut AssociativeListEntry,
	/// 0 = red, 1 = black.
	pub color: u8
}

#[repr(C)]
pub struct List {
	pub vector_part: *mut values::Value,
	pub assoc_part: *mut AssociativeListEntry,
	pub allocated: u32,
	pub length: u32,
	pub refcount: u32,
	unknown: u32
}
