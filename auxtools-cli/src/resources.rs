//! Resource files watched for hot reload.
//!
//! The `.dmb` resource table maps IDs to CRCs; the `.rsc` maps CRCs to the
//! names used in source. Resolve names in the `.dme` directory, then in
//! `FILE_DIR` order. Multiple spellings can give a file several IDs, while
//! files with identical content share an ID.

use std::{
	collections::{BTreeMap, HashMap},
	path::{Path, PathBuf}
};

use auxtools_cli::dmb_tables::{self, byond_crc};

/// A resource file, by its path relative to the `.dme`'s directory
#[derive(Debug, Clone, Default)]
pub struct File {
	/// Source spellings, used to match resource literals.
	pub names: Vec<String>,
	/// Resource IDs used for this file, starting with its built ID.
	pub ids: Vec<u32>,
	/// Current resource IDs. Values retained from earlier reloads still use
	/// the old IDs.
	pub current: Vec<u32>,
	/// CRC of the file currently shown by the world.
	pub crc: u32,
	/// CRC of the file as built.
	pub built: u32
}

#[derive(Debug, Default)]
pub struct Resources {
	pub files: BTreeMap<PathBuf, File>,
	/// `FILE_DIR`, in order, relative to the `.dme`'s directory
	pub dirs: Vec<String>
}

impl Resources {
	/// Read resources for `dmb`, built from `sources` under `root`.
	pub fn read(root: &Path, dmb: &Path, sources: &[PathBuf]) -> Result<Resources, String> {
		let dirs = file_dirs(root, sources);
		let rsc = dmb.with_extension("rsc");
		let dmb_data = std::fs::read(dmb).map_err(|e| format!("couldn't read {}: {}", dmb.display(), e))?;
		let rsc_data = match std::fs::read(&rsc) {
			Ok(data) => data,
			// A world without resources has no .rsc
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Resources { dirs, ..Default::default() }),
			Err(e) => return Err(format!("couldn't read {}: {}", rsc.display(), e))
		};
		let names = rsc_names(&rsc_data);
		let crcs = dmb_tables::resource_crcs(&dmb_data).map_err(|e| format!("couldn't read {}'s resources: {}", dmb.display(), e))?;
		let mut files: BTreeMap<PathBuf, File> = BTreeMap::new();
		for (crc, spellings) in &names {
			let ids: Vec<u32> = crcs.iter().enumerate().filter(|(_, c)| *c == crc).map(|(id, _)| id as u32).collect();
			if ids.is_empty() {
				continue;
			}
			for name in spellings {
				// Skins are resources too, but use their own whole-file reload.
				if name.to_ascii_lowercase().ends_with(".dmf") {
					continue;
				}
				let Some(path) = resolve(root, &dirs, name) else {
					continue;
				};
				let file = files.entry(path).or_insert_with(|| File {
					crc: *crc,
					built: *crc,
					..Default::default()
				});
				if !file.names.contains(name) {
					file.names.push(name.clone());
				}
				for id in &ids {
					if !file.ids.contains(id) {
						file.ids.push(*id);
						file.current.push(*id);
					}
				}
			}
		}
		Ok(Resources { files, dirs })
	}

	/// Watched files with content differing from the world's, and their new
	/// CRCs.
	pub fn changed<'a>(&self, root: &Path, files: impl IntoIterator<Item = &'a PathBuf>) -> Vec<(PathBuf, u32)> {
		files
			.into_iter()
			.filter_map(|path| {
				let file = self.files.get(path)?;
				let crc = byond_crc(&std::fs::read(root.join(path)).ok()?);
				(crc != file.crc).then(|| (path.clone(), crc))
			})
			.collect()
	}

	/// The files that differ from what the world was built with
	pub fn edited_since_built(&self, root: &Path) -> Vec<(PathBuf, u32)> {
		self.files
			.iter()
			.filter_map(|(path, file)| {
				let crc = byond_crc(&std::fs::read(root.join(path)).ok()?);
				(crc != file.built).then(|| (path.clone(), crc))
			})
			.collect()
	}

	/// Record `path` cached with content `crc` and resource `id`.
	pub fn reloaded(&mut self, path: &Path, id: u32, crc: u32) {
		if let Some(file) = self.files.get_mut(path) {
			if !file.ids.contains(&id) {
				file.ids.push(id);
			}
			file.current = vec![id];
			file.crc = crc;
		}
	}

	/// Current IDs to replace when reloading `path`, excluding IDs shared
	/// with another file now or previously. Also return the sharing files.
	pub fn replaced(&self, path: &Path) -> (Vec<u32>, Vec<PathBuf>) {
		let Some(file) = self.files.get(path) else {
			return (vec![], vec![]);
		};
		let mut sharing = vec![];
		let mut ids = vec![];
		for &id in &file.current {
			let others: Vec<&PathBuf> = self
				.files
				.iter()
				.filter(|(other, f)| *other != path && f.ids.contains(&id))
				.map(|(other, _)| other)
				.collect();
			if others.is_empty() {
				ids.push(id);
			}
			for other in others {
				if !sharing.contains(other) {
					sharing.push(other.clone());
				}
			}
		}
		(ids, sharing)
	}

	/// Record `path`'s current content when the world tracks its resource ID.
	pub fn shown(&mut self, path: &Path, crc: u32) {
		if let Some(file) = self.files.get_mut(path) {
			file.crc = crc;
		}
	}

	/// A new world from the same build: it shows what it was built with
	pub fn restarted(&mut self, built: &Resources) {
		for (path, file) in &mut self.files {
			if let Some(original) = built.files.get(path) {
				file.ids = original.ids.clone();
				file.current = original.ids.clone();
			}
			file.crc = file.built;
		}
	}
}

/// The `FILE_DIR` directories the sources define, in order
fn file_dirs(root: &Path, sources: &[PathBuf]) -> Vec<String> {
	let mut dirs = vec![];
	for source in sources {
		let Ok(text) = std::fs::read_to_string(root.join(source)) else {
			continue;
		};
		for line in text.lines() {
			let line = line.trim_start();
			let Some(rest) = line.strip_prefix('#').map(str::trim_start).and_then(|l| l.strip_prefix("define")) else {
				continue;
			};
			let mut words = rest.split_whitespace();
			if words.next() != Some("FILE_DIR") {
				continue;
			}
			let dir = words.collect::<Vec<_>>().join(" ").trim_matches('"').replace('\\', "/");
			if !dir.is_empty() && !dirs.contains(&dir) {
				dirs.push(dir);
			}
		}
	}
	dirs
}

/// Where DreamMaker finds `name`: under `root`, then under each of `dirs`
fn resolve(root: &Path, dirs: &[String], name: &str) -> Option<PathBuf> {
	let name = name.replace('\\', "/");
	std::iter::once(".")
		.chain(dirs.iter().map(String::as_str))
		.map(|dir| Path::new(dir).join(&name))
		.find_map(|candidate| existing(root, &candidate))
}

/// Find `path` under `root` with DreamMaker's case-insensitive lookup on Linux.
fn existing(root: &Path, path: &Path) -> Option<PathBuf> {
	use std::path::Component;
	let mut found = PathBuf::new();
	for part in path.components() {
		let Component::Normal(part) = part else {
			continue;
		};
		let exact = found.join(part);
		if root.join(&exact).exists() {
			found = exact;
			continue;
		}
		let part = part.to_str()?;
		let mut names: Vec<String> = std::fs::read_dir(root.join(&found))
			.ok()?
			.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
			.filter(|name| name.eq_ignore_ascii_case(part))
			.collect();
		names.sort();
		found.push(names.into_iter().next()?);
	}
	root.join(&found).is_file().then_some(found)
}

/// Read file names by CRC from `.rsc` records:
/// `u32 length, u8 in use, u8 type, u32 crc, u32 time, u32 time, u32 data
/// length, name, data`.
fn rsc_names(data: &[u8]) -> HashMap<u32, Vec<String>> {
	let mut names: HashMap<u32, Vec<String>> = HashMap::new();
	let mut at = 0;
	while at + 5 <= data.len() {
		let length = u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
		let record = &data[at + 4..(at + 5 + length).min(data.len())];
		at += 5 + length;
		if record.len() < 19 || record[0] == 0 {
			continue;
		}
		let crc = u32::from_le_bytes(record[2..6].try_into().unwrap());
		if let Some(end) = record[18..].iter().position(|&b| b == 0) {
			let name = String::from_utf8_lossy(&record[18..18 + end]).into_owned();
			if !name.is_empty() {
				names.entry(crc).or_default().push(name);
			}
		}
	}
	names
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rsc_records_in_use_name_their_crcs() {
		let record = |used: u8, crc: u32, name: &str| {
			let mut r = vec![used, 3];
			r.extend(crc.to_le_bytes());
			r.extend([0; 12]);
			r.extend(name.as_bytes());
			r.push(0);
			r.extend([1, 2, 3]);
			let mut out = ((r.len() - 1) as u32).to_le_bytes().to_vec();
			out.extend(r);
			out
		};
		let mut rsc = record(1, 7, "a.dmi");
		rsc.extend(record(0, 8, "old.dmi"));
		rsc.extend(record(1, 7, "icons/a.dmi"));
		let names = rsc_names(&rsc);
		assert_eq!(names.get(&7), Some(&vec!["a.dmi".to_owned(), "icons/a.dmi".to_owned()]));
		assert_eq!(names.get(&8), None);
	}

	#[test]
	fn a_resource_another_file_shares_isnt_replaced() {
		let file = |ids: &[u32]| File {
			ids: ids.to_vec(),
			current: ids.to_vec(),
			..Default::default()
		};
		let mut resources = Resources::default();
		// Identical built content gives a.dmi and b.dmi the same resource ID.
		resources.files.insert("a.dmi".into(), file(&[3]));
		resources.files.insert("b.dmi".into(), file(&[3]));
		resources.files.insert("c.dmi".into(), file(&[4]));
		assert_eq!(resources.replaced(Path::new("a.dmi")), (vec![], vec![PathBuf::from("b.dmi")]));
		assert_eq!(resources.replaced(Path::new("c.dmi")), (vec![4], vec![]));

		// After a save, replace only the current ID; held values retain ID 4.
		resources.reloaded(Path::new("c.dmi"), 9, 0xC);
		assert_eq!(resources.replaced(Path::new("c.dmi")), (vec![9], vec![]));
		// Saving c.dmi with a.dmi's content gives it the shared ID 3.
		// A later save must not replace that ID for the other files.
		resources.reloaded(Path::new("c.dmi"), 3, 0xA);
		assert_eq!(
			resources.replaced(Path::new("c.dmi")),
			(vec![], vec![PathBuf::from("a.dmi"), PathBuf::from("b.dmi")])
		);
	}
}
