//! Skin reloads and client reconnects.

use super::*;

/// Notify editors that connected clients need to reconnect for the new skin.
#[derive(serde::Serialize)]
struct SkinEvent {
	file: String,
	clients: u32,
	hosted: bool
}

impl Event for SkinEvent {
	const EVENT: &'static str = "auxtoolsSkin";
}

impl Adapter {
	/// Apply the skin at `file`, including after a reboot, and report its
	/// effect on connected clients. Return whether the world accepted it.
	pub(super) fn reload_skin(&mut self, request: aux::SkinRequest, file: &Path, images: &[String], restored: bool) -> bool {
		let file = file.display().to_string();
		let result = match self.request(&AuxRequest::Skin(request)) {
			Ok(AuxResponse::Skin(result)) => result,
			Ok(other) => Err(unexpected(other)),
			Err(e) => Err(e)
		};
		let reloaded = match result {
			Ok(reloaded) => reloaded,
			Err(e) => {
				self.out.console(format!("Hot reload failed: skin {}: {}", file, e));
				self.journal_reload(false, format!("skin {} not reloaded", file), vec![e], vec![]);
				return false;
			}
		};

		if restored {
			self.out.console(format!("Hot reload: skin {} restored after reboot", file));
			self.journal_reload(true, format!("skin {} restored", file), vec![], vec![]);
			return true;
		}
		let mut notes = vec![];
		if reloaded.clients > 0 {
			notes.push(if reloaded.hosted {
				"Reboot to load the new skin in Dream Seeker's host window; auxtools reconnect does this".to_owned()
			} else {
				format!(
					"{} the old interface until {}; run auxtools reconnect to load the new skin",
					if reloaded.clients == 1 {
						"1 connected client keeps".to_owned()
					} else {
						format!("{} connected clients keep", reloaded.clients)
					},
					if reloaded.clients == 1 { "it reconnects" } else { "they reconnect" }
				)
			});
		}
		if !images.is_empty() {
			notes.push(format!(
				"{}: new skin resources; rebuild to display {}",
				images.iter().map(|name| format!("'{}'", name)).collect::<Vec<_>>().join(", "),
				if images.len() == 1 { "it" } else { "them" }
			));
		}

		self.out.console(format!("Hot reload: skin {} updated for new logins", file));
		self.notes(&notes);
		self.journal_reload(true, format!("skin {}", file), vec![], notes);
		self.out.event(SkinEvent {
			file,
			clients: reloaded.clients,
			hosted: reloaded.hosted
		});
		true
	}

	/// Reconnect clients to load the skin. Reboot Dream Seeker-hosted worlds.
	pub(super) fn reconnect_clients(&mut self) -> Handled {
		match self.request(&AuxRequest::Reconnect)? {
			AuxResponse::Reconnect(Ok(aux::Reconnected::Clients(clients))) => Ok(serde_json::json!({ "reconnected": clients })),
			AuxResponse::Reconnect(Ok(aux::Reconnected::Hosted)) => {
				self.reboot()?;
				Ok(serde_json::json!({ "rebooted": true }))
			}
			AuxResponse::Reconnect(Err(e)) => Err(e),
			other => Err(unexpected(other))
		}
	}
}
