//! Resolve stddef.dm proc locations for stack frames. The debug server supplies
//! the source text; normal proc locations come from the world's debug info.

use std::collections::HashMap;

use dreammaker::Context;

/// A proc as the debug server knows it
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProcId {
	/// `/type/name`, `/name` for a global proc
	pub path: String,
	pub override_id: u32
}

/// `/mob/proc/Life` as `/mob/Life`
pub fn strip_declaration(path: &str) -> String {
	let mut parts: Vec<&str> = path.split('/').collect();
	if parts.len() >= 3 && matches!(parts[parts.len() - 2], "proc" | "verb") {
		parts.remove(parts.len() - 2);
	}
	parts.join("/")
}

/// Parse stddef.dm proc locations from the source supplied by the server.
pub struct Stddef {
	pub text: String,
	lines: HashMap<ProcId, u32>
}

impl Stddef {
	pub fn new(text: String) -> Self {
		let ctx = Context::default();
		let pp = dreammaker::Preprocessor::from_buffer(&ctx, "stddef.dm".into(), text.as_str());
		let tree = dreammaker::Parser::new(&ctx, pp).parse_object_tree_without_builtins();

		let mut lines = HashMap::new();
		for ty in tree.iter_types() {
			for (name, type_proc) in ty.get().procs.iter() {
				if let Some(value) = type_proc.value.first() {
					lines.insert(
						ProcId {
							path: format!("{}/{}", ty.get().path, name),
							override_id: 0
						},
						value.location.line
					);
				}
			}
		}
		Self { text, lines }
	}

	pub fn line(&self, path: &str, override_id: u32) -> Option<u32> {
		self.lines
			.get(&ProcId {
				path: strip_declaration(path),
				override_id
			})
			.copied()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn declarations_are_stripped() {
		assert_eq!(strip_declaration("/mob/proc/Life"), "/mob/Life");
		assert_eq!(strip_declaration("/proc/global"), "/global");
		assert_eq!(strip_declaration("/mob/verb/say"), "/mob/say");
		assert_eq!(strip_declaration("/mob/Login"), "/mob/Login");
	}
}
