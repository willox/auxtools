use auxtools::*;

pub struct AssembleEnv;

impl AssembleEnv {
	// Runs `code` in the expression stub with `name` set to `arg` and returns
	// what it evaluates to
	fn eval_with_name(code: &str, arg: &str) -> Option<Value> {
		let arg = Value::from_string(arg).ok()?;
		Self::eval_with(code, &[("name", &arg)])
	}

	// Runs `code` in the expression stub with the names given set, and returns
	// what it evaluates to, unless that's null
	fn eval_with(code: &str, params: &[(&str, &Value)]) -> Option<Value> {
		let names: Vec<&str> = params.iter().map(|(name, _)| *name).collect();
		let values: Vec<&Value> = params.iter().map(|(_, value)| *value).collect();
		let expr = dmasm::compiler::compile_expr_for(code, &names, crate::world_byond()).unwrap();
		let assembly = dmasm::assembler::assemble(&expr, &mut Self).unwrap();

		let proc = Proc::find("/proc/auxtools_expr_stub")?;
		proc.set_bytecode(assembly);

		let res = proc.call(&values).ok()?.as_list().ok()?.get(1).ok()?;

		if res == Value::NULL {
			return None;
		}

		Some(res)
	}

	/// Whether `object` has a proc or verb called `name`, its own or
	/// inherited
	pub fn has_proc(object: &Value, name: &str) -> bool {
		let Ok(name) = Value::from_string(name) else {
			return false;
		};
		Self::eval_with("hascall(object, name)", &[("object", object), ("name", &name)]).is_some_and(|has| has.is_truthy())
	}
}

impl dmasm::assembler::AssembleEnv for AssembleEnv {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		let string = StringRef::from_raw(string).ok()?;
		let id = string.get_id();

		// We leak here because the assembled code now holds a reference to this
		// string
		std::mem::forget(string);

		Some(id.0)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		let id = self.get_string_index(name)?;

		unsafe {
			let mut names = *raw_types::funcs::VARIABLE_NAME_ENTRIES;

			for i in 0..*raw_types::funcs::VARIABLE_NAME_COUNT {
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
		match path {
			// text2path() returns null for these two
			"/list" => return Some((raw_types::values::ValueTag::ListTypepath as u8, 0)),
			"/savefile" => return Some((raw_types::values::ValueTag::SaveFileTypepath as u8, 0)),
			// text2path() gives /client's path id as the data, but the code
			// DreamMaker compiles always has 0
			"/client" => return Some((raw_types::values::ValueTag::ClientTypepath as u8, 0)),
			_ => {}
		}

		if let Some(res) = Self::eval_with_name("text2path(name)", path) {
			return Some((res.raw.tag as u8, unsafe { res.raw.data.id }));
		}
		if let Some(id) = crate::hot_reload::image_type(path) {
			return Some((raw_types::values::ValueTag::ImageTypepath as u8, id));
		}

		// A reference to a proc written without `/proc/`, like /obj/foo/bar,
		// is one text2path() doesn't know
		Proc::find(path).map(|proc| (raw_types::values::ValueTag::ProcRef as u8, proc.id.0))
	}

	// fcopy_rsc() hands back the cache entry a file already has, and adds the
	// file to the cache if it isn't there yet
	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		let res = Self::eval_with_name("fcopy_rsc(name)", path).or_else(|| Self::eval_with_name("fcopy_rsc(name)", &case_insensitive_path(path)?))?;

		if res.raw.tag != raw_types::values::ValueTag::Resource {
			return None;
		}

		Some(unsafe { res.raw.data.id })
	}
}

/// Resolve resource paths case-insensitively, matching DreamMaker on Linux.
/// Prefer exact path components, then the first sorted case-insensitive match.
/// Return `None` if the path already exists or any component cannot be
/// resolved.
fn case_insensitive_path(path: &str) -> Option<String> {
	use std::path::{Component, Path, PathBuf};

	if Path::new(path).exists() {
		return None;
	}
	let mut found = PathBuf::new();
	for component in Path::new(path).components() {
		let Component::Normal(part) = component else {
			found.push(component);
			continue;
		};
		let exact = found.join(part);
		if exact.symlink_metadata().is_ok() {
			found = exact;
			continue;
		}
		let part = part.to_str()?;
		let dir = if found.as_os_str().is_empty() { Path::new(".") } else { found.as_path() };
		let mut names: Vec<String> = std::fs::read_dir(dir)
			.ok()?
			.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
			.filter(|name| name.eq_ignore_ascii_case(part))
			.collect();
		names.sort();
		found.push(names.into_iter().next()?);
	}
	found.to_str().map(|found| found.replace('\\', "/"))
}
