//! Client login scripts from `client/script` and included `.dms` files.
//!
//! World data stores a text string ID (0xFFFF for none) and an ordered list
//! of resource IDs: included `.dms` files, then the `client/script` file.
//! At login, s2c 0x8D sends the file contents followed by the text. Reload
//! changes only the text field; resource-file scripts remain as built.

use std::sync::{Mutex, OnceLock};

use auxtools::{
	byond_scan::{Anchor, Extract, OperandSelect, Recipe, RegMemDispSelect, Register, SignatureTreatment, VersionRange},
	raw_types::values::{ValueData, ValueTag},
	shutdown, StringRef, Value
};

use crate::server_types::{ScriptReloaded, ScriptRequest};

/// Sentinel for no script text.
const NO_TEXT: u32 = 0xFFFF;

// Every pattern is the start of the function that sends the script to one
// connection (u16 connection, cdecl), through its first reads of the world's
// data: `mov r,[G]; mov r,[r+0xC]; cmp word [r+N],...`. G points to the world's
// data at +0xC; N is the file count, 0x48 on 515 and 0x58 on 516. The list
// is just before the count and the text just before the list.
#[cfg(unix)]
mod recipes {
	use super::*;

	// 515.1637 to 515.1643, built with frame pointers
	const EARLY: &str = "55 89 E5 57 56 53 81 EC ?? ?? ?? ?? 0F B7 45 08 8D 95 ?? ?? ?? ?? 89 14 24 C7 44 24 04 8D 00 00 00 66 89 85 ?? ?? ?? ?? E8 \
	                     ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 8B 42 0C 66 83 78 ?? 00 0F 84";
	const LATER: &str = "55 89 E5 57 56 8D BD ?? ?? ?? ?? 53 81 EC ?? ?? ?? ?? 8B 45 08 68 8D 00 00 00 57 89 85 ?? ?? ?? ?? E8 ?? ?? ?? ?? A1 ?? ?? \
	                     ?? ?? 83 C4 10 8B 40 0C 66 83 78 ?? 00 0F 84";
	const EARLY_BUILDS: VersionRange = VersionRange { min: 1637, max: 1643 };
	const LATER_BUILDS: VersionRange = VersionRange { min: 1644, max: 1688 };

	const fn recipe(versions: VersionRange, signature: &'static str, extract: Extract) -> Recipe {
		Recipe {
			name: "client_script_send",
			versions,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, signature),
			hops: &[],
			extract
		}
	}

	pub const GLOBAL: &[Recipe] = &[
		recipe(EARLY_BUILDS, EARLY, Extract::AbsMem(OperandSelect::FirstAbsMem)),
		recipe(LATER_BUILDS, LATER, Extract::AbsMem(OperandSelect::FirstAbsMem))
	];
	pub const DATA: &[Recipe] = &[
		recipe(EARLY_BUILDS, EARLY, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EDX })),
		recipe(LATER_BUILDS, LATER, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX }))
	];
	pub const COUNT: &[Recipe] = &[
		recipe(EARLY_BUILDS, EARLY, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX })),
		recipe(
			LATER_BUILDS,
			LATER,
			Extract::RegMemDisp(RegMemDispSelect::Nth {
				base: Register::EAX,
				index: 1
			})
		)
	];
}

#[cfg(windows)]
mod recipes {
	use super::*;

	// After frame and exception setup: push 0x8D, construct the message,
	// then mov edx,[G] at byte 64.
	const SIGNATURE: &str = "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 \
	                         F4 64 A3 00 00 00 00 68 8D 00 00 00 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 33 C0 33 DB C7 45 FC 00 00 00 \
	                         00 33 FF 8B 4A 0C 66 3B 41 ?? 0F 83";
	const BUILDS: VersionRange = VersionRange { min: 1637, max: 1688 };

	const fn recipe(anchor: SignatureTreatment, extract: Extract) -> Recipe {
		Recipe {
			name: "client_script_send",
			versions: BUILDS,
			anchor: Anchor::Signature(anchor, SIGNATURE),
			hops: &[],
			extract
		}
	}

	pub const GLOBAL: &[Recipe] = &[recipe(SignatureTreatment::OffsetByInt(64), Extract::Entry)];
	pub const DATA: &[Recipe] = &[recipe(
		SignatureTreatment::NoOffset,
		Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EDX })
	)];
	pub const COUNT: &[Recipe] = &[recipe(
		SignatureTreatment::NoOffset,
		Extract::RegMemDisp(RegMemDispSelect::First { base: Register::ECX })
	)];
}

/// Script-field location: global pointer, world-data offset and file-count
/// offset.
struct Place {
	global: usize,
	data: usize,
	count: usize
}

fn place() -> Result<&'static Place, String> {
	static PLACE: OnceLock<Result<Place, String>> = OnceLock::new();
	PLACE
		.get_or_init(|| {
			Ok(Place {
				global: auxtools::find_recipe_of(recipes::GLOBAL)?,
				data: auxtools::find_recipe_of(recipes::DATA)?,
				count: auxtools::find_recipe_of(recipes::COUNT)?
			})
		})
		.as_ref()
		.map_err(|e| format!("cannot locate the client script: {}", e))
}

/// The script's three fields in the world's data
struct Fields {
	text: *mut u32,
	list: *const *const u32,
	count: *const u16
}

fn fields() -> Result<Fields, String> {
	let place = place()?;
	// SAFETY: the global and the offsets come from the code that reads the
	// fields, on this build; the pointers are checked before they're followed.
	unsafe {
		let holder = *(place.global as *const *const u8);
		if holder.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		let data = *(holder.add(place.data) as *const *mut u8);
		if data.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		Ok(Fields {
			text: data.add(place.count - 8) as *mut u32,
			list: data.add(place.count - 4) as *const *const u32,
			count: data.add(place.count) as *const u16
		})
	}
}

impl Fields {
	fn files(&self) -> Vec<u32> {
		// SAFETY: the list holds `count` resource ids
		unsafe {
			let count = *self.count as usize;
			let list = *self.list;
			if count == 0 || list.is_null() {
				return vec![];
			}
			std::slice::from_raw_parts(list, count).to_vec()
		}
	}

	fn text(&self) -> Option<u32> {
		// SAFETY: see `fields`
		let id = unsafe { *self.text };
		(id != NO_TEXT).then_some(id)
	}
}

/// Text installed by this library: `Some(None)` means no text, `None`
/// means unchanged since build. The field owns a string reference.
static OUR_TEXT: Mutex<Option<Option<u32>>> = Mutex::new(None);

/// Clear tracking at world shutdown; reboot restores the built script.
#[shutdown]
fn forget_world() {
	*OUR_TEXT.lock().unwrap() = None;
}

/// Validate the text field against `built` before the first write
/// (0xFFFF for none). Its offset is inferred from the file count, so
/// reject an unexpected layout before modifying world data.
fn check_built(fields: &Fields, built: Option<&str>) -> Result<(), String> {
	if OUR_TEXT.lock().unwrap().is_some() {
		return Ok(());
	}
	// Identical text has the same interned string ID.
	let expected = match built {
		Some(text) => Some(StringRef::new(text).map_err(|e| e.message)?),
		None => None
	};
	// SAFETY: see `fields`
	let found = unsafe { *fields.text };
	if found == expected.as_ref().map_or(NO_TEXT, |string| string.get_id().0) {
		return Ok(());
	}
	Err(
		"client script does not match the build; rebuild the world. If the source already matches, this BYOND script layout is unsupported"
			.to_owned()
	)
}

/// Replace script text, or clear it with `None`. Transfer the new string
/// reference to the field and release the old one only if we installed it.
fn write_text(fields: &Fields, text: Option<Value>) {
	let id = match text {
		Some(value) => {
			// SAFETY: a string value's data is its string id
			let id = unsafe { value.raw.data.id };
			std::mem::forget(value);
			Some(id)
		}
		None => None
	};
	let old = OUR_TEXT.lock().unwrap().replace(id);
	// SAFETY: see `fields`
	unsafe { *fields.text = id.unwrap_or(NO_TEXT) };
	if let Some(Some(old)) = old {
		// SAFETY: the reference `write_text` kept for the string it put there
		unsafe {
			auxtools::raw_types::funcs::dec_ref_count(auxtools::raw_types::values::Value {
				tag: ValueTag::String,
				data: ValueData { id: old }
			});
		}
	}
}

/// Script sent at login: each file's content and a newline, then the text.
fn full(fields: &Fields) -> Result<String, String> {
	let mut script = String::new();
	for id in fields.files() {
		// SAFETY: a resource id from the server's list
		let file = unsafe { Value::new(ValueTag::Resource, ValueData { id }) };
		let text = crate::skin::run("file2text(file)", &[("file", &file)])?;
		script += &text.as_string().map_err(|e| e.message)?;
		script.push('\n');
	}
	if let Some(id) = fields.text() {
		// SAFETY: a string id the field holds
		let text = unsafe { Value::new(ValueTag::String, ValueData { id }) };
		script += &text.as_string().map_err(|e| e.message)?;
	}
	Ok(script)
}

/// Apply the requested script text. Validate before changing world data.
pub fn set(request: &ScriptRequest) -> Result<ScriptReloaded, String> {
	let fields = fields()?;
	check_built(&fields, request.built.as_deref())?;
	let text = match &request.text {
		Some(text) => Some(Value::from_string(text).map_err(|e| e.message)?),
		None => None
	};
	let clients = crate::skin::clients()?;
	write_text(&fields, text);
	Ok(ScriptReloaded {
		clients,
		hosted: crate::skin::hosted()
	})
}

/// Inspect script string ID, file resource IDs and combined login script
/// for `#script`.
pub fn state() -> Result<String, String> {
	let fields = fields()?;
	let text = fields.text().map_or("none".to_owned(), |id| id.to_string());
	let files: Vec<String> = fields.files().iter().map(u32::to_string).collect();
	Ok(format!("text {}\nfiles {}\n{}", text, files.join(" "), full(&fields)?))
}
