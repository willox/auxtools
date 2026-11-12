//! Display and lookup names for procs without paths: `(global init)`,
//! `(in list)#N`, or `(var initializer)#N` when unclassified or tables are
//! unavailable. DreamMaker emits these for global/static initializers,
//! parameter `in` lists and type var initializers. BYOND later names the type
//! initializers `/TYPE/proc/(init)`, which take precedence when available.

use auxtools::{raw_types::procs::ProcId, runtime_tables, Proc};

use crate::{
	hot_reload,
	server_types::{ProcRef, Reloaded, GLOBAL_INIT}
};

const IN_LIST: &str = "(in list)";
/// Fallback name, indexed among auxtools' unnamed procs.
const OTHER: &str = "(var initializer)";

/// Construct a displayable, resolvable proc reference.
pub fn proc_ref(proc: &Proc) -> ProcRef {
	if !proc.path.is_empty() {
		return ProcRef {
			path: proc.path.clone(),
			override_id: proc.override_id()
		};
	}
	// Prefer an init proc path assigned by BYOND after the initial scan.
	if let Some(now) = Proc::from_id(proc.id).filter(|now| !now.path.is_empty()) {
		return ProcRef {
			path: now.path,
			override_id: 0
		};
	}
	Kinds::read().ok().and_then(|kinds| kinds.name(proc)).unwrap_or_else(|| ProcRef {
		path: OTHER.to_owned(),
		override_id: proc.override_id()
	})
}

/// Format a proc name, adding `#N` for definitions after the first.
pub fn proc_name(proc: &Proc) -> String {
	let proc = proc_ref(proc);
	match proc.override_id {
		0 => proc.path,
		n => format!("{}#{}", proc.path, n)
	}
}

/// Resolve a proc by path or one of the synthetic names in this module.
pub fn find(path: &str, override_id: u32) -> Option<Proc> {
	if !path.starts_with('(') {
		return Proc::find_override(path, override_id);
	}
	if path == OTHER {
		return Proc::find_override("", override_id);
	}
	let kinds = Kinds::read().ok()?;
	let id = match (path, override_id) {
		(GLOBAL_INIT, 0) => kinds.global_init?,
		(IN_LIST, n) => *kinds.in_lists.get(n as usize)?,
		_ => return None
	};
	Proc::from_id(ProcId(id))
}

/// Classify unnamed procs using the runtime tables.
struct Kinds {
	in_lists: Vec<u32>,
	global_init: Option<u32>
}

impl Kinds {
	fn read() -> Result<Self, String> {
		let in_lists = runtime_tables::in_list_procs()?;
		// The initial scan predates BYOND naming type init procs. DreamMaker
		// emits global init after those and the `in` lists; later unnamed procs
		// were added by hot reload.
		let global_init = Proc::overrides("")
			.iter()
			.map(|proc| proc.id)
			.filter(|&id| {
				Proc::from_id(id).is_some_and(|now| now.path.is_empty())
					&& !in_lists.contains(&id.0)
					&& hot_reload::reloaded(id.0) != Some(Reloaded::Added)
			})
			.map(|id| id.0)
			.max();
		Ok(Self { in_lists, global_init })
	}

	fn name(&self, proc: &Proc) -> Option<ProcRef> {
		let id = proc.id.0;
		let (path, override_id) = if self.global_init == Some(id) {
			(GLOBAL_INIT, 0)
		} else {
			(IN_LIST, self.in_lists.iter().position(|&listed| listed == id)? as u32)
		};
		Some(ProcRef {
			path: path.to_owned(),
			override_id
		})
	}
}
