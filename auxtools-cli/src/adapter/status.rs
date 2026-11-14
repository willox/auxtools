//! Client reload state and refused changes.

use dmasm::reload::Refusal;

use super::*;

/// Reload state: `reading`, `active`, `reloading`, `reloaded`, `refused`,
/// `failed` or `off`. `summary` describes a reload; `problems` explain
/// refusals.
#[derive(serde::Serialize)]
struct ReloadEvent {
	state: &'static str,
	summary: Option<String>,
	problems: Vec<Problem>
}

impl Event for ReloadEvent {
	const EVENT: &'static str = "auxtoolsReload";
}

/// Refused change with its source location.
#[derive(serde::Serialize)]
struct Problem {
	/// Absolute path; fall back to the `.dme` when no source file is known.
	file: Option<String>,
	line: Option<u32>,
	message: String
}

/// Current reload state, the last state before `reloading`, and refused
/// changes still missing from the world.
#[derive(Default)]
pub(super) struct ReloadStatus {
	reloading: bool,
	settled: Option<(&'static str, Option<String>)>,
	refusals: Vec<Refusal>
}

impl Adapter {
	/// Report reload state to the client, with the outstanding refusals as
	/// problems. Report `refused` instead of `active` or `reloaded` while any
	/// remain.
	pub(super) fn reload_state(&mut self, state: &'static str, summary: Option<String>) {
		self.reload_status.reloading = state == "reloading";
		if state != "reloading" {
			self.reload_status.settled = Some((state, summary.clone()));
		}
		let outstanding = !self.reload_status.refusals.is_empty();
		let shown = if outstanding && matches!(state, "active" | "reloaded") {
			"refused"
		} else {
			state
		};
		let dme = self.files.lock().unwrap().dme().map(|dme| dme.display().to_string());
		let problems = self
			.reload_status
			.refusals
			.iter()
			.map(|refusal| Problem {
				file: refusal.file.clone().or_else(|| dme.clone()),
				line: refusal.line,
				message: format!("{:#}", refusal)
			})
			.collect();
		self.out.event(ReloadEvent {
			state: shown,
			summary,
			problems
		});
	}

	/// Replace the outstanding refusals and report the settled state again.
	/// A reload in progress reports when it ends.
	pub(super) fn outstanding(&mut self, refusals: Vec<Refusal>) {
		self.reload_status.refusals = refusals;
		if !self.reload_status.reloading {
			if let Some((state, summary)) = self.reload_status.settled.clone() {
				self.reload_state(state, summary);
			}
		}
	}

	/// Forget refusals from a replaced source worker.
	pub(super) fn forget_refusals(&mut self) {
		self.reload_status.refusals.clear();
	}

	/// Begin reload progress.
	pub(super) fn reloading(&mut self) {
		self.progress_start(RELOADING, "Reloading changes".to_owned());
		self.reload_state("reloading", None);
	}

	/// Finish reload progress. Restore the previous state if nothing changed.
	pub(super) fn looked(&mut self) {
		self.progress_end(RELOADING);
		if self.reload_status.reloading {
			if let Some((state, summary)) = self.reload_status.settled.clone() {
				self.reload_state(state, summary);
			}
		}
	}

	/// Group refusals by subject and reason for the console. The client
	/// gets them with the outstanding set (`outstanding`).
	pub(super) fn refused(&mut self, refused: Vec<Refusal>, warnings: Vec<String>) {
		self.out.console("Hot reload requires a rebuild:");
		for group in grouped(&refused) {
			let text = format!("    {:#}", group);
			match &group.file {
				Some(file) => self.out.console_at(text, Path::new(file), group.line),
				None => self.out.console(text)
			}
		}
		self.journal_reload(
			false,
			"rebuild required".to_owned(),
			refused.iter().map(ToString::to_string).collect(),
			warnings
		);
	}
}

/// Merge refusals sharing a subject and reason, combining var names
/// and retaining the first location (`/area: layer, icon: why`).
fn grouped(refused: &[Refusal]) -> Vec<Refusal> {
	let mut groups: Vec<Refusal> = vec![];
	for refusal in refused {
		let same = groups
			.iter_mut()
			.find(|group| group.path == refusal.path && group.why == refusal.why && group.name.is_some() && refusal.name.is_some());
		match (same, &refusal.name) {
			(Some(group), Some(name)) => {
				let names = group.name.as_mut().expect("grouped by name");
				if !names.split(", ").any(|known| known == name) {
					names.push_str(", ");
					names.push_str(name);
				}
			}
			_ => groups.push(refusal.clone())
		}
	}
	groups
}

#[cfg(test)]
mod tests {
	use dmasm::reload::Refusal;

	use super::grouped;

	#[test]
	fn refusals_of_one_kind_are_one() {
		let why = "cannot restore this built-in type's original default";
		let refused = vec![
			Refusal::new("/area", why).named("layer"),
			Refusal::new("/area", why).named("luminosity"),
			Refusal::new("/mob", why).named("layer"),
			Refusal::new("/area", why).named("icon"),
			Refusal::new("/area", "another reason").named("icon"),
			Refusal::new("/proc/f", "doesn't compile: x"),
		];
		let shown: Vec<String> = grouped(&refused).iter().map(|r| format!("{:#}", r)).collect();
		assert_eq!(shown, [
			format!("/area: layer, luminosity, icon: {}", why),
			format!("/mob: layer: {}", why),
			"/area: icon: another reason".to_owned(),
			"/proc/f: doesn't compile: x".to_owned()
		]);
	}
}
