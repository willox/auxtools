//! Login skin from the world's `.dmf` interface file.
//!
//! World data stores a resource ID read at every login (s2c 0xEF with the
//! skin CRC, then 0xAF with its text). Replace that ID for future logins.
//! Connected clients retain their interface: resending 0xAF would rebuild
//! windows without their login-time setup.

use std::sync::OnceLock;

use auxtools::{
	byond_scan::{Anchor, Extract, OperandSelect, Recipe, RegMemDispSelect, Register, SignatureTreatment, VersionRange},
	raw_types::values::{ValueData, ValueTag},
	Proc, Value
};

use crate::server_types::{Reconnected, SkinReloaded, SkinRequest};

/// The skin field holds this when the world has no skin
const NO_SKIN: u32 = 0xFFFF;

// Every pattern starts where the server builds the skin message (0xAF) and
// covers its reading of the skin: `mov eax,[G]; mov r,[eax+0xC]; mov r,[r+F];
// cmp r,0xFFFF`. G points to the world's data at +0xC; F is 0x4C on 515 and
// 0x5C on 516.
#[cfg(unix)]
mod recipes {
	use super::*;

	// 515.1637 to 515.1643, built with frame pointers
	const EARLY: &str = "C7 44 24 04 AF 00 00 00 89 1C 24 E8 ?? ?? ?? ?? 89 B5 ?? ?? ?? ?? 89 34 24 E8 ?? ?? ?? ?? 85 C0 89 C7 74 ?? A1 ?? ?? ?? ?? \
	                     8B 40 0C 8B 50 ?? 81 FA FF FF 00 00";
	const LATER: &str = "68 AF 00 00 00 53 0F B7 FF E8 ?? ?? ?? ?? 89 3C 24 E8 ?? ?? ?? ?? 89 C6 83 C4 10 85 C0 74 ?? A1 ?? ?? ?? ?? 8B 40 0C 8B 50 \
	                     ?? 81 FA FF FF 00 00";
	const EARLY_BUILDS: VersionRange = VersionRange { min: 1637, max: 1643 };
	const LATER_BUILDS: VersionRange = VersionRange { min: 1644, max: 1688 };

	const fn recipe(versions: VersionRange, signature: &'static str, extract: Extract) -> Recipe {
		Recipe {
			name: "world_skin",
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
		recipe(EARLY_BUILDS, EARLY, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX })),
		recipe(LATER_BUILDS, LATER, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX }))
	];
	pub const FIELD: &[Recipe] = &[
		recipe(
			EARLY_BUILDS,
			EARLY,
			Extract::RegMemDisp(RegMemDispSelect::Nth {
				base: Register::EAX,
				index: 1
			})
		),
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

	const SIGNATURE: &str = "68 AF 00 00 00 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 56 C7 45 FC ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B D8 83 C4 04 85 DB 0F 84 ?? \
	                         ?? ?? ?? A1 ?? ?? ?? ?? 8B 48 0C 8B 41 ?? 3D FF FF 00 00";
	const BUILDS: VersionRange = VersionRange { min: 1637, max: 1688 };

	const fn recipe(extract: Extract) -> Recipe {
		Recipe {
			name: "world_skin",
			versions: BUILDS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SIGNATURE),
			hops: &[],
			extract
		}
	}

	pub const GLOBAL: &[Recipe] = &[recipe(Extract::AbsMem(OperandSelect::FirstAbsMem))];
	pub const DATA: &[Recipe] = &[recipe(Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX }))];
	pub const FIELD: &[Recipe] = &[recipe(Extract::RegMemDisp(RegMemDispSelect::First { base: Register::ECX }))];
}

/// Skin-field location: global pointer, world-data offset and field offset.
struct Place {
	global: usize,
	data: usize,
	field: usize
}

/// Field location is stable across worlds loaded in this process.
fn place() -> Result<&'static Place, String> {
	static PLACE: OnceLock<Result<Place, String>> = OnceLock::new();
	PLACE
		.get_or_init(|| {
			Ok(Place {
				global: auxtools::find_recipe_of(recipes::GLOBAL)?,
				data: auxtools::find_recipe_of(recipes::DATA)?,
				field: auxtools::find_recipe_of(recipes::FIELD)?
			})
		})
		.as_ref()
		.map_err(|e| format!("cannot locate the skin: {}", e))
}

/// The world's skin field
fn field() -> Result<*mut u32, String> {
	let place = place()?;
	// SAFETY: the global and both offsets come from the code that reads the
	// field, on this build; the pointers are checked before they're followed.
	unsafe {
		let holder = *(place.global as *const *const u8);
		if holder.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		let data = *(holder.add(place.data) as *const *mut u8);
		if data.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		Ok(data.add(place.field) as *mut u32)
	}
}

/// Whether Dream Seeker runs the world, rather than DreamDaemon
fn hosted() -> bool {
	std::env::current_exe()
		.ok()
		.and_then(|exe| exe.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()))
		.is_some_and(|stem| stem == "dreamseeker")
}

/// Run DM `code`, an expression or a proc body, with `params` set, and
/// return its value
fn run(code: &str, params: &[(&str, &Value)]) -> Result<Value, String> {
	let names: Vec<&str> = params.iter().map(|(name, _)| *name).collect();
	let values: Vec<&Value> = params.iter().map(|(_, value)| *value).collect();
	let body = dmasm::compiler::compile_body_in(code, &names, None, crate::world_byond()).map_err(|e| e.to_string())?;
	let assembly = dmasm::assembler::assemble(&body.nodes, &mut crate::assemble_env::AssembleEnv).map_err(|e| format!("{:?}", e))?;
	let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no evaluation stub")?;
	proc.set_bytecode(assembly);
	let result = crate::server::Server::call_with_locals(&proc, &body.locals, &values)?.map_err(|e| e.message)?;
	// The stub returns list(result, params...)
	result.as_list().and_then(|list| list.get(1)).map_err(|e| e.message)
}

fn text(value: &str) -> Result<Value, String> {
	Value::from_string(value).map_err(|e| e.message)
}

/// Make `request.path` the world's skin
pub fn set(request: &SkinRequest) -> Result<SkinReloaded, String> {
	let field = field()?;
	if !std::path::Path::new(&request.path).is_file() {
		return Err(format!("{}: no such file", request.path));
	}
	let resource = run("fcopy_rsc(file)", &[("file", &text(&request.path)?)])?;
	if resource.raw.tag != ValueTag::Resource {
		return Err(format!("{}: could not cache the skin resource", request.path));
	}
	// Count clients before changing the skin so a counting error cannot
	// report an already-applied change as failed.
	let clients = clients()?;
	// SAFETY: `field` is the world's skin field (see `field`); the resource
	// stays in the world's cache for the rest of the session.
	unsafe { *field = resource.raw.data.id };
	Ok(SkinReloaded { clients, hosted: hosted() })
}

/// Count logged-in clients by key. The idle-world waker has no key;
/// winset() on it would raise a runtime error.
fn clients() -> Result<u32, String> {
	let count = run("var/n = 0\nfor(var/client/C)\n\tif(C.key)\n\t\tn++\nn\n", &[])?;
	count.as_number().map(|n| n as u32).map_err(|e| e.message)
}

/// Inspect the skin field's resource ID and text; `None` for no skin.
pub fn state() -> Result<Option<(u32, String)>, String> {
	// SAFETY: see `field`
	let id = unsafe { *field()? };
	if id == NO_SKIN {
		return Ok(None);
	}
	// SAFETY: a resource value made from the id the server itself uses
	let resource = unsafe { Value::new(ValueTag::Resource, ValueData { id }) };
	let text = run("file2text(skin)", &[("skin", &resource)])?;
	Ok(Some((id, text.as_string().map_err(|e| e.message)?)))
}

/// Reconnect clients unless Dream Seeker hosts the world. The host's
/// .reconnect would restart from the .dmb outside the debugger.
pub fn reconnect() -> Result<Reconnected, String> {
	if hosted() {
		return Ok(Reconnected::Hosted);
	}
	let count = run(
		"var/n = 0\nfor(var/client/C)\n\tif(C.key)\n\t\twinset(C, null, \"command=.reconnect\")\n\t\tn++\nn\n",
		&[]
	)?;
	count.as_number().map(|n| Reconnected::Clients(n as u32)).map_err(|e| e.message)
}
