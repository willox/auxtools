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
			self.journal_reload(true, "client script restored".to_owned(), vec![], vec![]);
			return true;
		}
		let waiting = self.waiting_clients(reloaded.clients, reloaded.hosted, "script");
		self.out.console(format!(
			"Hot reload: client script updated.{}",
			waiting.iter().map(|w| format!(" {}", w)).collect::<String>()
		));
		self.journal_reload(true, "client script".to_owned(), vec![], waiting.into_iter().collect());
		self.out.event(ScriptEvent {
			clients: reloaded.clients,
			hosted: reloaded.hosted
		});
		true
	}
}
