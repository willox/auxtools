//! Track built, applied and source `client/script` values. Hot reload
//! replaces script text as a whole; resource-file scripts require a rebuild.

use auxtools_cli::server_types::ScriptRequest;
use dmasm::reload::ClientScript;

/// Parsed `client/script` value or its error.
type Source = Result<Option<ClientScript>, String>;

/// Built, applied and source client scripts.
pub struct Script {
	built: Source,
	/// Last applied text (`None` for no script), once a reload has applied.
	world: Option<Option<String>>,
	source: Source,
	/// Last reported script error.
	reported: Option<String>
}

/// Client script update.
pub struct Change {
	pub request: ScriptRequest,
	text: Option<String>
}

/// Script text, `None` for no script, or an error for an unsupported change.
fn text(script: &Source) -> Result<Option<&str>, String> {
	match script {
		Ok(None) => Ok(None),
		Ok(Some(ClientScript::Text(text))) => Ok(Some(text)),
		Ok(Some(ClientScript::File(name))) => Err(format!("client/script references '{}'; resource-file scripts require a rebuild", name)),
		Err(why) => Err(why.clone())
	}
}

impl Script {
	pub fn built(script: Source) -> Script {
		Script {
			source: script.clone(),
			built: script,
			world: None,
			reported: None
		}
	}

	/// Update the parsed source value.
	pub fn parsed(&mut self, script: Source) {
		self.source = script;
	}

	/// Reset to the build's script after reboot.
	pub fn rebooted(&mut self) {
		self.world = None;
	}

	/// Pending script update, or the reason it cannot be applied.
	fn difference(&self) -> Result<Option<Change>, String> {
		if self.world.is_none() && self.source == self.built {
			return Ok(None);
		}
		let built = text(&self.built)?;
		let source = text(&self.source)?;
		let world = match &self.world {
			Some(world) => world.as_deref(),
			None => built
		};
		Ok((source != world).then(|| Change {
			request: ScriptRequest {
				text: source.map(str::to_owned),
				built: built.map(str::to_owned)
			},
			text: source.map(str::to_owned)
		}))
	}

	/// Pending script update, if supported.
	pub fn change(&self) -> Option<Change> {
		self.difference().ok().flatten()
	}

	/// Record the applied script.
	pub fn applied(&mut self, change: Change) {
		self.world = Some(change.text);
	}

	/// Script error, unless already reported.
	pub fn problem(&mut self) -> Option<String> {
		let problem = self.difference().err();
		if problem == self.reported {
			return None;
		}
		self.reported.clone_from(&problem);
		problem
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_changed_script_text_is_given_as_a_whole() {
		let text = |t: &str| Ok(Some(ClientScript::Text(t.to_owned())));
		let mut script = Script::built(text("<style>x {}</style>"));
		assert!(script.change().is_none());

		script.parsed(text("<style>y {}</style>"));
		let change = script.change().unwrap();
		assert_eq!(change.request, ScriptRequest {
			text: Some("<style>y {}</style>".to_owned()),
			built: Some("<style>x {}</style>".to_owned())
		});
		script.applied(change);
		assert!(script.change().is_none());

		// Removing a script is a supported update.
		script.parsed(Ok(None));
		let change = script.change().unwrap();
		assert_eq!(change.request.text, None);
		script.applied(change);
		assert!(script.change().is_none() && script.problem().is_none());

		// Report unsupported changes once and retain the applied script.
		script.parsed(Err("not a constant".to_owned()));
		assert!(script.change().is_none());
		assert_eq!(script.problem().as_deref(), Some("not a constant"));
		assert_eq!(script.problem(), None);
		script.parsed(Ok(Some(ClientScript::File("c.css".to_owned()))));
		assert!(script.change().is_none());
		assert!(script.problem().is_some_and(|why| why.contains("'c.css'")));

		// Reboot restores the build's script.
		script.parsed(text("<style>y {}</style>"));
		script.rebooted();
		assert!(script.change().is_some());
		script.parsed(text("<style>x {}</style>"));
		assert!(script.change().is_none() && script.problem().is_none());
	}

	#[test]
	fn a_script_built_from_a_file_needs_a_rebuild_to_change() {
		let file = || Ok(Some(ClientScript::File("a.css".to_owned())));
		let mut script = Script::built(file());
		assert!(script.change().is_none() && script.problem().is_none());
		script.parsed(Ok(Some(ClientScript::Text("<style>y {}</style>".to_owned()))));
		assert!(script.change().is_none());
		assert!(script.problem().is_some_and(|why| why.contains("'a.css'")));
		script.parsed(file());
		assert!(script.change().is_none() && script.problem().is_none());
	}
}
