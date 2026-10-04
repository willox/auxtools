use super::strings;

// Not one BYOND struct, just two globals that sit next to each other: the name
// array and how many entries it has. `symbols` finds `entries` and backs up to
// wherever this starts.
#[cfg(windows)]
#[repr(C)]
pub struct VariableNameIdTable {
	pub entries: *const strings::StringId,
	pub count: u32
}

// On Linux the count comes first. What follows `entries` there is the pointer
// to the global values, which is not a count of anything.
#[cfg(unix)]
#[repr(C)]
pub struct VariableNameIdTable {
	pub count: u32,
	pub entries: *const strings::StringId
}
