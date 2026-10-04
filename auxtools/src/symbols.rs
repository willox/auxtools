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

use crate::raw_types::{funcs, procs, variables::VariableNameIdTable};

/// The BYOND major version the patterns were checked against. The same build
/// number under another major is a different binary.
const SUPPORTED_VERSION: u32 = 516;

fn module_and_build() -> Result<(Module, (u32, u32)), String> {
	let module = Module::current().map_err(|err| format!("Couldn't locate BYOND core ({err})"))?;
	let build = byond_scan::version().map_err(|err| format!("Couldn't read BYOND version ({err})"))?;
	Ok((module, build))
}

fn recipe(name: &str) -> Result<&'static Recipe, String> {
	RECIPES
		.iter()
		.find(|recipe| recipe.name == name)
		.ok_or_else(|| format!("Couldn't find {name} (no such pattern)"))
}

fn resolve(module: &Module, build: (u32, u32), name: &str) -> Result<usize, String> {
	resolve_checked(module, build, recipe(name)?)
}

fn resolve_checked(module: &Module, (version, build): (u32, u32), recipe: &Recipe) -> Result<usize, String> {
	let name = recipe.name;

	// Older than anything checked is a hard stop. A newer build is still tried,
	// and a pattern that has stopped matching fails with its own error.
	if recipe.versions.applicability(version, build, SUPPORTED_VERSION).is_none() {
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

/// Looks up every function and global except `variable_names`, and hands them
/// to the C++ shim and `raw_types::funcs`.
pub(crate) fn resolve_full() -> Result<(), String> {
	let (module, build) = module_and_build()?;
	let address = |name| resolve(&module, build, name);
	let function = |name| address(name).map(|address| address as *const c_void);

	unsafe {
		funcs::CURRENT_EXECUTION_CONTEXT = address("current_execution_context")? as *mut *mut procs::ExecutionContext;
		funcs::SUSPENDED_PROCS = address("suspended_procs")? as *mut procs::SuspendedProcs;
		funcs::SUSPENDED_PROCS_BUFFER = address("suspended_procs_buffer")? as *mut procs::SuspendedProcsBuffer;
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
	// the recipe finds the name array, which is not where the table starts on Linux
	let address = resolve(&module, build, "variable_names")? - std::mem::offset_of!(VariableNameIdTable, entries);
	unsafe {
		funcs::VARIABLE_NAMES = address as *const VariableNameIdTable;
	}
	Ok(())
}

/// The names that fail to resolve on the running build, for
/// `auxtools_check_signatures`.
pub(crate) fn unresolved() -> Result<Vec<&'static str>, String> {
	let (module, build) = module_and_build()?;
	Ok(RECIPES
		.iter()
		.map(|recipe| recipe.name)
		.filter(|name| resolve(&module, build, name).is_err())
		.collect())
}
