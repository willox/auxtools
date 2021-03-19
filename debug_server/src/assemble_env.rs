use auxtools::*;
use dmasm;

pub struct AssembleEnv;

impl dmasm::assembler::AssembleEnv for AssembleEnv {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		let string = StringRef::from_raw(string);
		let id = string.get_id();

		// We leak here because the assembled code now holds a reference to this string
		std::mem::forget(string);

		Some(id.0)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		let id = self.get_string_index(name)?;

		unsafe {
			let mut names = (*raw_types::funcs::VARIABLE_NAMES).entries;

			for i in 0..(*raw_types::funcs::VARIABLE_NAMES).count {
				if (*names).0 == id {
					return Some(i);
				}

				names = names.add(1);
			}

			None
		}
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		Proc::find(path).map(|p| p.id.0)
	}

	// TODO: Replace with something better
	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		let typeval = Proc::find("/proc/auxtools_text2path_wrapper")
			.unwrap()
			.call(&[&Value::from_string(path)])
			.unwrap();
		Some((typeval.value.tag as u8, unsafe { typeval.value.data.id }))
	}
}
