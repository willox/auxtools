//! Reload resource files such as icons and sounds.
//!
//! Cache each file under a new ID with fcopy_rsc(); BYOND announces it to
//! clients. The rest of the reload recompiles code and defaults referencing
//! the file. Update and resend appearances that used the old ID.
//!
//! Stored values retain old resources: vars, lists, /icon and /sound datums,
//! and mutable appearances.

use auxtools::appearance_tables;
use dmasm::assembler::AssembleEnv as _;

use crate::{
	assemble_env::{AssembleEnv, RESOURCE_DIRS},
	server_types::{HotReloadResource, ReloadedResource}
};

/// Prepared resource file: new resource ID and the IDs it replaces.
pub struct Prepared {
	path: String,
	id: u32,
	replaces: Vec<u32>
}

/// Set the `FILE_DIR` directories for this reload's resource lookups. They
/// come with every reload, so each one sees the code's current list.
pub fn use_dirs(dirs: &[String]) {
	if let Ok(mut current) = RESOURCE_DIRS.lock() {
		*current = dirs.to_vec();
	}
}

/// Cache edited files and validate resource reload support before
/// applying changes. Runs inside the reload evaluation.
pub fn prepare(resources: &[HotReloadResource]) -> Result<Vec<Prepared>, Vec<String>> {
	if resources.is_empty() {
		return Ok(vec![]);
	}
	appearance_tables::check().map_err(|e| vec![e])?;
	let mut prepared = vec![];
	let mut errors = vec![];
	for resource in resources {
		match AssembleEnv.get_resource_index(&resource.path) {
			Some(id) => prepared.push(Prepared {
				path: resource.path.clone(),
				id,
				replaces: resource.ids.iter().copied().filter(|&old| old != id).collect()
			}),
			None => errors.push(format!("resource reload: couldn't read {}", resource.path))
		}
	}
	if errors.is_empty() {
		Ok(prepared)
	} else {
		Err(errors)
	}
}

/// Applied resource changes.
pub struct Applied {
	pub reloaded: Vec<ReloadedResource>,
	/// The appearances that changed, to send again
	pub appearances: Vec<u32>,
	pub warnings: Vec<String>
}

/// Update appearances for all files together, before the remaining reload.
/// An error here leaves appearances unchanged.
pub fn apply(prepared: &[Prepared]) -> Result<Applied, String> {
	if prepared.is_empty() {
		return Ok(Applied {
			reloaded: vec![],
			appearances: vec![],
			warnings: vec![]
		});
	}
	let mappings: Vec<(u32, u32)> = prepared
		.iter()
		.flat_map(|resource| resource.replaces.iter().map(|&old| (old, resource.id)))
		.collect();
	let repointed = appearance_tables::repoint(&mappings)?;
	let reloaded = prepared
		.iter()
		.map(|resource| ReloadedResource {
			path: resource.path.clone(),
			id: resource.id,
			appearances: repointed.counts.get(&resource.id).copied().unwrap_or(0)
		})
		.collect();
	let mut appearances = repointed.appearances;
	appearances.sort_unstable();
	appearances.dedup();
	Ok(Applied {
		reloaded,
		appearances,
		warnings: repointed.warnings.into_iter().map(|w| format!("resource reload: {}", w)).collect()
	})
}

/// Resend changed appearances once the whole reload is applied.
pub fn resend(appearances: &[u32]) -> Result<usize, String> {
	appearance_tables::resend(appearances)
}
