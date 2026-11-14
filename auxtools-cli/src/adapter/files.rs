//! Map DreamMaker debug source names to local files.
//!
//! Debug names are not always project-relative paths. For example, a nested
//! `.dme` can include `_std/tgs.dm` as `tgs.dm`. Read the include hierarchy
//! to resolve these names; see `dmasm::debug_files`.

use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	time::{Duration, Instant}
};

use dmasm::debug_files;

/// Refresh interval for unresolved names; new files may have been added.
const STALE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct WorldFiles {
	/// The world's `.dme`
	dme: Option<PathBuf>,
	/// Nonstandard debug names and local paths, mapped in both directions,
	/// with the time they were read.
	names: Option<(HashMap<String, PathBuf>, HashMap<PathBuf, String>, Instant)>
}

impl WorldFiles {
	pub fn new(dme: Option<PathBuf>) -> Self {
		WorldFiles { dme, names: None }
	}

	/// Project `.dme`, if known.
	pub fn dme(&self) -> Option<&Path> {
		self.dme.as_deref()
	}

	/// The `.dme`'s directory
	fn root(&self) -> Option<&Path> {
		self.dme.as_deref()?.parent()
	}

	fn read(&mut self) {
		let Some(dme) = &self.dme else { return };
		let mut by_name = HashMap::new();
		let mut by_path = HashMap::new();
		for (name, path) in debug_files::names(dme, &|_| true) {
			if Path::new(&name) != path {
				// Debug info cannot distinguish duplicate names. Use the first
				// match.
				by_name.entry(name.clone()).or_insert_with(|| path.clone());
				by_path.entry(path).or_insert(name);
			}
		}
		self.names = Some((by_name, by_path, Instant::now()));
	}

	/// The file the world calls `name`
	pub fn path(&mut self, name: &str) -> PathBuf {
		let name = name.replace('\\', "/");
		let Some(root) = self.root().map(Path::to_owned) else {
			return PathBuf::from(name);
		};
		// The .dme itself has its whole path, as it was where it was built
		if is_absolute(&name) {
			let built = Path::new(&name);
			return match &self.dme {
				Some(dme) if !built.is_file() && name.rsplit('/').next() == dme.file_name().and_then(|file| file.to_str()) => dme.clone(),
				_ => built.to_owned()
			};
		}
		if self.names.is_none() {
			self.read();
		}
		if let Some(path) = self.names.as_ref().and_then(|(by_name, ..)| by_name.get(&name)) {
			return root.join(path);
		}
		let path = root.join(debug_files::normalize(Path::new(&name)));
		// A file added since the code was read?
		if !path.is_file() && self.names.as_ref().is_some_and(|(.., read)| read.elapsed() >= STALE) {
			self.read();
			if let Some(found) = self.names.as_ref().and_then(|(by_name, ..)| by_name.get(&name)) {
				return root.join(found);
			}
		}
		path
	}

	/// Resolve a local path to its debug name. Paths outside the project
	/// remain unchanged and are matched by suffix on the server.
	pub fn name(&mut self, path: &str) -> String {
		let Some(relative) = self.root().and_then(|root| Path::new(path).strip_prefix(root).ok()).map(Path::to_owned) else {
			return path.to_owned();
		};
		if self.names.is_none() {
			self.read();
		}
		match self
			.names
			.as_ref()
			.and_then(|(_, by_path, _)| by_path.get(&debug_files::normalize(&relative)))
		{
			Some(name) => name.clone(),
			None => relative.to_string_lossy().replace('\\', "/")
		}
	}
}

/// Whether the path is absolute on Linux or Windows.
fn is_absolute(name: &str) -> bool {
	let bytes = name.as_bytes();
	name.starts_with('/') || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_and_paths() {
		let dir = std::env::temp_dir().join(format!("auxtools-world-files-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		let write = |path: &str, text: &str| {
			let path = dir.join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, text).unwrap();
		};
		write("world.dme", "#include \"code\\map.dm\"\n#include \"_std\\__std.dme\"\n");
		write("code/map.dm", "");
		write("_std/__std.dme", "#include \"map.dm\"\n#include \"macros\\dir.dm\"\n");
		write("_std/map.dm", "");
		write("_std/macros/dir.dm", "");
		let mut files = WorldFiles::new(Some(dir.join("world.dme")));

		// A file is where its name says, but for the ones named by another .dme
		assert_eq!(files.path("code/map.dm"), dir.join("code/map.dm"));
		assert_eq!(files.path("map.dm"), dir.join("_std/map.dm"));
		assert_eq!(files.path("macros/dir.dm"), dir.join("_std/macros/dir.dm"));
		assert_eq!(files.path("_std/__std.dme"), dir.join("_std/__std.dme"));
		// One the code doesn't have, and one with its path spelled out
		assert_eq!(files.path("code\\gone.dm"), dir.join("code/gone.dm"));
		assert_eq!(files.path("code/sub/../map.dm"), dir.join("code/map.dm"));
		// The .dme, built somewhere else
		assert_eq!(files.path("/build/somewhere/world.dme"), dir.join("world.dme"));
		assert_eq!(files.path("C:\\build\\world.dme"), dir.join("world.dme"));
		assert_eq!(files.path("/build/somewhere/other.dme"), Path::new("/build/somewhere/other.dme"));

		let name = |files: &mut WorldFiles, path: &str| files.name(&dir.join(path).to_string_lossy());
		assert_eq!(name(&mut files, "code/map.dm"), "code/map.dm");
		assert_eq!(name(&mut files, "_std/map.dm"), "map.dm");
		assert_eq!(name(&mut files, "_std/macros/dir.dm"), "macros/dir.dm");
		assert_eq!(name(&mut files, "_std/__std.dme"), "_std/__std.dme");
		assert_eq!(files.name("/elsewhere/code/map.dm"), "/elsewhere/code/map.dm");

		// With no .dme a name is all there is
		let mut none = WorldFiles::default();
		assert_eq!(none.path("code\\map.dm"), Path::new("code/map.dm"));
		assert_eq!(none.name("/elsewhere/code/map.dm"), "/elsewhere/code/map.dm");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
