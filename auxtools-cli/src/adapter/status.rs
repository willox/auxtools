//! Source-linked reload refusals for console output.

use dmasm::reload::Refusal;

use super::*;

impl Adapter {
	/// Group refusals by subject and reason, with source links.
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
