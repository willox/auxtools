//! Client script reloads and reconnect notifications.

use super::*;

/// Notify editors that connected clients need to reconnect for the new script.
#[derive(serde::Serialize)]
struct ScriptEvent {
	clients: u32,
	hosted: bool
}

impl Event for ScriptEvent {
	const EVENT: &'static str = "auxtoolsScript";
}

impl Adapter {
	/// Apply `request`'s client script, including after a reboot.
	/// Return whether the world accepted it.
	pub(super) fn reload_script(&mut self, request: aux::ScriptRequest, restored: bool) -> bool {
		let result = match self.request(&AuxRequest::Script(request)) {
			Ok(AuxResponse::Script(result)) => result,
			Ok(other) => Err(unexpected(other)),
			Err(e) => Err(e)
		};
		let reloaded = match result {
			Ok(reloaded) => reloaded,
			Err(e) => {
				self.out.console(format!("Hot reload failed: client script: {}", e));
				self.journal_reload(false, "client script not reloaded".to_owned(), vec![e], vec![]);
				return false;
			}
		};
		if restored {
			self.out.console("Hot reload: client script restored after reboot");
			self.journal_reload(true, "client script restored".to_owned(), vec![], vec![]);
			return true;
		}
		let mut notes = vec![];
		if reloaded.clients > 0 {
			notes.push(if reloaded.hosted {
				"Reboot to load the new script in Dream Seeker's host window; auxtools reconnect does this".to_owned()
			} else {
				format!(
					"{} the old script until {}; run auxtools reconnect to load the new script",
					if reloaded.clients == 1 {
						"1 connected client keeps".to_owned()
					} else {
						format!("{} connected clients keep", reloaded.clients)
					},
					if reloaded.clients == 1 { "it reconnects" } else { "they reconnect" }
				)
			});
		}
		self.out.console("Hot reload: client script updated for new logins");
		self.notes(&notes);
		self.journal_reload(true, "client script".to_owned(), vec![], notes);
		self.out.event(ScriptEvent {
			clients: reloaded.clients,
			hosted: reloaded.hosted
		});
		true
	}
}
