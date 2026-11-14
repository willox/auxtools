//! Read and extend BYOND runtime tables for hot reload.
//! Use BYOND's allocators and table functions so ownership remains compatible
//! with its cleanup code.
//!
//! Layouts are based on 516.1688, with older-build differences documented
//! locally. Patterns were checked on every Windows and Linux build from
//! 515.1637 to 516.1688, and the tables on 515.1637, 515.1647, 516.1659,
//! 516.1685, 516.1686 and 516.1688.

use std::ffi::c_void;

use crate::raw_types::{misc, procs::ProcId};

/// A proc table entry as BYOND stores it, word by word: path, name, desc and
/// category string ids, the path's last part, flags, three packed bytes,
/// bytecode, locals and parameters misc ids, and the proc `..()` goes to.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcEntryWords(pub [u32; 11]);

impl ProcEntryWords {
	pub const BYTECODE: usize = 7;
	pub const CATEGORY: usize = 3;
	pub const DESC: usize = 2;
	pub const FLAGS: usize = 5;
	pub const LOCALS: usize = 8;
	pub const NAME: usize = 1;
	pub const NEXT_OVERRIDE: usize = 10;
	pub const PARAMETERS: usize = 9;
	pub const PATH: usize = 0;
	pub const PATH_BASENAME: usize = 4;
	/// A verb's `set src` (its range, then its kind) and invisibility, a byte
	/// each
	pub const SETTINGS: usize = 6;
}

/// No proc, string or misc entry.
pub const NONE: u32 = 0xFFFF;

/// One type's proc names, sorted by key.
#[repr(C)]
pub struct ProcNameMap {
	pub buckets: *mut ProcNameBucket,
	pub count: u16,
	pub built: u8
}

/// The procs a name finds on a type, most derived first. `ids` is a shared
/// buffer (`AllocShared`), and the same buffer can be in several types' maps.
#[repr(C)]
pub struct ProcNameBucket {
	/// The name's string id
	pub key: u32,
	pub ids: *mut u32,
	pub count: u16,
	/// Set for keys that are a proc's path basename rather than its name. Such
	/// a bucket builds no `..()` chain.
	pub no_link: u8
}

/// Byte offsets into a type table entry. The same on Windows and Linux.
pub mod type_fields {
	pub const PATH: usize = 0x00;
	pub const PARENT: usize = 0x04;
	/// String ids, or the resource id for the icon
	pub const NAME: usize = 0x08;
	pub const DESC: usize = 0x0C;
	pub const ICON: usize = 0x10;
	pub const ICON_STATE: usize = 0x14;
	pub const TEXT: usize = 0x18;
	pub const MAPTEXT: usize = 0x1C;
	/// 16 bits each
	pub const MAPTEXT_X: usize = 0x20;
	pub const MAPTEXT_Y: usize = 0x22;
	pub const MAPTEXT_WIDTH: usize = 0x24;
	pub const MAPTEXT_HEIGHT: usize = 0x26;
	/// Bit 1 opacity, bit 2 density, then `gender` (bits 6-7),
	/// `mouse_drop_zone` (bit 8), `mouse_opacity` (bits 12-13),
	/// `animate_movement` (bits 10, 14-15) and the low 11 bits of
	/// `appearance_flags` (from bit 21)
	pub const FLAGS: usize = 0x28;
	/// The rest of `appearance_flags`, from bit 0
	pub const FLAGS_HIGH: usize = 0x2C;
	/// A byte
	pub const DIR: usize = 0x32;
	/// Pointers to the type's own transform (6 floats) and color matrix (20)
	pub const TRANSFORM: usize = 0x34;
	pub const COLOR_MATRIX: usize = 0x38;
	pub const SUFFIX: usize = 0x54;
	/// A float
	pub const LAYER: usize = 0x5C;
	/// Misc id: the type's own verbs
	pub const VERBS: usize = 0x60;
	/// Misc id: the type's own procs
	pub const PROCS: usize = 0x64;
	/// Proc ID for computed variable initialization on new instances,
	/// merged from ancestor initializers at startup.
	pub const INIT_PROC: usize = 0x68;
	/// Misc id: (var id, flags) pairs for the vars the type declares
	pub const DECLARED_VARS: usize = 0x70;
	/// Misc id: (var id, tag, data) for the vars the type sets, as the .dmb has
	/// them; only read when the world starts
	pub const OWN_OVERRIDES: usize = 0x6C;
	/// Misc id: built-in vars set otherwise than in the record, its parent's
	/// too: the name's string id, then the value as bytecode has it
	pub const BUILTIN_VALUES: usize = 0x74;
	/// Misc id: every var the type has, (name string id, var id, tag, data)
	/// per var, sorted by name. Instances read their defaults from here.
	pub const VAR_TABLE: usize = 0x78;
	pub const EXTRA: usize = 0x7C;
	/// Variable initializer bytecode ranges used when merging init procs.
	pub const INIT_SEGMENTS: usize = 0x80;
	/// The type's place in the preorder of all types, and the end of its
	/// subtree's
	pub const PREORDER: usize = 0x84;
	pub const PREORDER_END: usize = 0x88;
	/// The type's typepath value (tag, data), from 515.1641 (see
	/// `TYPEPATH_BUILD`)
	pub const TYPEPATH: usize = 0x8C;
}

/// First build storing typepath values in type entries and indexing them
/// for text2path. Earlier entries end after PREORDER_END and compute
/// typepath values on demand.
const TYPEPATH_BUILD: u32 = 1641;

/// Linux builds before this one were compiled with frame pointers, and keep
/// some tables' pointers before their counts, as on Windows.
#[cfg(unix)]
const LINUX_SWAPPED_BUILD: u32 = 1644;

/// The size of a type table entry on `build`
fn type_stride(build: u32) -> usize {
	if build < TYPEPATH_BUILD {
		if cfg!(windows) {
			0x90
		} else {
			0x8C
		}
	} else if cfg!(windows) {
		0x98
	} else {
		0x94
	}
}

struct Functions {
	procdef_add: extern "C" fn(*const ProcEntryWords) -> u32,
	/// BYOND's own, or on builds that inlined it (Linux before 515.1644),
	/// `misc_intern`, which takes a 12-byte entry the caller allocated
	misc_intern: MiscIntern,
	alloc_shared: extern "C" fn(usize) -> *mut c_void,
	free_shared: extern "C" fn(*mut c_void) -> bool,
	copy_shared: extern "C" fn(*mut c_void) -> *mut c_void,
	/// Register a typepath for `text2path` (absent before `TYPEPATH_BUILD`).
	/// Only type additions require this; proc and var additions can proceed
	/// when it is unavailable.
	register_typepath: Result<Option<extern "C" fn(u32)>, String>,
	/// The size of a type table entry
	type_stride: usize,
	crt_realloc: extern "C" fn(*mut c_void, usize) -> *mut c_void,
	type_count: *const u32,
	type_table: *const *mut u8,
	name_maps: *const *mut ProcNameMap,
	/// Every type id in preorder
	preorder: *const *mut u32,
	/// Each type's default appearance, worked out when first needed (NONE
	/// until then)
	appearance_cache: *const *mut u32,
	/// The procs that return parameters' `in` lists, by the index a
	/// parameter's `in` list word has (`(index << 8) | 0x40`)
	in_list_count: *const u32,
	in_list_procs: *const *mut u32,
	/// Prefab instances (`/obj{a = 1}`, and the types placed on the map), by
	/// the id a value with tag 0x29 has: 12 bytes each (see `Instance`)
	instance_count: *const u32,
	instances: *const *mut Instance,
	/// Mob types, by the id a mob typepath value has (see `MobType`)
	mob_count: *const u32,
	mobs: *const *mut MobType,
	/// Separate world proc name map, built after global initialization.
	/// `built` is zero before construction; earlier edits are overwritten.
	world_name_map: *mut ProcNameMap,

	/// Global slot for each variable ID: global/static values and declared
	/// variable defaults. New locals start with their ID's slot value.
	global_values: *const *const crate::raw_types::values::Value,
	/// Misc entry of (var ID, flags) pairs, sorted by name string ID.
	/// `global.vars` enumerates it and binary-searches it for name lookups.
	/// Reloads can proceed without it, but new globals will be absent there.
	global_vars: Result<*mut u32, String>
}

#[derive(Clone, Copy)]
enum MiscIntern {
	/// Copies `count` words into a new entry, or finds an identical one
	Copy(extern "C" fn(*const u32, u32) -> u32),
	/// Interns an entry (u16 count, refcount at +4, words at +8) and its
	/// words, which it owns from then on (freeing them if one like it exists)
	Take(extern "C" fn(*mut u8) -> u32)
}

static mut FUNCTIONS: Option<Result<Functions, String>> = None;

/// Forgets what was looked up, for the next world.
pub(crate) fn reset() {
	unsafe {
		FUNCTIONS = None;
	}
}

#[cfg(windows)]
mod recipes {
	use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

	const CHECKED: VersionRange = VersionRange { min: 1637, max: 1688 };

	// Appends an entry to the proc table, or finds an identical one
	pub const PROCDEF_ADD: Recipe = Recipe {
		name: "procdef_add",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC A1 ?? ?? ?? ?? 53 8B 5D 08 56 33 F6 57 85 C0 74 ?? 33 FF A1 ?? ?? ?? ?? 03 C7 53 50 E8"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// Copies words into a new misc entry, or finds an identical one
	pub const MISC_INTERN_COPY: Recipe = Recipe {
		name: "misc_intern_copy",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 56 8B 35 ?? ?? ?? ?? 57 85 F6 74 0A 8B 46 08 A3 ?? ?? ?? ?? EB 0D 6A 0C FF 15 ?? ?? ?? ?? 83 C4 04 8B F0 85 F6 74 ?? 66 8B 4D \
			 0C"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// The proc name map of a type: the type count is operand 0, the maps 1. The
	// type table is the word before the count.
	const TYPE_MAP_ACCESSOR: &str = "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 77 0A A1 ?? ?? ?? ?? 8D 04 C8 5D C3 33 C0 5D C3";

	pub const TYPE_COUNT: Recipe = Recipe {
		name: "obj_type_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const NAME_MAPS: Recipe = Recipe {
		name: "proc_name_maps",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// The loader of the var tables, as auxtools' `variable_names` finds it.
	// global_values is operand 1.
	pub const GLOBAL_VALUES: Recipe = Recipe {
		name: "global_values",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 4B 01 00 00 A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 00 56 FF D3 83 C4 \
			 04 85 F6 74 08 85 C0 0F 84 2D 01 00 00 33 DB A3 ?? ?? ?? ?? 39 1D"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// Loader store of the sorted, interned global (var ID, flags) list.
	// The destination address is operand 0.
	pub const GLOBAL_VARS: Recipe = Recipe {
		name: "global_vars",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"0F B7 07 50 53 E8 ?? ?? ?? ?? 53 A3 ?? ?? ?? ?? FF 15 ?? ?? ?? ?? FF 35"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const ALLOC_SHARED: Recipe = Recipe {
		name: "AllocShared",
		versions: CHECKED,
		anchor: Anchor::Export(c"?AllocShared@@YAPAXK@Z"),
		hops: &[],
		extract: Extract::Entry
	};

	pub const FREE_SHARED: Recipe = Recipe {
		name: "FreeShared",
		versions: CHECKED,
		anchor: Anchor::Export(c"?FreeShared@@YA_NPAX@Z"),
		hops: &[],
		extract: Extract::Entry
	};

	pub const COPY_SHARED: Recipe = Recipe {
		name: "CopyShared",
		versions: CHECKED,
		anchor: Anchor::Export(c"?CopyShared@@YAPAXPAX@Z"),
		hops: &[],
		extract: Extract::Entry
	};

	// The loader of the type table, which allocates the table, the preorder,
	// the proc name maps and the appearance cache, one after the other. Memory
	// operands without a base register count, `lea`s of the sizes too.
	const TYPE_LOADER: &str = "69 F0 98 00 00 00 83 C4 04 A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? \
	                           ?? ?? ?? 8D 34 85 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D \
	                           34 C5 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 \
	                           00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? ?? ??";

	// Before 515.1641 a type entry is 0x90 bytes, multiplied with a `lea` and
	// a shift. The operands are numbered the same.
	const TYPE_LOADER_515: &str = "A3 ?? ?? ?? ?? 8D 34 C0 C1 E6 04 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? \
	                               ?? ?? 8D 34 85 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? \
	                               8D 34 C5 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 \
	                               85 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? ?? ??";
	const BEFORE_TYPEPATHS: VersionRange = VersionRange { min: 1637, max: 1640 };
	const WITH_TYPEPATHS: VersionRange = VersionRange { min: 1641, max: 1688 };

	pub const PREORDER: [Recipe; 2] = [
		Recipe {
			name: "type_preorder",
			versions: WITH_TYPEPATHS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
		},
		Recipe {
			name: "type_preorder",
			versions: BEFORE_TYPEPATHS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_515),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
		}
	];

	pub const APPEARANCE_CACHE: [Recipe; 2] = [
		Recipe {
			name: "type_appearance_cache",
			versions: WITH_TYPEPATHS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(10))
		},
		Recipe {
			name: "type_appearance_cache",
			versions: BEFORE_TYPEPATHS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_515),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(10))
		}
	];

	// The loader of the procs that return parameters' `in` lists: the count
	// is operand 1, the table 2 (after the `lea` of its size)
	const IN_LIST_LOADER: &str = "8D 34 85 00 00 00 00 83 C4 04 A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? \
	                              ?? ?? 39 35 ?? ?? ?? ?? 76 ?? 80 3D ?? ?? ?? ?? 00 57 74 ?? E8 ?? ?? ?? ?? 8B C8 EB ?? E8 ?? ?? ?? ?? 0F B7 C8 A1 \
	                              ?? ?? ?? ?? 83 C4 04 89 0C B0";

	pub const IN_LIST_COUNT: Recipe = Recipe {
		name: "in_list_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	pub const IN_LIST_PROCS: Recipe = Recipe {
		name: "in_list_procs",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	};

	// The loader of the mob types: the count is operand 0, the table 1
	// The world's proc name map: looking a proc up by name on the world
	// pushes it (the push's operand).
	pub const WORLD_NAME_MAP: Recipe = Recipe {
		name: "world_name_map",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"F6 45 08 02 0F 84 ?? ?? ?? ?? FF 75 20 FF 75 10 FF 75 0C 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10"
		),
		hops: &[],
		extract: Extract::Offset(20)
	};

	pub const MOB_COUNT: Recipe = Recipe {
		name: "mob_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const MOBS: Recipe = Recipe {
		name: "mobs",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	const MOB_LOADER: &str =
		"89 35 ?? ?? ?? ?? C1 E6 04 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 DB A3 ?? ?? ?? ?? 39 1D ?? ?? ?? ?? 76";

	// The loader of the prefab instances: the count is operand 0, the table 1
	const INSTANCE_LOADER: &str = "A3 ?? ?? ?? ?? 8D 34 40 C1 E6 02 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 DB A3 ?? ?? ?? ?? 39 \
	                               1D ?? ?? ?? ?? 76 ?? 33 F6 A1 ?? ?? ?? ?? 03 C6 57 50 E8";

	pub const INSTANCE_COUNT: Recipe = Recipe {
		name: "instance_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const INSTANCES: Recipe = Recipe {
		name: "instances",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// Builds before 1686 don't check for NONE
	pub const REGISTER_TYPEPATH: Recipe = Recipe {
		name: "register_typepath",
		versions: VersionRange { min: 1686, max: 1688 },
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 57 8B 7D 08 3B 3D ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 81 FF FF FF 00 00 0F 84 ?? ?? ?? ?? 56 69 F7 98 00 00 00 03 35 ?? ?? ?? ?? 0F \
			 84 ?? ?? ?? ?? 80 BE 8C 00 00 00 00 75"
		),
		hops: &[],
		extract: Extract::Entry
	};

	pub const REGISTER_TYPEPATH_OLD: Recipe = Recipe {
		name: "register_typepath",
		versions: VersionRange { min: 1641, max: 1685 },
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 57 8B 7D 08 3B 3D ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 56 69 F7 98 00 00 00 03 35 ?? ?? ?? ?? 0F 84 ?? ?? ?? ?? 80 BE 8C 00 00 00 00 \
			 75"
		),
		hops: &[],
		extract: Extract::Entry
	};
}

#[cfg(windows)]
fn resolve() -> Result<Functions, String> {
	use recipes::*;
	let find = |recipe| crate::find_recipe(recipe);
	let build = crate::byond_build()?;

	// BYOND's tables are allocated by the CRT DLL it links (ucrtbase), and
	// growing one has to go through the same heap. Ours is linked statically.
	let crt_realloc = unsafe {
		use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
		let module = GetModuleHandleA(c"ucrtbase.dll".as_ptr());
		if module.is_null() {
			return Err("Couldn't find ucrtbase.dll".to_owned());
		}
		let realloc = GetProcAddress(module, c"realloc".as_ptr());
		if realloc.is_null() {
			return Err("Couldn't find realloc in ucrtbase.dll".to_owned());
		}
		realloc
	};

	let type_count = find(&TYPE_COUNT)?;
	unsafe {
		Ok(Functions {
			procdef_add: std::mem::transmute(find(&PROCDEF_ADD)?),
			misc_intern: MiscIntern::Copy(std::mem::transmute(find(&MISC_INTERN_COPY)?)),
			alloc_shared: std::mem::transmute(find(&ALLOC_SHARED)?),
			free_shared: std::mem::transmute(find(&FREE_SHARED)?),
			copy_shared: std::mem::transmute(find(&COPY_SHARED)?),
			register_typepath: if build < TYPEPATH_BUILD {
				Ok(None)
			} else {
				find(&REGISTER_TYPEPATH)
					.or_else(|_| find(&REGISTER_TYPEPATH_OLD))
					.map(|address| Some(std::mem::transmute::<usize, extern "C" fn(u32)>(address)))
			},
			type_stride: type_stride(build),
			crt_realloc: std::mem::transmute(crt_realloc),
			type_count: type_count as *const u32,
			type_table: (type_count - 4) as *const *mut u8,
			name_maps: find(&NAME_MAPS)? as *const *mut ProcNameMap,
			preorder: crate::find_recipe_of(&PREORDER)? as *const *mut u32,
			appearance_cache: crate::find_recipe_of(&APPEARANCE_CACHE)? as *const *mut u32,
			in_list_count: find(&IN_LIST_COUNT)? as *const u32,
			in_list_procs: find(&IN_LIST_PROCS)? as *const *mut u32,
			instance_count: find(&INSTANCE_COUNT)? as *const u32,
			instances: find(&INSTANCES)? as *const *mut Instance,
			mob_count: find(&MOB_COUNT)? as *const u32,
			mobs: find(&MOBS)? as *const *mut MobType,
			// The code holds the map's address
			world_name_map: (find(&WORLD_NAME_MAP)? as *const *mut ProcNameMap).read_unaligned(),
			global_values: find(&GLOBAL_VALUES)? as *const *const crate::raw_types::values::Value,
			global_vars: find(&GLOBAL_VARS).map(|address| address as *mut u32)
		})
	}
}

// The same functions in libbyond.so, from 516.1688 (procdef_add 0x2ED8C0,
// misc_intern_copy 0x308840, the proc name map accessor 0x2EC7C0). Where
// Windows keeps a table's pointer before its count, Linux keeps it after.
#[cfg(unix)]
mod recipes {
	use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

	const CHECKED: VersionRange = VersionRange { min: 1644, max: 1688 };
	// exported by name on every build from 515.1637
	const EXPORTED: VersionRange = VersionRange { min: 1637, max: 1688 };

	pub const PROCDEF_ADD: Recipe = Recipe {
		name: "procdef_add",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"57 56 53 83 EC 10 A1 ?? ?? ?? ?? 8B 7C 24 20 85 C0 0F 84 ?? ?? ?? ?? 31 F6 31 DB EB ?? 8D 76 00 A1 ?? ?? ?? ?? 83 C3 01"
		),
		hops: &[],
		extract: Extract::Entry
	};

	pub const MISC_INTERN_COPY: Recipe = Recipe {
		name: "misc_intern_copy",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"57 56 53 8B 1D ?? ?? ?? ?? 8B 7C 24 10 8B 74 24 14 85 DB 74 ?? 8B 43 08 A3 ?? ?? ?? ?? 66 89 33 0F B7 F6"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// The type count is operand 0, the maps 1. The type table is the word
	// after the count.
	const TYPE_MAP_ACCESSOR: &str = "8B 54 24 04 31 C0 39 15 ?? ?? ?? ?? 72 08 A1 ?? ?? ?? ?? 8D 04 D0 C3";

	pub const TYPE_COUNT: Recipe = Recipe {
		name: "obj_type_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const NAME_MAPS: Recipe = Recipe {
		name: "proc_name_maps",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// The loader of the var tables, as auxtools' `variable_names` finds it.
	// global_values is operand 1.
	pub const GLOBAL_VALUES: Recipe = Recipe {
		name: "global_values",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"89 1D ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C C1 E3 02 A3 ?? ?? ?? ?? 53 E8 ?? ?? ?? ?? 83 \
			 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// Loader store of the sorted, interned global (var ID, flags) list.
	// The destination address is operand 0.
	pub const GLOBAL_VARS: Recipe = Recipe {
		name: "global_vars",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"66 D1 EA 0F B7 D2 E8 ?? ?? ?? ?? 59 58 0F B7 07 50 53 E8 ?? ?? ?? ?? 89 C7 89 1C 24 A3 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const ALLOC_SHARED: Recipe = Recipe {
		name: "AllocShared",
		versions: EXPORTED,
		anchor: Anchor::Export(c"_Z11AllocSharedm"),
		hops: &[],
		extract: Extract::Entry
	};

	pub const FREE_SHARED: Recipe = Recipe {
		name: "FreeShared",
		versions: EXPORTED,
		anchor: Anchor::Export(c"_Z10FreeSharedPv"),
		hops: &[],
		extract: Extract::Entry
	};

	pub const COPY_SHARED: Recipe = Recipe {
		name: "CopyShared",
		versions: EXPORTED,
		anchor: Anchor::Export(c"_Z10CopySharedPv"),
		hops: &[],
		extract: Extract::Entry
	};

	// The loader of the type table, which allocates the table, the preorder,
	// the proc name maps and the appearance cache, one after the other. Memory
	// operands without a base register count, `lea`s of the sizes too.
	const TYPE_LOADER: &str = "69 FB 94 00 00 00 83 EC 0C 89 1D ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C \
	                           8D 3C 9D 00 00 00 00 A3 ?? ?? ?? ?? 57 89 BD ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10 85 FF 0F 95 85 ?? ?? ?? ?? 0F B6 8D \
	                           ?? ?? ?? ?? 85 C0 75 08 84 C9 0F 85 ?? ?? ?? ?? 83 EC 0C 8D 3C DD 00 00 00 00 A3 ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 \
	                           10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C FF B5 ?? ?? ?? ?? A3 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10 85 C0 75 ?? \
	                           80 BD ?? ?? ?? ?? 00 0F 85 ?? ?? ?? ?? A3 ?? ?? ?? ??";

	pub const PREORDER: [Recipe; 2] = [
		Recipe {
			name: "type_preorder",
			versions: CHECKED,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
		},
		Recipe {
			name: "type_preorder",
			versions: EARLY,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
		}
	];

	pub const APPEARANCE_CACHE: [Recipe; 2] = [
		Recipe {
			name: "type_appearance_cache",
			versions: CHECKED,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(6))
		},
		Recipe {
			name: "type_appearance_cache",
			versions: EARLY,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
			hops: &[],
			extract: Extract::AbsMem(OperandSelect::NthAbsMem(8))
		}
	];

	// The loader of the procs that return parameters' `in` lists: the count
	// is operand 1, the table 2 (after the `lea` of its size)
	const IN_LIST_LOADER: &str = "83 EC 0C 8D 1C 85 00 00 00 00 A3 ?? ?? ?? ?? 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? \
	                              ?? ?? ?? 31 DB EB ?? 83 EC 0C 56 E8 ?? ?? ?? ?? 83 C4 10 8B 15 ?? ?? ?? ?? 89 04 9A";

	pub const IN_LIST_COUNT: Recipe = Recipe {
		name: "in_list_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	pub const IN_LIST_PROCS: Recipe = Recipe {
		name: "in_list_procs",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	};

	// The loader of the mob types: the count is operand 0, the table 1
	// The world's proc name map: looking a proc up by name on the world
	// stores it as an argument (the store's operand) and jumps to the lookup.
	pub const WORLD_NAME_MAP: Recipe = Recipe {
		name: "world_name_map",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"0F B6 45 ?? 89 7D 0C C7 45 08 ?? ?? ?? ?? 89 45 14 89 F0 0F B6 F0 89 75 10 8D 65 F4 5B 5E 5F 5D E9"
		),
		hops: &[],
		extract: Extract::Offset(10)
	};

	pub const MOB_COUNT: Recipe = Recipe {
		name: "mob_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const MOBS: Recipe = Recipe {
		name: "mobs",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	const MOB_LOADER: &str = "89 C7 83 C4 10 89 FB 83 EC 0C 89 3D ?? ?? ?? ?? C1 E3 04 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? \
	                          ?? A3 ?? ?? ?? ?? 31 DB 85 FF 74 ?? 89 D8 83 EC 08 C1 E0 04 03 05";

	// The loader of the prefab instances: the count is operand 0, the table 1
	const INSTANCE_LOADER: &str = "6B D8 0C 83 EC 0C A3 ?? ?? ?? ?? 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? \
	                               31 DB EB ?? 6B C3 0C 03 05 ?? ?? ?? ?? 57 57 56 50 E8";

	pub const INSTANCE_COUNT: Recipe = Recipe {
		name: "instance_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const INSTANCES: Recipe = Recipe {
		name: "instances",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// Builds before 1686 don't check for NONE
	pub const REGISTER_TYPEPATH: Recipe = Recipe {
		name: "register_typepath",
		versions: VersionRange { min: 1686, max: 1688 },
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"53 83 EC 18 8B 44 24 20 3B 05 ?? ?? ?? ?? 73 ?? 3D FF FF 00 00 74 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 \
			 ?? 8D 44 24 08 83 EC 04 6A 06"
		),
		hops: &[],
		extract: Extract::Entry
	};

	pub const REGISTER_TYPEPATH_OLD: Recipe = Recipe {
		name: "register_typepath",
		versions: VersionRange { min: 1644, max: 1685 },
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"53 83 EC 18 8B 44 24 20 3B 05 ?? ?? ?? ?? 73 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 ?? 8D 44 24 08 83 EC \
			 04 6A 06"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// Linux 515 before 515.1644, which GCC compiled with frame pointers. The
	// loaders' operands are numbered as above.
	const EARLY: VersionRange = VersionRange { min: 1637, max: 1643 };
	const TYPE_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 69 F0 ?? 00 00 00 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? \
	                                 ?? C1 E6 02 89 34 24 E8 ?? ?? ?? ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 03 89 34 24 \
	                                 E8 ?? ?? ?? ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 02 89 34 24 E8 ?? ?? ?? ?? 85 C0 \
	                                 0F 84 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? A3 ?? ?? ?? ??";
	const VAR_LOADER_EARLY: &str =
		"A3 ?? ?? ?? ?? 8D 34 C5 00 00 00 00 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 02";
	const IN_LIST_LOADER_EARLY: &str =
		"A3 ?? ?? ?? ?? C1 E0 02 31 F6 89 04 24 E8 ?? ?? ?? ?? A3 ?? ?? ?? ?? EB ?? 89 1C 24 E8 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 89 04 B2";
	const MOB_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 89 C6 C1 E6 04 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 3D ?? ?? ?? ?? A3 ?? ?? ?? ??";
	const INSTANCE_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 6B C0 0C 31 FF 31 F6 89 04 24 E8 ?? ?? ?? ?? A3 ?? ?? ?? ?? EB ?? A1 ?? ?? ?? ?? 83 C6 01";

	pub const PROCDEF_ADD_EARLY: Recipe = Recipe {
		name: "procdef_add",
		versions: EARLY,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C0 89 E5 57 56 53 83 EC 2C 8B 35 ?? ?? ?? ?? 8B 7D 08 85 F6 74 ?? 31 F6 31 DB"
		),
		hops: &[],
		extract: Extract::Entry
	};
	// What a misc entry's words are copied into is inlined into every caller:
	// this is what they hand the entry to
	pub const MISC_INTERN: Recipe = Recipe {
		name: "misc_intern",
		versions: EARLY,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 B8 FF FF 00 00 89 E5 83 EC 68 89 5D F4 8B 5D 08 89 75 F8 89 7D FC 85 DB 74"
		),
		hops: &[],
		extract: Extract::Entry
	};
	pub const TYPE_COUNT_EARLY: Recipe = Recipe {
		name: "obj_type_count",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};
	pub const NAME_MAPS_EARLY: Recipe = Recipe {
		name: "proc_name_maps",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(6))
	};
	pub const GLOBAL_VALUES_EARLY: Recipe = Recipe {
		name: "global_values",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, VAR_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(3))
	};
	pub const GLOBAL_VARS_EARLY: Recipe = Recipe {
		name: "global_vars",
		versions: EARLY,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"66 D1 E8 0F B7 C0 89 44 24 04 E8 ?? ?? ?? ?? 0F B7 06 89 3C 24 89 44 24 04 E8 ?? ?? ?? ?? 89 3C 24 A3 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};
	pub const IN_LIST_COUNT_EARLY: Recipe = Recipe {
		name: "in_list_count",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};
	pub const IN_LIST_PROCS_EARLY: Recipe = Recipe {
		name: "in_list_procs",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};
	pub const MOB_COUNT_EARLY: Recipe = Recipe {
		name: "mob_count",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};
	pub const MOBS_EARLY: Recipe = Recipe {
		name: "mobs",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	};
	pub const INSTANCE_COUNT_EARLY: Recipe = Recipe {
		name: "instance_count",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};
	pub const INSTANCES_EARLY: Recipe = Recipe {
		name: "instances",
		versions: EARLY,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};
	// the world's proc name map, which the lookup of a proc on the world hands
	// to the map search
	pub const WORLD_NAME_MAP_EARLY: Recipe = Recipe {
		name: "world_name_map",
		versions: EARLY,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"89 7C 24 0C 89 5C 24 08 C7 04 24 ?? ?? ?? ?? 89 74 24 04 E8 ?? ?? ?? ?? E9"
		),
		hops: &[],
		extract: Extract::Offset(11)
	};
	// from 515.1641, when types started keeping their typepath values
	pub const REGISTER_TYPEPATH_EARLY: Recipe = Recipe {
		name: "register_typepath",
		versions: VersionRange { min: 1641, max: 1643 },
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 53 83 EC 24 8B 45 08 3B 05 ?? ?? ?? ?? 73 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 ?? C7 44 24 08 \
			 06 00 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	};
}

// libbyond.so allocates with the C library, as we do
#[cfg(unix)]
fn resolve() -> Result<Functions, String> {
	use recipes::*;
	extern "C" {
		fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
	}
	let find = |recipe| crate::find_recipe(recipe);
	let build = crate::byond_build()?;

	let type_count = crate::find_recipe_of(&[TYPE_COUNT, TYPE_COUNT_EARLY])?;
	unsafe {
		Ok(Functions {
			procdef_add: std::mem::transmute(crate::find_recipe_of(&[PROCDEF_ADD, PROCDEF_ADD_EARLY])?),
			misc_intern: if build < LINUX_SWAPPED_BUILD {
				MiscIntern::Take(std::mem::transmute(find(&MISC_INTERN)?))
			} else {
				MiscIntern::Copy(std::mem::transmute(find(&MISC_INTERN_COPY)?))
			},
			alloc_shared: std::mem::transmute(find(&ALLOC_SHARED)?),
			free_shared: std::mem::transmute(find(&FREE_SHARED)?),
			copy_shared: std::mem::transmute(find(&COPY_SHARED)?),
			register_typepath: if build < TYPEPATH_BUILD {
				Ok(None)
			} else {
				crate::find_recipe_of(&[REGISTER_TYPEPATH, REGISTER_TYPEPATH_OLD, REGISTER_TYPEPATH_EARLY])
					.map(|address| Some(std::mem::transmute::<usize, extern "C" fn(u32)>(address)))
			},
			type_stride: type_stride(build),
			crt_realloc: std::mem::transmute::<unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void, _>(realloc),
			type_count: type_count as *const u32,
			// the table's pointer is the word after its count, or before it (as
			// on Windows) on builds before 515.1644
			type_table: if build < LINUX_SWAPPED_BUILD { type_count - 4 } else { type_count + 4 } as *const *mut u8,
			name_maps: crate::find_recipe_of(&[NAME_MAPS, NAME_MAPS_EARLY])? as *const *mut ProcNameMap,
			preorder: crate::find_recipe_of(&PREORDER)? as *const *mut u32,
			appearance_cache: crate::find_recipe_of(&APPEARANCE_CACHE)? as *const *mut u32,
			in_list_count: crate::find_recipe_of(&[IN_LIST_COUNT, IN_LIST_COUNT_EARLY])? as *const u32,
			in_list_procs: crate::find_recipe_of(&[IN_LIST_PROCS, IN_LIST_PROCS_EARLY])? as *const *mut u32,
			instance_count: crate::find_recipe_of(&[INSTANCE_COUNT, INSTANCE_COUNT_EARLY])? as *const u32,
			instances: crate::find_recipe_of(&[INSTANCES, INSTANCES_EARLY])? as *const *mut Instance,
			mob_count: crate::find_recipe_of(&[MOB_COUNT, MOB_COUNT_EARLY])? as *const u32,
			mobs: crate::find_recipe_of(&[MOBS, MOBS_EARLY])? as *const *mut MobType,
			// The code holds the map's address
			world_name_map: (crate::find_recipe_of(&[WORLD_NAME_MAP, WORLD_NAME_MAP_EARLY])? as *const *mut ProcNameMap).read_unaligned(),
			global_values: crate::find_recipe_of(&[GLOBAL_VALUES, GLOBAL_VALUES_EARLY])? as *const *const crate::raw_types::values::Value,
			global_vars: crate::find_recipe_of(&[GLOBAL_VARS, GLOBAL_VARS_EARLY]).map(|address| address as *mut u32)
		})
	}
}

fn functions() -> Result<&'static Functions, String> {
	unsafe {
		let functions = &mut *std::ptr::addr_of_mut!(FUNCTIONS);
		functions.get_or_insert_with(resolve).as_ref().map_err(Clone::clone)
	}
}

/// Check hot reload support excluding type registration
/// (see [`types_available`]).
pub fn available() -> Result<(), String> {
	functions().map(|_| ())
}

/// Whether this build of BYOND has everything [`add_type`] needs.
pub fn types_available() -> Result<(), String> {
	functions()?.register_typepath.as_ref().map(|_| ()).map_err(Clone::clone)
}

/// A proc's table entry.
pub fn proc_entry(id: ProcId) -> Option<ProcEntryWords> {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) != 1 || entry.is_null() {
			return None;
		}
		Some(*(entry as *const ProcEntryWords))
	}
}

/// Overwrites one word of a proc's table entry.
pub fn set_proc_entry_word(id: ProcId, word: usize, value: u32) {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) == 1 && !entry.is_null() {
			(*(entry as *mut ProcEntryWords)).0[word] = value;
		}
	}
}

/// Replaces a proc's whole table entry.
pub fn set_proc_entry(id: ProcId, words: &ProcEntryWords) {
	for (word, &value) in words.0.iter().enumerate() {
		set_proc_entry_word(id, word, value);
	}
}

/// Appends a proc to the proc table and returns its id. BYOND takes
/// references to its strings; the misc entries need theirs taken already.
pub fn add_proc(entry: &ProcEntryWords) -> Result<ProcId, String> {
	let id = (functions()?.procdef_add)(entry);
	if id == NONE {
		return Err("BYOND couldn't add the proc".to_owned());
	}
	Ok(ProcId(id))
}

/// Append a proc with a fresh ID, even if an identical entry exists.
/// Unlike `add_proc`, this supports re-adding removed procs and adding
/// identical definitions.
pub fn append_proc(entry: &ProcEntryWords) -> Result<ProcId, String> {
	// Temporarily hide matching paths to prevent BYOND from reusing an entry.
	let path = entry.0[ProcEntryWords::PATH];
	let hidden: Vec<ProcId> = (0..proc_count())
		.map(ProcId)
		.filter(|&id| proc_entry(id).is_some_and(|e| e.0[ProcEntryWords::PATH] == path))
		.collect();
	for &id in &hidden {
		set_proc_entry_word(id, ProcEntryWords::PATH, !path);
	}
	let added = add_proc(entry);
	for &id in &hidden {
		set_proc_entry_word(id, ProcEntryWords::PATH, path);
	}
	added
}

/// How many procs there are. The next one added gets this id.
pub fn proc_count() -> u32 {
	let mut id = 0;
	while proc_entry(ProcId(id)).is_some() {
		id += 1;
	}
	id
}

/// A misc entry's words.
pub fn misc_words(id: u32) -> Option<Vec<u32>> {
	if id == NONE {
		return Some(vec![]);
	}
	let mut entry: *mut c_void = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_misc_by_id(&mut entry, misc::MiscId::from_raw(id)) != 1 || entry.is_null() {
			return None;
		}
		// count (u16), reference count, data
		let count = *(entry as *const u16) as usize;
		let data = *((entry as *const u8).add(8) as *const *const u32);
		if count == 0 || data.is_null() {
			return Some(vec![]);
		}
		Some(std::slice::from_raw_parts(data, count).to_vec())
	}
}

/// A misc entry holding `words`, with a reference taken so it is never freed.
/// BYOND shares entries with the same words.
pub fn intern_misc(words: &[u32]) -> Result<u32, String> {
	if words.len() > u16::MAX as usize {
		return Err(format!("{} words is too many for a misc entry", words.len()));
	}
	let functions = functions()?;
	let id = match functions.misc_intern {
		MiscIntern::Copy(copy) => copy(words.as_ptr(), words.len() as u32),
		MiscIntern::Take(take) => unsafe {
			// allocated as BYOND allocates them, so it can free them
			let entry = (functions.crt_realloc)(std::ptr::null_mut(), 12) as *mut u8;
			if entry.is_null() {
				return Err("out of memory making a misc entry".to_owned());
			}
			let mut data = std::ptr::null_mut::<u32>();
			if !words.is_empty() {
				data = (functions.crt_realloc)(std::ptr::null_mut(), words.len() * 4) as *mut u32;
				if data.is_null() {
					return Err("out of memory making a misc entry".to_owned());
				}
				std::ptr::copy_nonoverlapping(words.as_ptr(), data, words.len());
			}
			*(entry as *mut u16) = words.len() as u16;
			*(entry.add(2) as *mut u16) = 0;
			*(entry.add(4) as *mut u32) = 0;
			*(entry.add(8) as *mut *mut u32) = data;
			take(entry)
		}
	};
	if id == NONE {
		return Err("BYOND couldn't make the misc entry".to_owned());
	}
	misc::add_ref(&misc::MiscId::from_raw(id));
	Ok(id)
}

pub fn type_count() -> Result<u32, String> {
	Ok(unsafe { *functions()?.type_count })
}

fn type_entry(type_id: u32) -> Result<*mut u8, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.type_table).add(type_id as usize * functions.type_stride))
	}
}

/// A word of a type's table entry (see `type_fields`).
pub fn type_field(type_id: u32, offset: usize) -> Result<u32, String> {
	Ok(unsafe { *(type_entry(type_id)?.add(offset) as *const u32) })
}

pub fn set_type_field(type_id: u32, offset: usize, value: u32) -> Result<(), String> {
	unsafe {
		*(type_entry(type_id)?.add(offset) as *mut u32) = value;
	}
	Ok(())
}

/// A mob type's entry in the mob table, which a mob typepath value's data
/// is an index of.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobType {
	pub type_id: u32,
	/// String id
	pub key: u32,
	pub sight: u32,
	pub see_in_dark: u8,
	pub see_invisible: u8,
	pub padding: u16
}

/// A prefab instance: a type, and the proc that sets the vars it overrides
/// on each new instance (named `TYPE/proc/(map_init)`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
	/// The type's typepath value: its tag and data
	pub tag: u8,
	pub data: u32,
	/// NONE for a type placed on the map as it is
	pub init: u32
}

/// The typepath value tag of a mob type
const MOB_TYPEPATH: u8 = 0x08;

/// Adds a mob type to the mob table, and returns its index.
fn add_mob_type(mob: MobType) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.mob_count;
		let table = functions.mobs as *mut *mut MobType;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * std::mem::size_of::<MobType>()) as *mut MobType;
		if grown.is_null() {
			return Err("out of memory growing the mob table".to_owned());
		}
		*grown.add(count as usize) = mob;
		*table = grown;
		*(functions.mob_count as *mut u32) = count + 1;
		Ok(count)
	}
}

/// Sets a mob type's `key`, `sight`, `see_in_dark` and `see_invisible`.
pub fn set_mob_type(type_id: u32, set: impl FnOnce(&mut MobType)) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		let mobs = std::slice::from_raw_parts_mut(*functions.mobs, *functions.mob_count as usize);
		let mob = mobs.iter_mut().find(|m| m.type_id == type_id).ok_or("it isn't a mob type")?;
		set(mob);
	}
	Ok(())
}

/// The world's prefab instances, by id.
pub fn instances() -> Result<Vec<Instance>, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.instance_count as usize;
		let table = *functions.instances;
		if count == 0 || table.is_null() {
			return Ok(vec![]);
		}
		Ok(std::slice::from_raw_parts(table, count).to_vec())
	}
}

/// Adds a prefab instance, and returns its id.
pub fn add_instance(instance: Instance) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.instance_count;
		let table = functions.instances as *mut *mut Instance;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * std::mem::size_of::<Instance>()) as *mut Instance;
		if grown.is_null() {
			return Err("out of memory growing the prefab instances".to_owned());
		}
		*grown.add(count as usize) = instance;
		*table = grown;
		*(functions.instance_count as *mut u32) = count + 1;
		Ok(count)
	}
}

/// How many procs that return `in` lists the world has.
pub fn in_list_count() -> Result<u32, String> {
	Ok(unsafe { *functions()?.in_list_count })
}

/// The procs that return parameters' `in` lists, by index.
pub fn in_list_procs() -> Result<Vec<u32>, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.in_list_count as usize;
		let table = *functions.in_list_procs;
		if table.is_null() {
			return Ok(vec![]);
		}
		Ok(std::slice::from_raw_parts(table, count).to_vec())
	}
}

/// Adds a proc that returns a parameter's `in` list, and returns the index
/// the parameter's word (`(index << 8) | 0x40`) gives it.
pub fn add_in_list(proc: ProcId) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.in_list_count;
		let table = functions.in_list_procs as *mut *mut u32;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * 4) as *mut u32;
		if grown.is_null() {
			return Err("out of memory growing the `in` list procs".to_owned());
		}
		*grown.add(count as usize) = proc.0;
		*table = grown;
		*(functions.in_list_count as *mut u32) = count + 1;
		Ok(count)
	}
}

/// Rebuild a type's default appearance for future instances, including
/// its verbs. Existing instances retain their old appearance.
pub fn forget_type_appearance(type_id: u32) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		*(*functions.appearance_cache).add(type_id as usize) = NONE;
	}
	Ok(())
}

/// A type's proc name map.
fn name_map(type_id: u32) -> Result<*mut ProcNameMap, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.name_maps).add(type_id as usize))
	}
}

/// A map's buckets. A map with none can have no buffer.
unsafe fn buckets<'a>(map: *mut ProcNameMap) -> &'a [ProcNameBucket] {
	if (*map).buckets.is_null() {
		&[]
	} else {
		std::slice::from_raw_parts((*map).buckets, (*map).count as usize)
	}
}

/// The procs a name (its string id) finds on a type, most derived first.
pub fn named_procs(type_id: u32, key: u32) -> Result<Vec<u32>, String> {
	named_procs_in(name_map(type_id)?, key)
}

/// The procs a name finds on the world, most derived first.
pub fn world_named_procs(key: u32) -> Result<Vec<u32>, String> {
	named_procs_in(functions()?.world_name_map, key)
}

fn named_procs_in(map: *mut ProcNameMap, key: u32) -> Result<Vec<u32>, String> {
	unsafe {
		let buckets = buckets(map);
		Ok(match buckets.binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(i) if !buckets[i].ids.is_null() => std::slice::from_raw_parts(buckets[i].ids, buckets[i].count as usize).to_vec(),
			_ => vec![]
		})
	}
}

/// Replace a type's proc name entry, inserting it if absent.
/// Release the old shared buffer and transfer ownership of the new one.
/// A map BYOND hasn't built yet is left alone: it builds it from the type's
/// procs, and links their `..()` chains, as the world loads.
pub fn set_named_procs(type_id: u32, key: u32, procs: &[u32]) -> Result<(), String> {
	let map = name_map(type_id)?;
	if unsafe { (*map).built } == 0 {
		return Ok(());
	}
	set_named_procs_in(map, key, procs)
}

/// Whether BYOND has built the world proc name map. Construction occurs
/// after global initialization and before world/New; earlier edits are lost.
pub fn world_names_built() -> Result<bool, String> {
	unsafe { Ok((*functions()?.world_name_map).built != 0) }
}

/// Gives a name these procs on the world, as [`set_named_procs`] does on a
/// type.
pub fn set_world_named_procs(key: u32, procs: &[u32]) -> Result<(), String> {
	set_named_procs_in(functions()?.world_name_map, key, procs)
}

fn set_named_procs_in(map: *mut ProcNameMap, key: u32, procs: &[u32]) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		let count = buckets(map).len();
		let at = match buckets(map).binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(at) => at,
			Err(at) => {
				let size = std::mem::size_of::<ProcNameBucket>();
				let buckets = (functions.crt_realloc)((*map).buckets as *mut c_void, (count + 1) * size) as *mut ProcNameBucket;
				if buckets.is_null() {
					return Err("out of memory growing a proc name map".to_owned());
				}
				std::ptr::copy(buckets.add(at), buckets.add(at + 1), count - at);
				buckets.add(at).write(ProcNameBucket {
					key,
					ids: std::ptr::null_mut(),
					count: 0,
					no_link: 0
				});
				(*map).buckets = buckets;
				(*map).count += 1;
				at
			}
		};

		let ids = (functions.alloc_shared)(procs.len().max(1) * 4) as *mut u32;
		if ids.is_null() {
			return Err("out of memory for a proc name map entry".to_owned());
		}
		std::ptr::copy_nonoverlapping(procs.as_ptr(), ids, procs.len());

		let bucket = &mut *(*map).buckets.add(at);
		if !bucket.ids.is_null() {
			(functions.free_shared)(bucket.ids as *mut c_void);
		}
		bucket.ids = ids;
		bucket.count = procs.len() as u16;
	}
	Ok(())
}

/// Replace a global slot, retaining the new value and releasing the old one.
pub fn set_global_value(var_id: u32, value: crate::Value) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		let slot = (*functions.global_values as *mut crate::raw_types::values::Value).add(var_id as usize);
		drop(crate::Value::from_raw_owned(*slot));
		*slot = value.raw;
		std::mem::forget(value);
	}
	Ok(())
}

/// Replace a global slot with tag/data words, transferring the caller's
/// reference and releasing the old value.
pub fn set_global_words(var_id: u32, tag: u8, data: u32) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		let slot = (*functions.global_values as *mut [u32; 2]).add(var_id as usize);
		drop(crate::Value::from_raw_owned(*(slot as *const crate::raw_types::values::Value)));
		slot.write([tag as u32, data]);
	}
	Ok(())
}

/// The value of a var id's global slot (see `Functions::global_values`).
pub fn global_value(var_id: u32) -> Result<crate::raw_types::values::Value, String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		Ok(*(*functions.global_values).add(var_id as usize))
	}
}

/// Add globals to `global.vars`, sorted by name string ID.
/// Leave existing (var ID, flags) pairs unchanged.
pub fn add_global_vars(vars: &[(u32, u32)]) -> Result<(), String> {
	let global_vars = functions()?.global_vars.clone()?;
	let current = unsafe { *global_vars };
	let mut words = misc_words(current).ok_or_else(|| format!("global-var misc entry {} is missing", current))?;
	let name = |var_id: u32| variable_name(var_id).ok_or_else(|| format!("var ID {} does not exist", var_id));
	for &(var_id, flags) in vars {
		if words.chunks(2).any(|pair| pair[0] == var_id) {
			continue;
		}
		let key = name(var_id)?;
		let mut at = 0;
		for pair in words.chunks(2) {
			if name(pair[0])? > key {
				break;
			}
			at += 2;
		}
		words.splice(at..at, [var_id, flags]);
	}
	let id = intern_misc(&words)?;
	unsafe {
		*global_vars = id;
	}
	Ok(())
}

/// Var IDs listed by `global.vars`.
pub fn global_var_ids() -> Result<Vec<u32>, String> {
	let current = unsafe { *functions()?.global_vars.clone()? };
	let words = misc_words(current).ok_or_else(|| format!("global-var misc entry {} is missing", current))?;
	Ok(words.chunks(2).map(|pair| pair[0]).collect())
}

/// How many var ids there are.
pub fn variable_count() -> u32 {
	unsafe { *crate::raw_types::funcs::VARIABLE_NAME_COUNT }
}

/// The string id of a var id's name.
pub fn variable_name(var_id: u32) -> Option<u32> {
	unsafe { (var_id < variable_count()).then(|| (*(*crate::raw_types::funcs::VARIABLE_NAME_ENTRIES).add(var_id as usize)).0) }
}

/// Append a variable ID with name string ID `name` and initial tag/data.
/// Transfer the value reference to the slot. Existing IDs remain stable.
pub fn add_variable(name: u32, tag: u8, data: u32) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let entries = crate::raw_types::funcs::VARIABLE_NAME_ENTRIES as *mut *const crate::raw_types::strings::StringId;
		let count_ptr = crate::raw_types::funcs::VARIABLE_NAME_COUNT as *mut u32;
		let count = *count_ptr as usize;
		if count as u32 >= NONE {
			return Err("BYOND variable ID limit reached".to_owned());
		}

		let names = (functions.crt_realloc)(*entries as *mut c_void, (count + 1) * 4) as *mut u32;
		if names.is_null() {
			return Err("out of memory growing the var names".to_owned());
		}
		*entries = names as *const crate::raw_types::strings::StringId;

		let slots = functions.global_values as *mut *mut [u32; 2];
		let values = (functions.crt_realloc)(*slots as *mut c_void, (count + 1) * 8) as *mut [u32; 2];
		if values.is_null() {
			return Err("out of memory growing the var values".to_owned());
		}
		*slots = values;

		names.add(count).write(name);
		values.add(count).write([tag as u32, data]);
		*count_ptr += 1;
		Ok(count as u32)
	}
}

/// Append a type inheriting from `parent`, named by string ID `path`.
/// Insert after the parent's subtree and register it for text2path.
///
/// Share inherited variable lists, init proc and proc name buffers. Own proc,
/// verb and override lists start empty; record fields inherit until changed.
pub fn add_type(parent: u32, path: u32) -> Result<u32, String> {
	use type_fields::*;
	let functions = functions()?;
	let register_typepath = functions.register_typepath.clone()?;
	unsafe {
		let count = *functions.type_count as usize;
		if parent as usize >= count {
			return Err(format!("there's no type {}", parent));
		}
		if count + 1 >= NONE as usize {
			return Err("BYOND type ID limit reached".to_owned());
		}

		// The tables BYOND sizes by the type count, one entry longer
		let grow = |table: *const *mut u8, size: usize| -> Result<*mut u8, String> {
			let grown = (functions.crt_realloc)(*table as *mut c_void, size) as *mut u8;
			if grown.is_null() {
				return Err("out of memory growing the type tables".to_owned());
			}
			*(table as *mut *mut u8) = grown;
			Ok(grown)
		};
		let stride = functions.type_stride;
		let table = grow(functions.type_table, (count + 1) * stride)?;
		let preorder = grow(functions.preorder as *const *mut u8, (count + 1) * 4)? as *mut u32;
		let maps = grow(functions.name_maps as *const *mut u8, (count + 1) * 8)? as *mut ProcNameMap;
		let cache = grow(functions.appearance_cache as *const *mut u8, (count + 1) * 4)? as *mut u32;

		let entry = |id: usize| table.add(id * stride);
		let word = |id: usize, offset: usize| entry(id).add(offset) as *mut u32;

		std::ptr::copy_nonoverlapping(entry(parent as usize), entry(count), stride);
		*word(count, PATH) = path;
		*word(count, PARENT) = parent;
		for offset in [VERBS, PROCS, OWN_OVERRIDES] {
			*word(count, offset) = NONE;
		}
		for offset in [DECLARED_VARS, BUILTIN_VALUES, VAR_TABLE, EXTRA] {
			let id = *word(count, offset);
			if id != NONE {
				misc::add_ref(&misc::MiscId::from_raw(id));
			}
		}
		// What BYOND frees with the type is the type's own
		for (offset, size) in [(TRANSFORM, 6 * 4), (COLOR_MATRIX, 20 * 4)] {
			let from = *word(count, offset) as *const u8;
			if !from.is_null() {
				let copy = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut u8;
				if copy.is_null() {
					return Err("out of memory copying a type".to_owned());
				}
				std::ptr::copy_nonoverlapping(from, copy, size);
				*word(count, offset) = copy as u32;
			}
		}
		*word(count, INIT_SEGMENTS) = 0;
		if register_typepath.is_some() {
			*word(count, TYPEPATH) = 0;
			*word(count, TYPEPATH + 4) = 0;
		}

		// The parent's proc name map, sharing its id buffers as BYOND does
		let from = &*maps.add(parent as usize);
		let mut buckets = std::ptr::null_mut();
		if from.count > 0 && !from.buckets.is_null() {
			let size = from.count as usize * std::mem::size_of::<ProcNameBucket>();
			buckets = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut ProcNameBucket;
			if buckets.is_null() {
				return Err("out of memory copying a proc name map".to_owned());
			}
			std::ptr::copy_nonoverlapping(from.buckets, buckets, from.count as usize);
			for i in 0..from.count as usize {
				let bucket = &mut *buckets.add(i);
				if !bucket.ids.is_null() {
					bucket.ids = (functions.copy_shared)(bucket.ids as *mut c_void) as *mut u32;
				}
			}
		}
		maps.add(count).write(ProcNameMap {
			buckets,
			count: from.count,
			built: from.built
		});
		*cache.add(count) = NONE;

		// Last in its parent's subtree: what comes after moves along one, and
		// the subtrees it's in grow by one
		let at = *word(parent as usize, PREORDER_END) as usize;
		std::ptr::copy(preorder.add(at), preorder.add(at + 1), count - at);
		*preorder.add(at) = count as u32;
		for id in 0..count {
			if *word(id, PREORDER) as usize >= at {
				*word(id, PREORDER) += 1;
				*word(id, PREORDER_END) += 1;
			}
		}
		let mut ancestor = parent;
		while (ancestor as usize) < count {
			*word(ancestor as usize, PREORDER_END) += 1;
			ancestor = *word(ancestor as usize, PARENT);
		}
		*word(count, PREORDER) = at as u32;
		*word(count, PREORDER_END) = at as u32 + 1;

		*(functions.type_count as *mut u32) += 1;

		// For mob types, allocate a mob table entry initialized from the
		// parent. Its index is the mob typepath value. Older builds without
		// stored typepath values resolve it by type ID.
		let mobs = std::slice::from_raw_parts(*functions.mobs, *functions.mob_count as usize);
		let mob = match register_typepath {
			Some(register) => {
				if *(word(parent as usize, TYPEPATH) as *const u8) == 0 {
					register(parent);
				}
				*(word(parent as usize, TYPEPATH) as *const u8) == MOB_TYPEPATH
			}
			None => mobs.iter().any(|m| m.type_id == parent)
		};
		if mob {
			let from = mobs
				.iter()
				.find(|m| m.type_id == parent)
				.copied()
				.ok_or("parent mob type is missing from the mob table")?;
			add_mob_type(MobType {
				type_id: count as u32,
				..from
			})?;
		}
		if let Some(register) = register_typepath {
			register(count as u32);
		}
		Ok(count as u32)
	}
}

/// Sets 16 bits of a type's table entry.
pub fn set_type_half(type_id: u32, offset: usize, value: u16) -> Result<(), String> {
	unsafe {
		*(type_entry(type_id)?.add(offset) as *mut u16) = value;
	}
	Ok(())
}

/// Sets a byte of a type's table entry.
pub fn set_type_byte(type_id: u32, offset: usize, value: u8) -> Result<(), String> {
	unsafe {
		*type_entry(type_id)?.add(offset) = value;
	}
	Ok(())
}

/// All proc-name map buckets as (key, procs, no_link), sorted by key.
pub fn name_map_entries(type_id: u32) -> Result<Vec<(u32, Vec<u32>, u8)>, String> {
	unsafe {
		Ok(buckets(name_map(type_id)?)
			.iter()
			.map(|bucket| {
				let ids = if bucket.ids.is_null() {
					vec![]
				} else {
					std::slice::from_raw_parts(bucket.ids, bucket.count as usize).to_vec()
				};
				(bucket.key, ids, bucket.no_link)
			})
			.collect())
	}
}

/// Preallocated proc-name map. [`NameMapPlan::write`] cannot fail;
/// dropping an unwritten plan releases its buffers.
pub struct NameMapPlan {
	map: *mut ProcNameMap,
	buckets: *mut ProcNameBucket,
	count: u16,
	written: bool
}

/// Allocate a proc-name map from `entries` in any order, omitting empty
/// entries. Accepts the output of [`name_map_entries`].
pub fn plan_name_map(type_id: u32, entries: &[(u32, Vec<u32>, u8)]) -> Result<NameMapPlan, String> {
	let functions = functions()?;
	let map = name_map(type_id)?;
	let mut entries: Vec<&(u32, Vec<u32>, u8)> = entries.iter().filter(|(_, ids, _)| !ids.is_empty()).collect();
	entries.sort_by_key(|(key, ..)| *key);
	let size = entries.len().max(1) * std::mem::size_of::<ProcNameBucket>();
	unsafe {
		let buckets = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut ProcNameBucket;
		if buckets.is_null() {
			return Err("out of memory for a proc name map".to_owned());
		}
		let mut plan = NameMapPlan {
			map,
			buckets,
			count: 0,
			written: false
		};
		for (key, ids, no_link) in entries {
			let buffer = (functions.alloc_shared)(ids.len() * 4) as *mut u32;
			if buffer.is_null() {
				return Err("out of memory for a proc name map entry".to_owned());
			}
			std::ptr::copy_nonoverlapping(ids.as_ptr(), buffer, ids.len());
			buckets.add(plan.count as usize).write(ProcNameBucket {
				key: *key,
				ids: buffer,
				count: ids.len() as u16,
				no_link: *no_link
			});
			plan.count += 1;
		}
		Ok(plan)
	}
}

impl NameMapPlan {
	/// Install the map and release the old shared buffers. Retain the old
	/// bucket array, as with type matrices.
	pub fn write(mut self) {
		let Ok(functions) = functions() else {
			return;
		};
		unsafe {
			let old_ids: Vec<*mut u32> = buckets(self.map).iter().map(|bucket| bucket.ids).collect();
			(*self.map).buckets = self.buckets;
			(*self.map).count = self.count;
			for ids in old_ids {
				if !ids.is_null() {
					(functions.free_shared)(ids as *mut c_void);
				}
			}
		}
		self.written = true;
	}
}

impl Drop for NameMapPlan {
	fn drop(&mut self) {
		if self.written {
			return;
		}
		if let Ok(functions) = functions() {
			unsafe {
				for at in 0..self.count as usize {
					(functions.free_shared)((*self.buckets.add(at)).ids as *mut c_void);
				}
			}
		}
	}
}

/// Prepared parents and type preorder. [`ParentPlan::write`] cannot fail.
pub struct ParentPlan {
	changes: Vec<(u32, u32)>,
	order: Vec<u32>,
	places: Vec<(u32, u32)>
}

/// Recompute parents and the preorder used by `istype` and `typesof`,
/// visiting children by ascending ID as BYOND does. Leave other inherited
/// state unchanged.
pub fn plan_parents(changes: &[(u32, u32)]) -> Result<ParentPlan, String> {
	use type_fields::*;
	let functions = functions()?;
	let count = type_count()? as usize;
	for &(type_id, parent) in changes {
		if type_id as usize >= count || parent as usize >= count {
			return Err(format!("type {} or parent {} does not exist", type_id, parent));
		}
	}
	unsafe {
		let word = |id: usize, offset: usize| (*functions.type_table).add(id * functions.type_stride + offset) as *mut u32;
		let mut parents: Vec<u32> = (0..count).map(|id| *word(id, PARENT)).collect();
		for &(type_id, parent) in changes {
			parents[type_id as usize] = parent;
		}
		let mut children: Vec<Vec<u32>> = vec![vec![]; count];
		let mut roots = vec![];
		for (id, &parent) in parents.iter().enumerate() {
			if (parent as usize) < count && parent as usize != id {
				children[parent as usize].push(id as u32);
			} else {
				roots.push(id as u32);
			}
		}
		// (type, place, end) in the new preorder
		let mut order = Vec::with_capacity(count);
		let mut places = vec![(0u32, 0u32); count];
		// (type, next child to visit)
		let mut stack: Vec<(u32, usize)> = vec![];
		for root in roots {
			places[root as usize].0 = order.len() as u32;
			order.push(root);
			stack.push((root, 0));
			while let Some(top) = stack.last_mut() {
				let (id, next) = *top;
				match children[id as usize].get(next) {
					Some(&child) => {
						top.1 += 1;
						places[child as usize].0 = order.len() as u32;
						order.push(child);
						stack.push((child, 0));
					}
					None => {
						places[id as usize].1 = order.len() as u32;
						stack.pop();
					}
				}
			}
		}
		if order.len() != count {
			return Err(format!(
				"type hierarchy contains a cycle: preorder includes {} of {} types",
				order.len(),
				count
			));
		}
		Ok(ParentPlan {
			changes: changes.to_vec(),
			order,
			places
		})
	}
}

impl ParentPlan {
	pub fn write(&self) {
		use type_fields::*;
		let Ok(functions) = functions() else {
			return;
		};
		unsafe {
			let word = |id: usize, offset: usize| (*functions.type_table).add(id * functions.type_stride + offset) as *mut u32;
			for &(type_id, parent) in &self.changes {
				*word(type_id as usize, PARENT) = parent;
			}
			let preorder = *functions.preorder;
			for (at, &id) in self.order.iter().enumerate() {
				*preorder.add(at) = id;
				*word(id as usize, PREORDER) = self.places[id as usize].0;
				*word(id as usize, PREORDER_END) = self.places[id as usize].1;
			}
		}
	}
}

/// Copy a type's transform (`TRANSFORM`, 6 floats) or colour matrix
/// (`COLOR_MATRIX`, 20), or clear it. Each type owns its copy, as at load.
/// Retain the old allocation because readers may still reference it.
pub fn copy_type_matrix(type_id: u32, from: u32, offset: usize) -> Result<(), String> {
	use type_fields::*;
	let size = match offset {
		TRANSFORM => 6 * 4,
		COLOR_MATRIX => 20 * 4,
		_ => return Err(format!("+{:#x} isn't a type's matrix", offset))
	};
	let functions = functions()?;
	unsafe {
		let source = type_field(from, offset)? as *const u8;
		let copy = if source.is_null() {
			0
		} else {
			let copy = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut u8;
			if copy.is_null() {
				return Err("out of memory copying a type's matrix".to_owned());
			}
			std::ptr::copy_nonoverlapping(source, copy, size);
			copy as u32
		};
		set_type_field(type_id, offset, copy)
	}
}
