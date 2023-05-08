use auxtools::*;
use dmasm;

pub struct DisassembleEnv;

impl dmasm::disassembler::DisassembleEnv for DisassembleEnv {
	fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
		unsafe {
			Some(
				StringRef::from_id(raw_types::strings::StringId(index))
					.data()
					.to_vec(),
			)
		}
	}

	fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
		unsafe {
			Some(
				StringRef::from_variable_id(raw_types::strings::VariableId(index))
					.data()
					.to_vec(),
			)
		}
	}

	fn get_proc_name(&mut self, index: u32) -> Option<String> {
		Proc::from_id(raw_types::procs::ProcId(index)).map(|x| x.path)
	}

	fn value_to_string_data(&mut self, tag: u32, data: u32) -> Option<Vec<u8>> {
		unsafe {
			let value = Value::new(std::mem::transmute(tag as u8), std::mem::transmute(data));
			match value.to_dmstring() {
				Ok(s) => Some(s.data().to_vec()),
				_ => None,
			}
		}
	}
}
