//! Track the last `.dmf` included by the `.dme` and reload its text as a whole.

use std::{
	collections::HashSet,
	path::{Path, PathBuf}
};

use auxtools_cli::server_types::SkinRequest;

/// A skin's file and text
#[derive(Clone, PartialEq, Eq)]
struct Text {
	path: PathBuf,
	text: Vec<u8>
}

impl Text {
	fn read(root: &Path, path: Option<&Path>) -> Option<Text> {
		let path = path?;
		Some(Text {
			text: std::fs::read(root.join(path)).ok()?,
			path: path.to_owned()
		})
	}
}

/// Built, applied and source skins.
pub struct Skin {
	built: Option<Text>,
	world: Option<Text>,
	/// Parsed skin path, relative to the `.dme` directory.
	source: Option<PathBuf>
}

/// Skin update.
pub struct Change {
	pub request: SkinRequest,
	/// Relative to the `.dme` directory.
	pub file: PathBuf,
	/// New image references absent from the built skin and its resources.
	pub images: Vec<String>,
	text: Text
}

impl Skin {
	/// Record the skin used to build the world.
	pub fn built(root: &Path, path: Option<&Path>) -> Skin {
		let built = Text::read(root, path);
		Skin {
			world: built.clone(),
			built,
			source: path.map(Path::to_owned)
		}
	}

	/// Remove the skin from the changed-file set; return whether it was
	/// present.
	pub fn take_edit(&self, changed: &mut HashSet<PathBuf>) -> bool {
		self.source.as_ref().is_some_and(|path| changed.remove(path))
	}

	/// Update the parsed skin path.
	pub fn parsed(&mut self, path: Option<&Path>) {
		self.source = path.map(Path::to_owned);
	}

	/// Reset to the build's skin after reboot.
	pub fn rebooted(&mut self) {
		self.world = self.built.clone();
	}

	/// Pending skin update, if it differs from the world's.
	pub fn change(&self, root: &Path) -> Option<Change> {
		let text = Text::read(root, self.source.as_deref())?;
		if self.world.as_ref() == Some(&text) {
			return None;
		}
		let file = text.path.clone();
		let absolute = std::path::absolute(root.join(&file)).unwrap_or_else(|_| root.join(&file));
		let built: HashSet<String> = self
			.built
			.as_ref()
			.map(|built| images(&built.text).into_iter().collect())
			.unwrap_or_default();
		let mut new: Vec<String> = images(&text.text).into_iter().filter(|name| !built.contains(name)).collect();
		new.sort();
		new.dedup();
		Some(Change {
			request: SkinRequest {
				path: absolute.display().to_string()
			},
			file,
			images: new,
			text
		})
	}

	/// Record the applied skin.
	pub fn applied(&mut self, change: Change) {
		self.world = Some(change.text);
	}
}

/// Resource-file parameters in a skin (`param = 'file'`).
fn images(text: &[u8]) -> Vec<String> {
	String::from_utf8_lossy(text)
		.lines()
		.filter_map(|line| {
			let (_, value) = line.split_once('=')?;
			let name = value.trim().strip_prefix('\'')?.strip_suffix('\'')?;
			(!name.is_empty() && !name.contains('\'')).then(|| name.to_owned())
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn images_are_the_single_quoted_values() {
		let skin = b"window \"w\"\n\telem \"b\"\n\t\timage = 'icons\\coin.png'\n\t\tcommand = \"say 'hi'\"\n\t\ttext = 'x.png'\n";
		assert_eq!(images(skin), ["icons\\coin.png", "x.png"]);
	}

	#[test]
	fn a_changed_skin_names_its_new_images() {
		let dir = std::env::temp_dir().join(format!("auxtools-skin-{}", std::process::id()));
		std::fs::create_dir_all(dir.join("ui")).unwrap();
		std::fs::write(dir.join("ui/skin.dmf"), "\t\timage = 'old.png'\n").unwrap();
		let mut skin = Skin::built(&dir, Some(Path::new("ui/skin.dmf")));
		assert!(skin.change(&dir).is_none());

		std::fs::write(dir.join("ui/skin.dmf"), "\t\timage = 'old.png'\n\t\timage = 'new.png'\n").unwrap();
		let change = skin.change(&dir).unwrap();
		assert_eq!(change.file, Path::new("ui/skin.dmf"));
		assert_eq!(change.images, ["new.png"]);
		assert!(change.request.path.ends_with("skin.dmf"));
		skin.applied(change);
		assert!(skin.change(&dir).is_none());

		skin.rebooted();
		assert!(skin.change(&dir).is_some());
		let _ = std::fs::remove_dir_all(&dir);
	}
}
