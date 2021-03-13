use auxtools::*;
use dmasm;

pub struct AssembleEnv;

impl dmasm::assembler::AssembleEnv for AssembleEnv {
    fn get_string_index(&mut self, string: &[u8]) -> u32 {
        let string = StringRef::from_raw(string);
		let id = string.get_id();

		// We leak here because the assembled code now holds a reference to this string
		std::mem::forget(string);

		id.0
    }

    fn get_variable_name_index(&mut self, name: &[u8]) -> u32 {
		let id = self.get_string_index(name);

		unsafe {
			// TODO: We don't know the length of the VARIABLE_NAMES array...
        	let mut names = raw_types::funcs::VARIABLE_NAMES;
			let mut count = 0;

			loop {
				if (*names).0 == id {
					return count;
				}

				names = names.add(1);
				count += 1;
			}
		}
    }

	fn get_proc_index(&mut self, path: &str) -> u32 {
		// TODO: error handle
		let proc = Proc::find(path).unwrap();
		proc.id.0
	}
}
