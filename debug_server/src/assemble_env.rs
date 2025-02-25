use auxtools::*;

pub struct AssembleEnv;

impl dmasm::assembler::AssembleEnv for AssembleEnv {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		let string = StringRef::from_raw(string).ok()?;
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

	// This is... pretty crazy
	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		let path = Value::from_string(path).ok()?;
		let expr = dmasm::compiler::compile_expr("text2path(name)", &["name"]).unwrap();
		let assembly = dmasm::assembler::assemble(&expr, &mut Self).unwrap();

		let proc = Proc::find("/proc/auxtools_expr_stub")?;
		proc.set_bytecode(assembly);

		let res = proc.call(&[&path]).unwrap().as_list().unwrap().get(1).unwrap();

		if res == Value::NULL {
			return None;
		}

		Some((res.raw.tag as u8, unsafe { res.raw.data.id }))
	}
}
