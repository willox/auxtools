//! Restore inherited built-in defaults after override removals or parent
//! changes. Read from the running parent: source metadata does not always
//! include BYOND's own defaults.

use super::*;

/// Locate a built-in in the type record. Only the location matters,
/// so any accepted value is sufficient.
fn places(var: &str) -> Result<Vec<BuiltinWrite>, String> {
	[ConstValue::Null, ConstValue::Number(1.0), ConstValue::String("neuter".to_owned())]
		.iter()
		.find_map(|value| builtin_writes(var, value).ok())
		.ok_or_else(|| format!("{}: built-in var is unsupported by hot reload", var))
}

/// A movable's bound and step vars are also kept worked out in a block at
/// +0x40: bound_x and bound_y (16 bits each), bound_width and bound_height,
/// then step_x, step_y and step_size as floats. (byte offset, mask)
fn bounds_place(var: &str) -> Option<(usize, u32)> {
	Some(match var {
		"bound_x" => (0x40, 0xFFFF),
		"bound_y" => (0x40, 0xFFFF_0000),
		"bound_width" => (0x44, 0xFFFF),
		"bound_height" => (0x44, 0xFFFF_0000),
		"step_x" => (0x48, u32::MAX),
		"step_y" => (0x4C, u32::MAX),
		"step_size" => (0x50, u32::MAX),
		_ => return None
	})
}

/// Type built-in list entries: name string ID, then PushVal-encoded value
/// (numbers use an extra word).
pub(super) fn listed_entries(list: &[u32]) -> Vec<Vec<u32>> {
	let mut out = vec![];
	let mut at = 0;
	while at < list.len() {
		let len = if list.get(at + 1).is_some_and(|tag| tag & 0xFF == 0x2A) { 4 } else { 3 };
		out.push(list[at..(at + len).min(list.len())].to_vec());
		at += len;
	}
	out
}

fn listed(type_id: u32) -> Result<Vec<Vec<u32>>, String> {
	Ok(listed_entries(
		&runtime_tables::misc_words(runtime_tables::type_field(type_id, type_fields::BUILTIN_VALUES)?).unwrap_or_default()
	))
}

fn set_listed(type_id: u32, entries: &[Vec<u32>]) -> Result<(), String> {
	let words: Vec<u32> = entries.iter().flatten().copied().collect();
	let list = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
	runtime_tables::set_type_field(type_id, type_fields::BUILTIN_VALUES, list)
}

/// Retain an additional reference for a string copied into a type record.
fn keep_string(id: u32) {
	if id != NONE {
		unsafe {
			raw_types::funcs::inc_ref_count(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { id }
			});
		}
	}
}

/// Gives `type_id` the value `from` has for a built-in var.
pub(super) fn copy_builtin(type_id: u32, from: u32, var: &str) -> Result<(), String> {
	use runtime_tables::{set_type_field, type_field};
	use BuiltinWrite::*;
	for place in places(var)? {
		match place {
			Word(offset, _) => {
				let word = type_field(from, offset)?;
				if offset != type_fields::ICON && offset != type_fields::LAYER {
					keep_string(word);
				}
				set_type_field(type_id, offset, word)?
			}
			Half(offset, _) => {
				let half = (type_field(from, offset & !3)? >> ((offset & 2) * 8)) as u16;
				runtime_tables::set_type_half(type_id, offset, half)?
			}
			Byte(offset, _) => {
				let byte = (type_field(from, offset & !3)? >> ((offset & 3) * 8)) as u8;
				runtime_tables::set_type_byte(type_id, offset, byte)?
			}
			Bits(offset, mask, _) => {
				let word = type_field(type_id, offset)?;
				set_type_field(type_id, offset, (word & !mask) | (type_field(from, offset)? & mask))?
			}
			Listed(name, _) => {
				let mut entries: Vec<Vec<u32>> = listed(type_id)?.into_iter().filter(|e| e[0] != name).collect();
				entries.extend(listed(from)?.into_iter().filter(|e| e[0] == name));
				set_listed(type_id, &entries)?;
				if let Some((offset, mask)) = bounds_place(var) {
					let word = type_field(type_id, offset)?;
					set_type_field(type_id, offset, (word & !mask) | (type_field(from, offset)? & mask))?;
				}
			}
			Mob(var, _) => {
				// (only mob types are in the mob table)
				let mut parent = None;
				if runtime_tables::set_mob_type(from, |mob| parent = Some(*mob)).is_err() {
					continue;
				}
				let parent = parent.unwrap();
				if var == "key" {
					keep_string(parent.key);
				}
				runtime_tables::set_mob_type(type_id, |mob| match var {
					"key" => mob.key = parent.key,
					"sight" => mob.sight = parent.sight,
					"see_in_dark" => mob.see_in_dark = parent.see_in_dark,
					_ => mob.see_invisible = parent.see_invisible
				})?
			}
		}
	}
	Ok(())
}

/// Types inheriting built-in values, in parent-first order.
pub(super) fn prepare(inherited: &[BuiltinInherited], prepared: &mut Prepared, errors: &mut Vec<String>) {
	let mut out = vec![];
	for i in inherited {
		if let Err(e) = places(&i.var) {
			errors.push(e);
			continue;
		}
		for path in &i.types {
			match find_type(path) {
				Ok(type_id) => {
					out.push((type_and_ancestors(type_id).len(), type_id, i.var.clone()));
					prepared.appearances.insert(type_id);
				}
				Err(e) => errors.push(e)
			}
		}
	}
	out.sort();
	prepared.builtin_inherits = out.into_iter().map(|(_, t, var)| (t, var)).collect();
}

/// Apply inherited defaults after the reload's explicit built-in values.
pub(super) fn apply(inherits: &[(u32, String)], warnings: &mut Vec<String>) {
	for (type_id, var) in inherits {
		let copied = runtime_tables::type_field(*type_id, type_fields::PARENT).and_then(|parent| copy_builtin(*type_id, parent, var));
		if let Err(e) = copied {
			warnings.push(format!("{}: could not inherit built-in {}: {}", type_path(*type_id), var, e));
		}
	}
}
