//! Finds the BYOND functions and globals auxtools needs, through `byond-scan`.
//!
//! Every miss is an error that names the lookup. Nothing here hands back a null
//! pointer, because the C++ shim calls these functions without checking and
//! `debug.rs` reads the globals the same way.

#[cfg(unix)]
mod linux;
#[cfg(windows)]
mod windows;

use std::ffi::c_void;

use byond_scan::{resolve_recipe, Module, Recipe};
#[cfg(unix)]
use linux::RECIPES;
#[cfg(windows)]
use windows::RECIPES;

use crate::raw_types::{funcs, lists, procs};

/// Supported BYOND major versions and their earliest checked builds.
/// Build numbers from different major versions do not identify the same binary.
const SUPPORTED_VERSIONS: &[(u32, u32)] = &[(515, 1637), (516, 1648)];

/// First Linux build with reversed global-pair layout: queue back before
/// front, and variable-name count before pointer.
const LINUX_FLIPPED_BUILD: u32 = 1644;

/// The first build with alists. Before it there's no alist table to find.
pub(crate) const ALIST_BUILD: u32 = 1648;

fn module_and_build() -> Result<(Module, (u32, u32)), String> {
	let module = Module::current().map_err(|err| format!("Couldn't locate BYOND core ({err})"))?;
	let build = byond_scan::version().map_err(|err| format!("Couldn't read BYOND version ({err})"))?;
	Ok((module, build))
}

/// Select the newest pattern for `name` at or before `build`.
fn recipe(name: &str, build: u32) -> Result<&'static Recipe, String> {
	pick(RECIPES.iter().filter(|recipe| recipe.name == name), build).ok_or_else(|| format!("Couldn't find {name} (no such pattern)"))
}

/// Of one name's patterns, the one for `build` (see `recipe`)
fn pick<'a>(patterns: impl Iterator<Item = &'a Recipe>, build: u32) -> Option<&'a Recipe> {
	let patterns: Vec<&Recipe> = patterns.collect();
	if let Some(recipe) = patterns.iter().find(|recipe| recipe.versions.contains(build)) {
		return Some(recipe);
	}
	patterns
		.iter()
		.filter(|recipe| recipe.versions.max < build)
		.max_by_key(|recipe| recipe.versions.max)
		.or(patterns.first())
		.copied()
}

fn resolve(module: &Module, build: (u32, u32), name: &str) -> Result<usize, String> {
	resolve_checked(module, build, recipe(name, build.1)?)
}

fn resolve_checked(module: &Module, (version, build): (u32, u32), recipe: &Recipe) -> Result<usize, String> {
	let name = recipe.name;

	// Older than anything checked is a hard stop. A newer build is still tried,
	// and a pattern that has stopped matching fails with its own error.
	let supported = SUPPORTED_VERSIONS.iter().any(|&(major, first)| major == version && build >= first);
	if !supported || recipe.versions.applicability(version, build, version).is_none() {
		return Err(format!(
			"Couldn't find {name} (BYOND {version}.{build} is outside the checked range {}..={})",
			recipe.versions.min, recipe.versions.max
		));
	}

	resolve_recipe(module, recipe, |_| None).map_err(|err| format!("Couldn't find {name} ({err})"))
}

/// Resolves a recipe of your own with the same rules auxtools applies to its
/// own: builds older than the recipe's range are refused, newer ones are tried.
pub fn find_recipe(recipe: &Recipe) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	resolve_checked(&module, build, recipe)
}

/// Resolve a symbol using the newest applicable recipe for the running build.
pub fn find_recipe_of(recipes: &[Recipe]) -> Result<usize, String> {
	let (module, build) = module_and_build()?;
	let recipe = pick(recipes.iter(), build.1).ok_or("Couldn't find a pattern (none given)")?;
	resolve_checked(&module, build, recipe)
}

/// The running BYOND build's number
pub fn byond_build() -> Result<u32, String> {
	Ok(module_and_build()?.1 .1)
}

/// Looks up every function and global except `variable_names`, and hands them
/// to the C++ shim and `raw_types::funcs`.
pub(crate) fn resolve_full() -> Result<(), String> {
	let (module, build) = module_and_build()?;
	let address = |name| resolve(&module, build, name);
	let function = |name| address(name).map(|address| address as *const c_void);

	unsafe {
		funcs::CURRENT_EXECUTION_CONTEXT = address("current_execution_context")? as *mut *mut procs::ExecutionContext;
		// Windows keeps the queue's front index first, then the back. So does
		// Linux before 515.1644; later Linux builds the other way round, and
		// there the pattern finds the back.
		let queue = address("suspended_procs")?;
		let (front, back) = if cfg!(unix) && build.1 >= LINUX_FLIPPED_BUILD {
			(queue + 4, queue)
		} else {
			(queue, queue + 4)
		};
		funcs::SUSPENDED_FRONT = front as *const usize;
		funcs::SUSPENDED_BACK = back as *const usize;
		funcs::SUSPENDED_PROCS_BUFFER = address("suspended_procs_buffer")? as *mut procs::SuspendedProcsBuffer;
		if build.1 >= ALIST_BUILD {
			funcs::ALIST_TABLE = address("alist_table_ptr")? as *const *const *const lists::Alist;
			funcs::ALIST_TABLE_COUNT = address("alist_table_count")? as *const u32;
		} else {
			// No alists: an empty table
			static NO_ALISTS: u32 = 0;
			funcs::ALIST_TABLE = std::ptr::null();
			funcs::ALIST_TABLE_COUNT = &NO_ALISTS;
		}
		funcs::call_proc_by_id_byond = function("call_proc_by_id")?;
		funcs::call_datum_proc_by_name_byond = function("call_datum_proc_by_name")?;
		funcs::get_proc_array_entry_byond = function("get_proc_array_entry")?;
		funcs::get_string_id_byond = function("get_string_id")?;
		funcs::get_variable_byond = function("get_variable")?;
		funcs::set_variable_byond = function("set_variable")?;
		funcs::get_string_table_entry_byond = function("get_string_table_entry")?;
		funcs::inc_ref_count_byond = function("inc_ref_count")?;
		funcs::dec_ref_count_byond = function("dec_ref_count")?;
		funcs::get_assoc_element_byond = function("get_assoc_element")?;
		funcs::set_assoc_element_byond = function("set_assoc_element")?;
		funcs::create_list_byond = function("create_list")?;
		funcs::append_to_list_byond = function("append_to_list")?;
		funcs::remove_from_list_byond = function("remove_from_list")?;
		#[cfg(unix)]
		{
			funcs::remove_from_list_in_registers = build.1 < linux::REMOVE_FROM_LIST_STACK_BUILD;
			funcs::get_length_returns_count = build.1 < LINUX_FLIPPED_BUILD;
		}
		funcs::get_length_byond = function("get_length")?;
		funcs::value_is_list_byond = function("value_is_list")?;
		funcs::get_misc_by_id_byond = function("get_misc_by_id")?;
		funcs::to_string_byond = function("to_string")?;
		funcs::runtime_byond = function("runtime")?;
	}

	Ok(())
}

/// Looks up `variable_names`. BYOND reallocates it for every world, so this
/// runs on every partial init and not only the first.
pub(crate) fn resolve_partial() -> Result<(), String> {
	let (module, build) = module_and_build()?;
	// The recipe finds the name array. Its count is the word after it on
	// Windows and on Linux before 515.1644, the word before it on later Linux
	// builds.
	let names = resolve(&module, build, "variable_names")?;
	let count = if cfg!(unix) && build.1 >= LINUX_FLIPPED_BUILD {
		names - 4
	} else {
		names + 4
	};
	unsafe {
		funcs::VARIABLE_NAME_ENTRIES = names as *const *const crate::raw_types::strings::StringId;
		funcs::VARIABLE_NAME_COUNT = count as *const u32;
	}
	Ok(())
}

/// The names that fail to resolve on the running build, for
/// `auxtools_check_signatures`.
pub(crate) fn unresolved() -> Result<Vec<&'static str>, String> {
	let (module, build) = module_and_build()?;
	let mut names: Vec<&'static str> = RECIPES
		.iter()
		.map(|recipe| recipe.name)
		.filter(|name| build.1 >= ALIST_BUILD || !name.starts_with("alist_"))
		.filter(|name| resolve(&module, build, name).is_err())
		.collect();
	names.dedup();
	Ok(names)
}
