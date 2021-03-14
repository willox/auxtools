use super::strings;

#[repr(C)]
pub struct VariableNameIdTable {
	pub entries: *const strings::StringId,
	pub count: u32,
}
