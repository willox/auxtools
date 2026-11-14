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

/// Find the last definition at `/type/name`, as a call on that type would.
fn last_definition(path: &str) -> Option<Proc> {
	let mut last = None;
	let mut n = 0;
	while let Some(proc) = Proc::find_override(path, n) {
		last = Some(proc);
		n += 1;
	}
	last
}

/// Type paths from `ty` through its actual parents, nearest first.
/// Follow the runtime table, including builtin and explicit parent_type
/// ancestry. Fall back to path prefixes if the table is unavailable.
fn ancestry(ty: &str) -> Vec<String> {
	let ty = ty.trim_end_matches('/');
	if let Some(paths) = crate::hot_reload::ancestry(ty) {
		return paths;
	}
	let mut paths = vec![];
	let mut at = ty;
	while !at.is_empty() {
		paths.push(at.to_owned());
		at = at.rsplit_once('/').map_or("", |(parent, _)| parent);
	}
	paths
}

fn found(proc: Proc, warning: Option<String>) -> dmasm::compiler::FoundProc {
	dmasm::compiler::FoundProc {
		tag: raw_types::values::ValueTag::ProcRef as u8,
		data: proc.id.0,
		warning
	}
}

/// Resolve `ty::name()` to the nearest definition in the type ancestry,
/// matching DreamMaker.
pub fn type_proc(ty: &str, name: &str) -> Result<dmasm::compiler::FoundProc, String> {
	if dmasm::assembler::AssembleEnv::get_type(&mut AssembleEnv, ty).is_none() {
		return Err(format!("there's no type {} in the running world", ty));
	}
	ancestry(ty)
		.iter()
		.find_map(|at| last_definition(&format!("{}/{}", at, name)))
		.map(|proc| found(proc, None))
		.ok_or_else(|| format!("{} has no proc called {}", ty, name))
}

/// Resolve `/type/proc/name` or `/type/verb/name` to the first definition on
/// the original declaring type. Warn if that type differs from the written
/// path, which DreamMaker rejects. Only `/proc/name` resolves a global proc.
pub fn proc_path(path: &str) -> Option<dmasm::compiler::FoundProc> {
	let (ty, name) = path.rsplit_once("/proc/").or_else(|| path.rsplit_once("/verb/"))?;
	if name.is_empty() || name.contains('/') {
		return None;
	}
	if ty.is_empty() {
		return Proc::find(format!("/{}", name)).map(|proc| found(proc, None));
	}
	let (at, proc) = ancestry(ty)
		.into_iter()
		.filter_map(|at| Proc::find(format!("{}/{}", at, name)).map(|proc| (at, proc)))
		.last()?;
	let warning = (at != ty).then(|| {
		format!(
			"{}: DreamMaker refuses this path, since {} isn't declared on {}; this is {}/proc/{}",
			path, name, ty, at, name
		)
	});
	Some(found(proc, warning))
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

		// Without `/proc/`, a proc path names the last definition, matching
		// DreamMaker. text2path() does not resolve this form.
		last_definition(path).map(|proc| (raw_types::values::ValueTag::ProcRef as u8, proc.id.0))
	}

	// fcopy_rsc() hands back the cache entry a file already has, and adds the
	// file to the cache if it isn't there yet
	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		// Before resources load, fcopy_rsc() allocates IDs the build later
		// reuses for other files (see `world_resources`).
		if !crate::world_resources::is_loaded() {
			return crate::world_resources::built_id(&resource_file(path)?);
		}
		let res = resource_paths(path).find_map(|path| {
			Self::eval_with_name("fcopy_rsc(name)", &path).or_else(|| Self::eval_with_name("fcopy_rsc(name)", &case_insensitive_path(&path)?))
		})?;

		if res.raw.tag != raw_types::values::ValueTag::Resource {
			return None;
		}

		Some(unsafe { res.raw.data.id })
	}
}

/// The `FILE_DIR` directories of the code being reloaded (see
/// `HotReloadRequest::resource_dirs`)
pub(crate) static RESOURCE_DIRS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(vec![]);

/// Search for resources in the world's directory, then in `FILE_DIR` order.
fn resource_paths(path: &str) -> impl Iterator<Item = String> + '_ {
	let dirs = RESOURCE_DIRS.lock().map(|dirs| dirs.clone()).unwrap_or_default();
	std::iter::once(path.to_owned()).chain(dirs.into_iter().map(move |dir| format!("{}/{}", dir.trim_end_matches('/'), path)))
}

/// Resolve a resource path relative to the world's directory, matching
/// `get_resource_index`.
pub(crate) fn resource_file(path: &str) -> Option<std::path::PathBuf> {
	resource_paths(path).find_map(|path| {
		let file = std::path::Path::new(&path);
		if file.is_file() {
			return Some(file.to_owned());
		}
		case_insensitive_path(&path).map(Into::into)
	})
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
