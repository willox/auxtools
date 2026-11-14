//! Resolve resources safely while a launched world loads.
//!
//! The debugger starts before BYOND loads its resources. fcopy_rsc() would
//! allocate IDs later reused by the build's resources, pointing reloaded
//! code at unrelated files. Until `world/New`, resolve existing resources
//! by content using the `.dmb` resource table. Defer reloads introducing
//! other files until `world/New` (see `Server::world_data_loaded`).
//!
//! The final `.dmb` table maps IDs to BYOND CRCs (`dmb_tables`); the `.rsc`
//! uses those CRCs to identify file data.

use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	sync::{
		atomic::{AtomicBool, Ordering},
		Mutex
	}
};

use dmasm::reload::{ConstValue, InList, Param, StaticVar};

use crate::server_types::HotReloadRequest;

/// Whether resources have loaded. Worlds not launched by the debugger
/// load the library from their own code, after resources are available.
static LOADED: AtomicBool = AtomicBool::new(true);

/// Lazy resource CRC table from the build, refreshed after world loading.
static BUILT: Mutex<Option<Option<Vec<u32>>>> = Mutex::new(None);

/// A launched world is loading: resources resolve from its build until
/// `world/New`.
pub fn loading() {
	LOADED.store(false, Ordering::Relaxed);
	*BUILT.lock().unwrap() = None;
}

/// `world/New` is starting: the world has its resources.
pub fn loaded() {
	LOADED.store(true, Ordering::Relaxed);
	*BUILT.lock().unwrap() = None;
}

pub fn is_loaded() -> bool {
	LOADED.load(Ordering::Relaxed)
}

/// Resource ID for `path`'s current content in the build, relative to the
/// world directory. Return none for unknown content or an unreadable table.
pub fn built_id(path: &Path) -> Option<u32> {
	let crc = crate::dmb_tables::byond_crc(&std::fs::read(path).ok()?);
	let mut built = BUILT.lock().unwrap();
	let crcs = built.get_or_insert_with(read_built).as_ref()?;
	crcs.iter().position(|&built| built == crc).map(|id| id as u32)
}

/// Number of built resources; smaller IDs belong to the build.
/// Return none if the table cannot be read reliably.
pub fn built_count() -> Option<usize> {
	let mut built = BUILT.lock().unwrap();
	built.get_or_insert_with(read_built).as_ref().map(Vec::len)
}

fn read_built() -> Option<Vec<u32>> {
	let dmb = world_dmb()?;
	crate::dmb_tables::resource_crcs(&std::fs::read(&dmb).ok()?).ok()
}

/// Lazy set of resource file names in the build.
static BUILT_NAMES: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Whether `file` is named in the build's resources, regardless of its
/// current content. Paths are relative to the world directory.
pub fn built_file(file: &Path) -> bool {
	let names = BUILT_NAMES
		.lock()
		.unwrap()
		.get_or_insert_with(|| read_names().unwrap_or_default())
		.clone();
	names.iter().any(|name| crate::assemble_env::resource_file(name).as_deref() == Some(file))
}

/// The names of the `.rsc`'s records in use: `u32 length, u8 in use, u8 type,
/// u32 crc, u32 time, u32 time, u32 data length, name, data`
fn read_names() -> Option<Vec<String>> {
	let data = std::fs::read(world_dmb()?.with_extension("rsc")).ok()?;
	let mut names = vec![];
	let mut at = 0;
	while at + 5 <= data.len() {
		let length = u32::from_le_bytes(data[at..at + 4].try_into().ok()?) as usize;
		let record = &data[at + 4..(at + 5 + length).min(data.len())];
		at += 5 + length;
		if record.len() < 19 || record[0] == 0 {
			continue;
		}
		if let Some(end) = record[18..].iter().position(|&b| b == 0) {
			let name = String::from_utf8_lossy(&record[18..18 + end]).replace('\\', "/");
			if !name.is_empty() && !names.contains(&name) {
				names.push(name);
			}
		}
	}
	Some(names)
}

/// The .dmb the world runs, from its command line
fn world_dmb() -> Option<PathBuf> {
	let (args, cwd) = crate::launched::command_line();
	let dmb = args.iter().skip(1).find(|arg| arg.to_ascii_lowercase().ends_with(".dmb"))?;
	Some(Path::new(&cwd).join(dmb))
}

/// Resource files referenced by `request` but absent from the build.
/// These need resource IDs allocated at `world/New`.
pub fn not_built(request: &HotReloadRequest) -> Vec<String> {
	let mut named = Named::default();
	for new in &request.new_types {
		for (_, field) in &new.fields {
			named.value(field);
		}
	}
	for proc in request.procs.iter().chain(request.new_procs.iter().map(|new| &new.code)) {
		named.code(&proc.nodes, &proc.params, &proc.statics);
	}
	for init in request.init_procs.iter().filter_map(|init| init.code.as_ref()) {
		named.code(&init.nodes, &init.params, &init.statics);
	}
	for var in &request.new_vars {
		named.value(&var.value);
		if let Some(init) = &var.init {
			named.code(&init.nodes, &init.params, &init.statics);
		}
	}
	for default in &request.var_defaults {
		named.value(&default.value);
	}
	for default in &request.builtin_defaults {
		named.value(&default.value);
	}

	named
		.0
		.into_iter()
		.filter(|path| crate::assemble_env::resource_file(path).and_then(|file| built_id(&file)).is_none())
		.collect()
}

/// Collect resource names by assembling against a resolver accepting every
/// name.
#[derive(Default)]
struct Named(BTreeSet<String>);

impl Named {
	fn value(&mut self, value: &ConstValue) {
		if let ConstValue::Resource(path) = value {
			self.0.insert(path.clone());
		}
	}

	fn code(&mut self, nodes: &[dmasm::Node], params: &[Param], statics: &[StaticVar]) {
		let _ = dmasm::assembler::assemble(nodes, self);
		for param in params {
			if let InList::New(list) = &param.in_list {
				self.code(&list.nodes, &list.params, &list.statics);
			}
		}
		for local in statics {
			if let Some(value) = &local.value {
				self.value(value);
			}
			if let Some(init) = &local.init {
				self.code(&init.nodes, &init.params, &init.statics);
			}
		}
	}
}

impl dmasm::assembler::AssembleEnv for Named {
	fn get_string_index(&mut self, _: &[u8]) -> Option<u32> {
		Some(0)
	}

	fn get_variable_name_index(&mut self, _: &[u8]) -> Option<u32> {
		Some(0)
	}

	fn get_proc_index(&mut self, _: &str) -> Option<u32> {
		Some(0)
	}

	fn get_type(&mut self, _: &str) -> Option<(u8, u32)> {
		Some((0x20, 0))
	}

	fn get_global_index(&mut self, _: &[u8], _: Option<&str>) -> Option<u32> {
		Some(0)
	}

	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		self.0.insert(path.to_owned());
		Some(0)
	}

	fn get_instance(&mut self, _: &str, _: &[u32]) -> Option<u32> {
		Some(0)
	}
}
