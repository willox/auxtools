use super::strings;

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct MiscId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BytecodeId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct LocalsId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ParametersId(u32);

pub trait AsMiscId {
	fn as_misc_id(&self) -> MiscId;
}

impl AsMiscId for BytecodeId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

impl AsMiscId for LocalsId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

impl AsMiscId for ParametersId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union Misc {
	pub bytecode: Bytecode,
	pub locals: Locals,
	pub parameters: Parameters,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Bytecode {
	pub count: u16,
	pub bytecode: *mut u32,
	unk_0: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Locals {
	pub count: u16,
	pub names: *const strings::VariableId,
	unk_0: u32,
}

#[repr(C)]
pub struct ParametersData {
	unk_0: u32,
	unk_1: u32,
	pub name: strings::VariableId,
	unk_4: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Parameters {
	params_count_mul_4: u16,
	pub data: *const ParametersData,
	unk_0: u32,
}

impl Parameters {
	pub fn count(&self) -> u16 {
		self.params_count_mul_4 / 4
	}
}
