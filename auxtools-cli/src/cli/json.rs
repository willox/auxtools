//! JSON result formatting for `--json`.

use auxtools_cli::server_types::*;
use serde_json::{json, Map, Value};

use super::proc_name;

/// How deep `--json` writes out lists and objects unless told
pub const DEFAULT_DEPTH: u32 = 2;

/// Serialize a DM value. Lists use arrays or keyed objects where possible.
/// Objects include ref, type and name, with variables to the requested depth.
/// Other values use tagged representations such as `{"path": ...}`.
pub fn value(rendered: &Rendered) -> Value {
	match rendered {
		Rendered::Null => Value::Null,
		Rendered::Number(number) => self::number(*number),
		Rendered::Text(text) => Value::String(text.clone()),
		Rendered::List { len, entries: None } => json!({ "list": true, "len": len }),
		Rendered::List { entries: Some(entries), .. } => list(entries),
		Rendered::Object { reference, kind, name, vars } => {
			let mut object = json!({ "ref": reference, "type": kind, "name": name });
			if let Some(vars) = vars {
				object["vars"] = named(vars);
			}
			object
		}
		Rendered::Other { kind, text } if kind == "path" => json!({ "path": text }),
		Rendered::Other { kind, text } => json!({ "kind": kind, "text": text })
	}
}

/// Names and their values, as one object
pub fn named(values: &[(String, Rendered)]) -> Value {
	Value::Object(values.iter().map(|(name, rendered)| (name.clone(), value(rendered))).collect())
}

/// Format BYOND numbers without float conversion artifacts.
/// Non-finite values use tagged objects, e.g. `{"number": "inf"}`.
fn number(number: f32) -> Value {
	if !number.is_finite() {
		let text = if number.is_nan() {
			"nan"
		} else if number > 0.0 {
			"inf"
		} else {
			"-inf"
		};
		return json!({ "number": text });
	}
	match number.to_string().parse::<serde_json::Number>() {
		Ok(number) => Value::Number(number),
		Err(_) => json!(f64::from(number))
	}
}

/// Serialize a list as an array, a text-keyed object, or key/value entries.
fn list(entries: &[(Rendered, Option<Rendered>)]) -> Value {
	if entries.iter().all(|(_, value)| value.is_none()) {
		return Value::Array(entries.iter().map(|(key, _)| value(key)).collect());
	}
	let mut object = Map::new();
	for (key, entry) in entries {
		let Rendered::Text(key) = key else {
			break;
		};
		if object.insert(key.clone(), entry.as_ref().map_or(Value::Null, value)).is_some() {
			break;
		}
	}
	if object.len() == entries.len() {
		return Value::Object(object);
	}
	Value::Array(
		entries
			.iter()
			.map(|(key, entry)| json!({ "key": value(key), "value": entry.as_ref().map_or(Value::Null, value) }))
			.collect()
	)
}

/// A frame of a stack
pub fn frame(frame: &StackFrame) -> Value {
	json!({
		"proc": proc_name(&frame.instruction.proc),
		"file": frame.file,
		"line": frame.line,
		"offset": frame.instruction.offset,
		"old_code": frame.old_code
	})
}

/// Shared execution status: process, pause reason, stack and optional
/// instruction. Unavailable fields are null or empty arrays.
pub fn execution(pid: u32, paused: Option<(&BreakpointReason, &[StackFrame], Option<(u32, String)>)>) -> Map<String, Value> {
	let mut document = json!({
		"pid": pid,
		"paused": false,
		"reason": null,
		"message": null,
		"caught": null,
		"changes": [],
		"stack": [],
		"instruction": null
	});
	if let Some((reason, frames, instruction)) = paused {
		let (word, message, caught, changes) = match reason {
			BreakpointReason::Breakpoint => ("breakpoint", None, None, vec![]),
			BreakpointReason::Step => ("step", None, None, vec![]),
			BreakpointReason::Pause => ("pause", None, None, vec![]),
			BreakpointReason::Runtime { message, caught } => ("runtime", Some(message.clone()), Some(*caught), vec![]),
			BreakpointReason::Data { changes } => (
				"change",
				None,
				None,
				changes
					.iter()
					.map(|change| json!({ "name": change.name, "old": change.old, "new": change.new }))
					.collect()
			)
		};
		document["paused"] = json!(true);
		document["reason"] = json!(word);
		document["message"] = json!(message);
		document["caught"] = json!(caught);
		document["changes"] = json!(changes);
		document["stack"] = frames.iter().map(frame).collect();
		document["instruction"] = json!(instruction.map(|(offset, text)| json!({ "offset": offset, "text": text })));
	}
	match document {
		Value::Object(map) => map,
		_ => unreachable!()
	}
}

#[cfg(test)]
mod tests {
	use auxtools_cli::server_types::Rendered;
	use serde_json::json;

	use super::value;

	#[test]
	fn a_running_world_has_every_field_and_nothing_in_them() {
		let running = super::execution(12, None);
		assert_eq!(
			serde_json::Value::Object(running),
			json!({ "pid": 12, "paused": false, "reason": null, "message": null, "caught": null, "changes": [], "stack": [], "instruction": null })
		);
		let reason = auxtools_cli::server_types::BreakpointReason::Runtime {
			message: "bad index".to_owned(),
			caught: true
		};
		let paused = super::execution(12, Some((&reason, &[], Some((4, "Ret".to_owned())))));
		assert_eq!(
			serde_json::Value::Object(paused),
			json!({
				"pid": 12, "paused": true, "reason": "runtime", "message": "bad index", "caught": true, "changes": [], "stack": [],
				"instruction": { "offset": 4, "text": "Ret" }
			})
		);
	}

	fn text(text: &str) -> Rendered {
		Rendered::Text(text.to_owned())
	}

	#[test]
	fn numbers_are_as_byond_writes_them() {
		assert_eq!(value(&Rendered::Number(42.0)), json!(42));
		assert_eq!(value(&Rendered::Number(0.1)).to_string(), "0.1");
		assert_eq!(value(&Rendered::Number(-2.5)), json!(-2.5));
		assert_eq!(value(&Rendered::Number(f32::INFINITY)), json!({ "number": "inf" }));
		assert_eq!(value(&Rendered::Number(f32::NAN)), json!({ "number": "nan" }));
	}

	#[test]
	fn a_list_is_an_array_or_an_object_with_its_keys() {
		let plain = Rendered::List {
			len: 2,
			entries: Some(vec![(Rendered::Number(1.0), None), (text("two"), None)])
		};
		assert_eq!(value(&plain), json!([1, "two"]));
		let keyed = Rendered::List {
			len: 2,
			entries: Some(vec![(text("a"), Some(Rendered::Number(1.0))), (text("b"), None)])
		};
		assert_eq!(value(&keyed), json!({ "a": 1, "b": null }));
		// Keys JSON can't have, or one twice
		for entries in [
			vec![
				(Rendered::Number(1.0), None),
				(text("b"), Some(Rendered::Null)),
				(text("c"), Some(text("x"))),
			],
			vec![(text("a"), Some(text("x"))), (text("a"), Some(text("y")))]
		] {
			let list = Rendered::List {
				len: 2,
				entries: Some(entries.clone())
			};
			let pairs: Vec<_> = entries
				.iter()
				.map(|(key, entry)| json!({ "key": value(key), "value": entry.as_ref().map_or(json!(null), value) }))
				.collect();
			assert_eq!(value(&list), json!(pairs));
		}
		assert_eq!(value(&Rendered::List { len: 7, entries: None }), json!({ "list": true, "len": 7 }));
	}

	#[test]
	fn an_object_is_its_ref_type_and_name_and_its_vars_when_written_out() {
		let mob = |vars| Rendered::Object {
			reference: "[0x3000000]".to_owned(),
			kind: "/mob/living".to_owned(),
			name: Some("Bob".to_owned()),
			vars
		};
		assert_eq!(value(&mob(None)), json!({ "ref": "[0x3000000]", "type": "/mob/living", "name": "Bob" }));
		let opened = value(&mob(Some(vec![("health".to_owned(), Rendered::Number(100.0))])));
		assert_eq!(opened["vars"], json!({ "health": 100 }));
		let path = Rendered::Other {
			kind: "path".to_owned(),
			text: "/obj/item".to_owned()
		};
		assert_eq!(value(&path), json!({ "path": "/obj/item" }));
		let other = Rendered::Other {
			kind: "proc".to_owned(),
			text: "/proc/x".to_owned()
		};
		assert_eq!(value(&other), json!({ "kind": "proc", "text": "/proc/x" }));
	}
}
