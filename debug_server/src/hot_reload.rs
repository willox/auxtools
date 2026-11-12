//! Apply code and metadata changes to a running world.
//!
//! Preparation validates and assembles the reload before replacing existing
//! code. Active and sleeping frames retain their original bytecode buffers.
//!
//! New procs update type lists, name maps and override chains. Removed
//! definitions are unlinked but remain allocated for active frames and old
//! call sites. Variable defaults update inheriting types; computed defaults
//! run through init procs. New variables receive appended IDs.
//!
//! Parameter `in` expressions use generated procs that return their lists.
//! Prefabs use init procs and instance-table entries.

use std::{
	collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet},
	sync::Mutex
};

use auxtools::{
	raw_types,
	runtime_tables::{self, type_fields, ProcEntryWords, NONE},
	shutdown, Proc, StringRef
};
use dmasm::{
	assembler::AssembleEnv as _,
	reload::{ConstValue, InList, NewType, NewVar, ProcSettings, Redeclared, RemovedProc},
	Instruction, Node, Opcode
};

use crate::{
	assemble_env::AssembleEnv,
	instruction_hooking::{hook_instruction, hooked_offsets, original_bytecode, unhook_instruction},
	server_types::{HotReloadInitProc, HotReloadNewProc, HotReloadProc, HotReloadRequest, HotReloadVarDefault, ProcRef, Reloaded}
};

/// A reload, checked and assembled, ready to apply.
#[derive(Default)]
pub struct Prepared {
	new_vars: Vec<PreparedNewVar>,
	/// New globals indexed by name; BYOND's original global list excludes them
	/// (see `added_global`).
	new_globals: Vec<(String, u32)>,
	removed_procs: Vec<PreparedRemoval>,
	new_procs: Vec<PreparedNewProc>,
	init_procs: Vec<PreparedInitProc>,
	code: Vec<PreparedCode>,
	/// (proc, word, value) for procs' new settings
	entry_words: Vec<(u32, usize, u32)>,
	/// Procs given new names, for the name maps
	renames: Vec<Rename>,
	/// Types whose default appearances must be rebuilt after verb or built-in
	/// changes.
	appearances: BTreeSet<u32>,
	/// New values of built-in vars, by type
	builtin_writes: Vec<(u32, Vec<BuiltinWrite>)>,
	/// (type id, field, misc id) for the types' new var tables and declared
	/// vars
	type_fields: Vec<(u32, usize, u32)>,
	/// (var id, name, bytecode) for initial values to work out once
	run_once: Vec<(u32, String, Vec<u32>)>,
	/// (var id, value) for the global slots of vars that are static now
	global_values: Vec<(u32, (u32, u32))>,
	/// Procs that return parameters' `in` lists, by the index each gets
	in_lists: Vec<(u32, ProcEntryWords)>,
	/// New prefab instances and init proc entries, in ID allocation order.
	instances: Vec<(u32, runtime_tables::Instance, ProcEntryWords)>
}

/// Variable to append: name string ID and initial global-slot value.
struct PreparedNewVar {
	/// Expected appended variable ID.
	id: u32,
	name: u32,
	value: (u32, u32)
}

/// Variables staged in ID allocation order.
struct AddedVars {
	first: u32,
	vars: Vec<PreparedNewVar>,
	/// Ids added to name parameters, by name
	param_names: HashMap<String, u32>,
	/// Computed global and static initializers: variable ID, label and
	/// bytecode.
	run_once: Vec<(u32, String, dmasm::reload::ProcReload)>,
	/// Existing parameter `in` proc count and staged additions by index.
	in_lists_before: u32,
	in_lists: Vec<(u32, ProcEntryWords)>,
	instances: StagedInstances
}

/// Existing and staged prefab instances, including init proc entries and
/// bytecode.
#[derive(Default)]
struct StagedInstances {
	existing: Option<Vec<runtime_tables::Instance>>,
	new: Vec<(runtime_tables::Instance, ProcEntryWords, Vec<u32>)>
}

impl StagedInstances {
	/// Find or stage a prefab instance of `path` with initializer `init`
	/// and typepath value `tag`/`data`.
	fn find_or_add(&mut self, path: &str, (tag, data): (u8, u32), init: &[u32]) -> Result<u32, String> {
		let existing = match &mut self.existing {
			Some(existing) => existing,
			none => none.insert(runtime_tables::instances()?)
		};
		let runs_init = |proc: u32| {
			runtime_tables::proc_entry(raw_types::procs::ProcId(proc))
				.and_then(|entry| runtime_tables::misc_words(entry.0[ProcEntryWords::BYTECODE]))
				.is_some_and(|code| code == init)
		};
		if let Some(id) = existing
			.iter()
			.position(|i| i.tag == tag && i.data == data && i.init != NONE && runs_init(i.init))
		{
			return Ok(id as u32);
		}
		let count = existing.len() as u32;
		if let Some(id) = self.new.iter().position(|(i, _, code)| i.tag == tag && i.data == data && code == init) {
			return Ok(count + id as u32);
		}

		// As DreamMaker makes them, named as BYOND names them when it loads
		// the world
		let mut entry = proc_template()?;
		for word in [
			ProcEntryWords::NAME,
			ProcEntryWords::DESC,
			ProcEntryWords::CATEGORY,
			ProcEntryWords::PATH_BASENAME,
			ProcEntryWords::PARAMETERS,
			ProcEntryWords::NEXT_OVERRIDE
		] {
			entry.0[word] = NONE;
		}
		entry.0[ProcEntryWords::PATH] = leaked_string(&format!("{}/proc/(map_init)", path))?;
		entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(init)?;
		self.new.push((runtime_tables::Instance { tag, data, init: NONE }, entry, init.to_vec()));
		Ok(count + self.new.len() as u32 - 1)
	}
}

impl AddedVars {
	const fn next_id(&self) -> u32 {
		self.first + self.vars.len() as u32
	}

	/// Find or allocate a variable ID for a parameter name. BYOND uses it
	/// only to match named arguments.
	fn param_name(&mut self, name: &str) -> Result<u32, String> {
		if let Some(id) = variable_ids(name.as_bytes()).first().or_else(|| self.param_names.get(name)).copied() {
			return Ok(id);
		}
		let id = self.add(leaked_string(name)?, (0, 0));
		self.param_names.insert(name.to_owned(), id);
		Ok(id)
	}

	/// A new var id, with its name's string id and its global slot's value
	fn add(&mut self, name: u32, value: (u32, u32)) -> u32 {
		let id = self.next_id();
		self.vars.push(PreparedNewVar { id, name, value });
		id
	}
}

/// Staged name maps and override links. Later additions in the same reload
/// resolve against earlier staged definitions.
#[derive(Default)]
struct StagedProcs {
	maps: HashMap<(u32, u32), Vec<u32>>,
	/// New `..()` targets of existing procs
	next_override: HashMap<u32, u32>,
	/// The entries of new procs
	entries: HashMap<u32, ProcEntryWords>,
	/// New procs and verbs, by the type they're on and its list's field
	own: HashMap<(u32, usize), Vec<u32>>,
	/// New verbs that go after another on their type's list, rather than
	/// first
	listed_after: HashMap<u32, u32>,
	/// Procs taken out of their types' lists
	removed: HashSet<u32>,
	/// Definitions by proc path in source order; each `..()` targets its
	/// predecessor.
	world: HashMap<String, Vec<u32>>
}

impl StagedProcs {
	fn named_procs(&self, type_id: u32, key: u32) -> Result<Vec<u32>, String> {
		match self.maps.get(&(type_id, key)) {
			Some(procs) => Ok(procs.clone()),
			None => runtime_tables::named_procs(type_id, key)
		}
	}

	fn entry(&self, proc: u32) -> Option<ProcEntryWords> {
		if let Some(entry) = self.entries.get(&proc) {
			return Some(*entry);
		}
		let mut entry = runtime_tables::proc_entry(raw_types::procs::ProcId(proc))?;
		if let Some(&next) = self.next_override.get(&proc) {
			entry.0[ProcEntryWords::NEXT_OVERRIDE] = next;
		}
		Some(entry)
	}

	/// Current staged definitions for `path`, without `/proc`.
	fn world_definitions(&mut self, path: &str) -> &mut Vec<u32> {
		self.world
			.entry(path.to_owned())
			.or_insert_with(|| Proc::overrides(path).iter().map(|p| p.id.0).collect())
	}

	/// Staged proc or verb list declared directly on a type.
	fn own(&self, type_id: u32, field: usize) -> Result<Vec<u32>, String> {
		let mut procs = runtime_tables::misc_words(runtime_tables::type_field(type_id, field)?).unwrap_or_default();
		procs.retain(|proc| !self.removed.contains(proc));
		for &proc in self.own.get(&(type_id, field)).into_iter().flatten() {
			match self.listed_after.get(&proc).and_then(|after| procs.iter().position(|p| p == after)) {
				Some(at) if !procs.contains(&proc) => procs.insert(at + 1, proc),
				_ => add_to_list(&mut procs, field, proc)
			}
		}
		Ok(procs)
	}
}

/// Staged additions available to assembled code under their expected IDs.
#[derive(Default)]
struct Pending {
	/// Procs, by path without `/proc`
	procs: HashMap<String, u32>,
	/// Static and global vars, by declaring type (`""` for a global) and name
	globals: HashMap<(String, Vec<u8>), u32>
}

/// Mutable copies of type variable tables, indexed by type ID.
/// Intern only after all edits are prepared.
#[derive(Default)]
struct TypeVars {
	tables: BTreeMap<u32, Vec<u32>>,
	declared: BTreeMap<u32, Vec<u32>>,
	/// The names of var ids this reload adds
	new_names: HashMap<u32, u32>,
	/// Each type's parent
	parents: Option<Vec<u32>>
}

impl TypeVars {
	/// Lazily read a type's misc entry at `field`.
	fn words(tables: &mut BTreeMap<u32, Vec<u32>>, type_id: u32, field: usize) -> Result<&mut Vec<u32>, String> {
		Ok(match tables.entry(type_id) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => {
				let id = runtime_tables::type_field(type_id, field)?;
				entry.insert(runtime_tables::misc_words(id).ok_or_else(|| format!("couldn't read the vars of type {}", type_id))?)
			}
		})
	}

	/// (name, var id, tag, data) per var, sorted by name
	fn table(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.tables, type_id, type_fields::VAR_TABLE)
	}

	/// (var id, flags) per var, sorted by the var ids' names
	fn declared(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.declared, type_id, type_fields::DECLARED_VARS)
	}

	fn name_of(&self, var_id: u32) -> u32 {
		self.new_names
			.get(&var_id)
			.copied()
			.or_else(|| runtime_tables::variable_name(var_id))
			.unwrap_or(u32::MAX)
	}

	/// `type_id` and every type that descends from it
	fn subtree(&mut self, type_id: u32) -> Result<Vec<u32>, String> {
		if self.parents.is_none() {
			let count = runtime_tables::type_count()?;
			self.parents = Some(
				(0..count)
					.map(|t| runtime_tables::type_field(t, type_fields::PARENT).unwrap_or(NONE))
					.collect()
			);
		}
		let parents = self.parents.as_ref().unwrap();
		Ok((0..parents.len() as u32)
			.filter(|&t| {
				let mut at = t;
				for _ in 0..parents.len() {
					if at == type_id {
						return true;
					}
					match parents.get(at as usize) {
						Some(&parent) if parent != NONE => at = parent,
						_ => return false
					}
				}
				false
			})
			.collect())
	}

	/// Intern changed tables as (type ID, field, misc ID).
	fn intern(self) -> Result<Vec<(u32, usize, u32)>, String> {
		let mut out = vec![];
		for (field, tables) in [(type_fields::VAR_TABLE, self.tables), (type_fields::DECLARED_VARS, self.declared)] {
			for (type_id, words) in tables {
				// (an empty one is none at all, as in a built world)
				let id = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
				out.push((type_id, field, id));
			}
		}
		Ok(out)
	}
}

/// New contents for an existing proc's table entry.
struct PreparedCode {
	proc: Proc,
	name: String,
	bytecode: Vec<u32>,
	bytecode_id: u32,
	locals_id: Option<u32>,
	params_id: Option<u32>,
	/// (word, value) for the entry's new settings
	settings: Vec<(usize, u32)>,
	/// Optional renamed proc metadata.
	rename: Option<Rename>,
	/// Replacement local names, if changed.
	local_names: Option<Vec<String>>,
	/// Line remapping for unchanged code. `None` for replaced code, where
	/// breakpoints retain their requested source lines.
	lines: Option<Vec<(u32, u32)>>
}

/// A type's new init proc
struct PreparedInitProc {
	type_id: u32,
	/// The new proc's entry and local names, or none for no init proc
	proc: Option<(ProcEntryWords, Vec<String>)>
}

struct PreparedNewProc {
	entry: ProcEntryWords,
	/// Expected proc ID; IDs are append-only.
	id: u32,
	/// Position in the path's definition list, used when renumbering overrides.
	place: ProcRef,
	local_names: Vec<String>,
	on_type: Option<NewProcOnType>,
	/// Where a world proc goes in the world's own name map
	on_world: Option<WorldEdit>
}

/// Unlinked proc definition. Keep its table entry, but remove it from
/// type lists, name maps and override chains.
struct PreparedRemoval {
	id: u32,
	/// `NONE` for a global proc's or the world's: neither is on a type
	type_id: u32,
	/// The type's list it's on: `type_fields::PROCS` or `VERBS`
	field: usize,
	/// (type id, name key, procs) for each proc name map it was in
	map_entries: Vec<(u32, u32, Vec<u32>)>,
	/// Procs whose `..()` must skip the removed definition.
	relinked: Vec<u32>,
	next: u32,
	/// How a world proc comes out of the world's own name map
	on_world: Option<WorldEdit>
}

/// Type metadata changes required when adding a proc.
struct NewProcOnType {
	type_id: u32,
	/// The type's list it goes on: `type_fields::PROCS` or `VERBS`
	field: usize,
	/// (type id, name key, procs) for each proc name map that gets it
	map_entries: Vec<(u32, u32, Vec<u32>)>,
	/// Procs whose `..()` must target the new definition.
	relinked: Vec<u32>,
	/// Insert after this proc, or use the default `add_to_list` position.
	after: Option<u32>
}

/// A change to the world's own proc names (see `edit_world_names`)
#[derive(Clone)]
enum WorldEdit {
	/// Insert under `keys`, overriding `below` and overridden by `above`,
	/// or as the most derived definition if `above` is absent.
	Add {
		id: u32,
		keys: Vec<u32>,
		below: u32,
		above: Option<u32>
	},
	/// Remove from `keys` and redirect incoming `..()` calls to its
	/// predecessor.
	Remove { id: u32, keys: Vec<u32> }
}

/// Map each client's original breakpoint offsets to their reloaded positions.
/// Retain the mapping until the client acknowledges the new position by
/// setting or removing the breakpoint there, or disconnects.
pub type MovedBreakpoints<K> = HashMap<(raw_types::procs::ProcId, u32, K), u32>;

fn describe(proc: &ProcRef) -> String {
	if proc.override_id == 0 {
		proc.path.clone()
	} else {
		format!("{} (override {})", proc.path, proc.override_id)
	}
}

fn find(proc: &ProcRef) -> Result<Proc, String> {
	Proc::find_override(&proc.path, proc.override_id).ok_or_else(|| format!("{}: there's no such proc in the running world", describe(proc)))
}

/// A proc path the way auxtools keys procs: without `/proc` or `/verb`.
fn strip_path(path: &str) -> String {
	path.replace("/proc/", "/").replace("/verb/", "/")
}

/// Validate and assemble a reload. Errors prevent application.
pub fn prepare(request: &HotReloadRequest, warnings: &mut Vec<String>) -> Result<Prepared, Vec<String>> {
	let HotReloadRequest {
		new_types: _,
		procs,
		settings,
		new_procs,
		removed_procs,
		new_vars,
		redeclared,
		var_defaults,
		builtin_defaults,
		init_procs,
		line_moves
	} = request;
	runtime_tables::available().map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;

	let mut prepared = Prepared::default();
	let mut errors = vec![];
	let mut world_globals = WorldGlobals::default();

	let mut pending = Pending::default();
	let mut type_vars = TypeVars::default();

	// Reserve IDs in allocation order so new code can refer to staged
	// additions.
	let mut added = AddedVars {
		first: runtime_tables::variable_count(),
		vars: vec![],
		param_names: HashMap::new(),
		run_once: vec![],
		in_lists_before: runtime_tables::in_list_count().unwrap_or(0),
		in_lists: vec![],
		instances: StagedInstances::default()
	};
	for new in new_vars {
		if let Err(e) = prepare_new_var(new, &mut added, &mut type_vars, &mut pending) {
			errors.push(e);
		}
	}
	for var in redeclared {
		if let Err(e) = prepare_redeclared(var, &mut added, &mut type_vars, &mut pending, &mut prepared.global_values) {
			errors.push(e);
		}
	}

	let mut staged = StagedProcs::default();
	for removed in removed_procs {
		// An already removed proc must not block subsequent reloads.
		if Proc::overrides(removed.proc.path.as_str()).is_empty() {
			warnings.push(format!("{}: not found in the running world; skipping removal", removed.proc.path));
			continue;
		}
		match prepare_removal(removed, &mut staged) {
			Ok(p) => {
				if removed.verb {
					prepared.appearances.extend(type_vars.subtree(p.type_id).unwrap_or_default());
				}
				prepared.removed_procs.push(p);
			}
			Err(e) => errors.push(e)
		}
	}

	// New procs get the next ids, in order
	let first_id = runtime_tables::proc_count();
	let ids: Vec<u32> = (first_id..).take(new_procs.len()).collect();
	pending.procs = new_procs
		.iter()
		.zip(&ids)
		.map(|(new, &id)| (strip_path(&new.code.proc.path), id))
		.collect();

	for (new, &id) in new_procs.iter().zip(&ids) {
		match prepare_new_proc(new, id, &pending, &mut world_globals, &mut added, &mut staged) {
			Ok(p) => {
				if let Some(on_type) = p.on_type.as_ref().filter(|_| new.verb) {
					prepared.appearances.extend(type_vars.subtree(on_type.type_id).unwrap_or_default());
				}
				prepared.new_procs.push(p);
			}
			Err(e) => errors.push(e)
		}
	}

	for init in init_procs {
		match prepare_init_proc(init, &pending, &mut world_globals, &mut added) {
			Ok(p) => prepared.init_procs.push(p),
			Err(e) => errors.push(e)
		}
	}

	for reload in procs {
		match prepare_code(reload, &pending, &mut world_globals, &mut added, warnings) {
			Ok(p) => prepared.code.push(p),
			Err(e) => errors.push(e)
		}
	}

	for change in settings {
		let proc_ref = ProcRef {
			path: change.proc.path.clone(),
			override_id: change.proc.override_id
		};
		let words = find(&proc_ref).and_then(|proc| {
			let entry = runtime_tables::proc_entry(proc.id).ok_or_else(|| format!("{}: couldn't read its table entry", describe(&proc_ref)))?;
			let words = settings_words(&entry, &change.settings, &proc_ref.path)?;
			prepared.renames.extend(renamed(&proc, &entry, &words));
			Ok(words.into_iter().map(move |(word, value)| (proc.id.0, word, value)))
		});
		match words {
			Ok(words) => prepared.entry_words.extend(words),
			Err(e) => errors.push(e)
		}
	}

	for moved in line_moves {
		let proc = match find(&moved.proc) {
			Ok(proc) => proc,
			Err(e) => {
				errors.push(e);
				continue;
			}
		};

		// A world built without DEBUG has no lines to move
		if let Some(bytecode) = remap_lines(&original_bytecode(&proc), &moved.lines) {
			match runtime_tables::intern_misc(&bytecode) {
				Ok(bytecode_id) => prepared.code.push(PreparedCode {
					proc,
					name: describe(&moved.proc),
					bytecode,
					bytecode_id,
					locals_id: None,
					params_id: None,
					settings: vec![],
					rename: None,
					local_names: None,
					lines: Some(moved.lines.clone())
				}),
				Err(e) => errors.push(format!("{}: {}", describe(&moved.proc), e))
			}
		}
	}

	for default in var_defaults {
		if let Err(e) = prepare_var_default(default, &mut type_vars) {
			errors.push(e);
		}
	}
	for default in builtin_defaults {
		let writes = builtin_writes(&default.var, &default.value).map_err(|e| format!("{}: {}", default.var, e));
		match writes {
			Ok(writes) => {
				for path in &default.types {
					match find_type(path) {
						Ok(type_id) => {
							prepared.builtin_writes.push((type_id, writes.clone()));
							prepared.appearances.insert(type_id);
						}
						Err(e) => errors.push(e)
					}
				}
			}
			Err(e) => errors.push(e)
		}
	}

	// Computed initializers may use other additions in this reload.
	for (id, name, init) in std::mem::take(&mut added.run_once) {
		let code = value_code(init);
		if !code.locals.is_empty() {
			errors.push(format!(
				"{}: working out its initial value needs locals, which a reload can't give it yet",
				name
			));
			continue;
		}
		match assemble(&name, &code, &[], &pending, &mut world_globals, &mut added) {
			Ok(bytecode) => prepared.run_once.push((id, name, bytecode)),
			Err(e) => errors.push(e)
		}
	}

	if !prepared.appearances.is_empty() {
		warnings
			.push("things made before the reload keep the verbs and built-in values they had; things made after get them as they are now".to_owned());
	}

	prepared.new_vars = added.vars;
	prepared.new_globals = pending
		.globals
		.iter()
		.filter(|((scope, _), _)| scope.is_empty())
		.map(|((_, name), id)| (String::from_utf8_lossy(name).into_owned(), *id))
		.collect();
	prepared.in_lists = added.in_lists;
	let first_instance = added.instances.existing.as_ref().map_or(0, Vec::len) as u32;
	prepared.instances = added
		.instances
		.new
		.into_iter()
		.enumerate()
		.map(|(n, (instance, entry, _))| (first_instance + n as u32, instance, entry))
		.collect();
	if errors.is_empty() {
		match type_vars.intern() {
			Ok(fields) => prepared.type_fields = fields,
			Err(e) => errors.push(format!("couldn't make the types' new vars: {}", e))
		}
	}

	if errors.is_empty() {
		Ok(prepared)
	} else {
		Err(errors)
	}
}

/// Assembles a proc's code for the running world.
fn assemble(
	name: &str,
	code: &HotReloadProc,
	original: &[u32],
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<Vec<u32>, String> {
	let nodes = adapt_debug_info(&code.nodes, original);

	let proc_globals = globals_in(original);
	let new_statics = new_statics(code, &proc_globals, added).map_err(|e| format!("{}: {}", name, e))?;
	let mut env = ReloadEnv {
		proc_globals,
		world_globals,
		pending,
		new_statics,
		instances: &mut added.instances,
		error: None
	};
	let bytecode = match dmasm::assembler::assemble(&nodes, &mut env) {
		Ok(bytecode) => bytecode,
		Err(e) => return Err(format!("{}: {}", name, env.error.take().unwrap_or_else(|| describe_assemble_error(&e))))
	};

	if bytecode.len() > u16::MAX as usize {
		return Err(format!(
			"{}: the new code is {} words long, and BYOND's limit is {}",
			name,
			bytecode.len(),
			u16::MAX
		));
	}

	Ok(bytecode)
}

fn prepare_code(
	reload: &HotReloadProc,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars,
	warnings: &mut Vec<String>
) -> Result<PreparedCode, String> {
	let name = describe(&reload.proc);
	let proc = find(&reload.proc)?;
	let entry = runtime_tables::proc_entry(proc.id).ok_or_else(|| format!("{}: couldn't read its table entry", name))?;

	if proc.is_hooked() {
		warnings.push(format!("{}: auxtools has hooked it, so the hook runs instead of the new code", name));
	}

	let original = original_bytecode(&proc);
	let bytecode = assemble(&name, reload, &original, pending, world_globals, added)?;
	let bytecode_id = runtime_tables::intern_misc(&bytecode).map_err(|e| format!("{}: {}", name, e))?;

	// Frames get as many locals as the table has names
	let locals_id = if reload.locals.len() == proc.locals_count() {
		None
	} else {
		Some(locals_table(&reload.locals, entry.0[ProcEntryWords::LOCALS]).map_err(|e| format!("{}: {}", name, e))?)
	};

	let params_id =
		params_table(&reload.params, entry.0[ProcEntryWords::PARAMETERS], pending, world_globals, added).map_err(|e| format!("{}: {}", name, e))?;

	let settings = match &reload.settings {
		Some(settings) => settings_words(&entry, settings, &reload.proc.path).map_err(|e| format!("{}: {}", name, e))?,
		None => vec![]
	};
	let rename = renamed(&proc, &entry, &settings);

	Ok(PreparedCode {
		proc,
		name,
		bytecode,
		bytecode_id,
		locals_id,
		params_id,
		settings,
		rename,
		local_names: Some(reload.locals.clone()),
		lines: None
	})
}

/// Compute changed proc settings as entry-word updates: flags, `set src`,
/// invisibility, name, description and category. `path` identifies the proc.
fn settings_words(entry: &ProcEntryWords, settings: &ProcSettings, path: &str) -> Result<Vec<(usize, u32)>, String> {
	let string = |text: Option<&str>| text.map_or(Ok(NONE), leaked_string);
	// Without `set name`, the name in its path with spaces for underscores
	let basename = path.rsplit('/').next().unwrap_or(path).replace('_', " ");
	let words = [
		(
			ProcEntryWords::FLAGS,
			(entry.0[ProcEntryWords::FLAGS] & !ProcSettings::FLAGS) | settings.flags()
		),
		(ProcEntryWords::SETTINGS, settings.packed()),
		(ProcEntryWords::NAME, string(Some(settings.name.as_deref().unwrap_or(&basename)))?),
		(ProcEntryWords::DESC, string(settings.desc.as_deref())?),
		(ProcEntryWords::CATEGORY, string(settings.category.as_deref())?)
	];
	Ok(words.into_iter().filter(|&(word, value)| entry.0[word] != value).collect())
}

/// Update dynamic-call name maps for a renamed proc on its type and subtypes.
#[derive(Clone, Copy)]
struct Rename {
	proc: u32,
	type_id: u32,
	from: u32,
	to: u32,
	/// The name of its path, which it's also found by
	basename: u32
}

/// Find a renamed proc whose table updates require name-map changes.
fn renamed(proc: &Proc, entry: &ProcEntryWords, words: &[(usize, u32)]) -> Option<Rename> {
	let to = words.iter().find(|&&(word, _)| word == ProcEntryWords::NAME)?.1;
	let from = entry.0[ProcEntryWords::NAME];
	if from == NONE || from == to {
		return None;
	}
	let path = strip_path(&proc.path);
	let (type_path, _) = path.rsplit_once('/')?;
	let type_id = find_type(type_path).ok()?;
	let on_map = runtime_tables::named_procs(type_id, from).is_ok_and(|procs| procs.contains(&proc.id.0));
	on_map.then_some(Rename {
		proc: proc.id.0,
		type_id,
		from,
		to,
		basename: entry.0[ProcEntryWords::PATH_BASENAME]
	})
}

/// Move a renamed proc between name-map entries, preserving override order.
fn rename(r: Rename) -> Result<(), String> {
	let depth = |id: u32| {
		Proc::from_id(raw_types::procs::ProcId(id))
			.and_then(|p| {
				let path = strip_path(&p.path);
				let (type_path, _) = path.rsplit_once('/')?;
				find_type(type_path).ok()
			})
			.map_or(0, |t| type_and_ancestors(t).len())
	};
	let own_depth = depth(r.proc);
	for sub in 0..runtime_tables::type_count()? {
		if !type_and_ancestors(sub).contains(&r.type_id) {
			continue;
		}
		// Still found by its path's name
		if r.from != r.basename {
			let mut old = runtime_tables::named_procs(sub, r.from)?;
			if let Some(at) = old.iter().position(|&p| p == r.proc) {
				old.remove(at);
				runtime_tables::set_named_procs(sub, r.from, &old)?;
			}
		}
		let mut new = runtime_tables::named_procs(sub, r.to)?;
		if !new.contains(&r.proc) {
			let at = new.iter().position(|&p| depth(p) < own_depth).unwrap_or(new.len());
			new.insert(at, r.proc);
			runtime_tables::set_named_procs(sub, r.to, &new)?;
		}
	}
	Ok(())
}

/// Build a locals table, reusing `empty` for zero locals because BYOND does
/// not handle creating empty misc entries reliably.
///
/// BYOND initializes locals from their variable IDs' global slots. Choose
/// IDs with null slots; the debugger stores display names separately.
pub(crate) fn locals_table(names: &[String], empty: u32) -> Result<u32, String> {
	if names.is_empty() {
		return Ok(empty);
	}
	let null_slot = |id: &u32| runtime_tables::global_value(*id).is_ok_and(|v| v.tag == raw_types::values::ValueTag::Null);
	let mut words = vec![];
	for name in names {
		let id = match variable_ids(name.as_bytes()).into_iter().find(null_slot) {
			Some(id) => id,
			None => {
				let count = unsafe { *raw_types::funcs::VARIABLE_NAME_COUNT };
				(0..count).find(null_slot).ok_or("no var id has a null value to start a local with")?
			}
		};
		words.push(id);
	}
	runtime_tables::intern_misc(&words)
}

/// Wrap compiled value code (an initializer or parameter `in` list) as a proc
/// for assembly.
fn value_code(code: dmasm::reload::ProcReload) -> HotReloadProc {
	HotReloadProc {
		proc: ProcRef {
			path: code.proc.path,
			override_id: 0
		},
		nodes: code.nodes,
		locals: code.locals,
		params: vec![],
		settings: code.settings,
		statics: vec![]
	}
}

/// Add a generated proc returning a parameter's `in` list.
/// Return the encoded parameter word.
fn in_list_proc(
	param: &str,
	code: &dmasm::reload::ProcReload,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<u32, String> {
	let shown = format!("parameter {}'s `in` list", param);
	let template = proc_template()?;
	let code = value_code(code.clone());
	let bytecode = assemble(&shown, &code, &[], pending, world_globals, added)?;

	let mut entry = template;
	for word in [
		ProcEntryWords::PATH,
		ProcEntryWords::NAME,
		ProcEntryWords::DESC,
		ProcEntryWords::CATEGORY,
		ProcEntryWords::PATH_BASENAME,
		ProcEntryWords::NEXT_OVERRIDE
	] {
		entry.0[word] = NONE;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&code.locals, template.0[ProcEntryWords::LOCALS])?;

	let index = added.in_lists_before + added.in_lists.len() as u32;
	added.in_lists.push((index, entry));
	Ok((index << 8) | 0x40)
}

fn params_table(
	params: &[dmasm::reload::Param],
	current: u32,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<Option<u32>, String> {
	let current_words = runtime_tables::misc_words(current).unwrap_or_default();
	let name_of = |words: &[u32]| {
		unsafe { StringRef::from_variable_id(raw_types::strings::VariableId(words[2])) }
			.data()
			.to_vec()
	};
	let current_in_list = |name: &str| -> Option<u32> {
		current_words
			.chunks(4)
			.find(|words| words.len() == 4 && name_of(words) == name.as_bytes())
			.map(|words| words[1])
	};

	let mut in_lists = vec![];
	for param in params {
		in_lists.push(match &param.in_list {
			InList::Builtin(word) => *word,
			InList::Kept => current_in_list(&param.name)
				.ok_or_else(|| format!("parameter {} has an `in` list expression the running proc doesn't", param.name))?,
			InList::New(code) => in_list_proc(&param.name, code, pending, world_globals, added)?
		});
	}

	let unchanged = current_words.len() == params.len() * 4
		&& params
			.iter()
			.zip(&in_lists)
			.zip(current_words.chunks(4))
			.all(|((param, &in_list), words)| {
				words[0] == param.input && words[1] == in_list && words[3] == param.input_extra && name_of(words) == param.name.as_bytes()
			});
	if unchanged {
		return Ok(None);
	}
	// What a proc with no parameters has
	if params.is_empty() {
		return Ok(Some(NONE));
	}

	let mut words = vec![];
	for (param, in_list) in params.iter().zip(in_lists) {
		let id = added.param_name(&param.name).map_err(|e| format!("parameter {}: {}", param.name, e))?;
		words.extend([param.input, in_list, id, param.input_extra]);
	}
	runtime_tables::intern_misc(&words).map(Some)
}

/// The type whose path is `path`.
fn find_type(path: &str) -> Result<u32, String> {
	let id = StringRef::new(path).map_err(|_| format!("couldn't make the string {}", path))?.get_id().0;
	(0..runtime_tables::type_count()?)
		.find(|&t| runtime_tables::type_field(t, type_fields::PATH) == Ok(id))
		.ok_or_else(|| format!("there's no type {} in the running world", path))
}

/// Resolve `/image` and its subtypes directly from the type table.
/// text2path("/image") can return null before a rebooted world starts
/// executing.
pub(crate) fn image_type(path: &str) -> Option<u32> {
	let id = find_type(path).ok()?;
	let image = find_type("/image").ok()?;
	type_and_ancestors(id).contains(&image).then_some(id)
}

/// The types `type_id` is or descends from, itself first.
fn type_and_ancestors(type_id: u32) -> Vec<u32> {
	let mut out = vec![type_id];
	let mut at = type_id;
	while let Ok(parent) = runtime_tables::type_field(at, type_fields::PARENT) {
		if parent == NONE || out.contains(&parent) {
			break;
		}
		out.push(parent);
		at = parent;
	}
	out
}

fn prepare_init_proc(
	init: &HotReloadInitProc,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<PreparedInitProc, String> {
	let shown = format!("{}'s init proc", init.type_path);
	let fail = |e: String| format!("{}: {}", shown, e);
	let type_id = find_type(&init.type_path).map_err(fail)?;
	let Some(code) = &init.code else {
		return Ok(PreparedInitProc { type_id, proc: None });
	};

	// Like the one it replaces, or like any proc
	let current = runtime_tables::type_field(type_id, type_fields::INIT_PROC).map_err(fail)?;
	let current = (current != NONE).then(|| Proc::from_id(raw_types::procs::ProcId(current))).flatten();
	let template = match &current {
		Some(proc) => runtime_tables::proc_entry(proc.id),
		None => proc_template().ok()
	}
	.ok_or_else(|| fail("couldn't find a proc entry to copy".to_owned()))?;
	let original = current.as_ref().map(original_bytecode).unwrap_or_default();

	let bytecode = assemble(&shown, code, &original, pending, world_globals, added)?;
	let mut entry = template;
	if current.is_none() {
		// As BYOND names the init procs it makes
		entry.0[ProcEntryWords::PATH] = leaked_string(&format!("{}/proc/(init)", init.type_path)).map_err(fail)?;
		entry.0[ProcEntryWords::NAME] = NONE;
		entry.0[ProcEntryWords::DESC] = NONE;
		entry.0[ProcEntryWords::CATEGORY] = NONE;
		entry.0[ProcEntryWords::PATH_BASENAME] = NONE;
		entry.0[ProcEntryWords::PARAMETERS] = NONE;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode).map_err(fail)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&code.locals, template.0[ProcEntryWords::LOCALS]).map_err(fail)?;
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = NONE;

	Ok(PreparedInitProc {
		type_id,
		proc: Some((entry, code.locals.clone()))
	})
}

fn prepare_removal(removed: &RemovedProc, staged: &mut StagedProcs) -> Result<PreparedRemoval, String> {
	let proc_ref = ProcRef {
		path: removed.proc.path.clone(),
		override_id: removed.proc.override_id
	};
	let shown = describe(&proc_ref);
	let fail = |e: String| format!("{}: {}", shown, e);
	let id = find(&proc_ref)?.id.0;
	let entry = staged.entry(id).ok_or_else(|| fail("couldn't read its table entry".to_owned()))?;
	let next = entry.0[ProcEntryWords::NEXT_OVERRIDE];

	// Keep removed global procs for old call sites but exclude them from
	// override numbering. Remove world procs from the world name map.
	if removed.type_path.is_empty() || (removed.type_path == "/world" && !removed.verb) {
		let on_world = if removed.type_path.is_empty() {
			None
		} else {
			staged.world_definitions(&strip_path(&proc_ref.path)).retain(|&p| p != id);
			Some(WorldEdit::Remove {
				id,
				keys: world_keys(&entry)
			})
		};
		return Ok(PreparedRemoval {
			id,
			type_id: NONE,
			field: type_fields::PROCS,
			map_entries: vec![],
			relinked: vec![],
			next,
			on_world
		});
	}

	let type_id = find_type(&removed.type_path).map_err(fail)?;
	let field = if removed.verb { type_fields::VERBS } else { type_fields::PROCS };
	// Verbs aren't in the proc name maps
	let keys = if removed.verb {
		vec![]
	} else {
		vec![entry.0[ProcEntryWords::NAME], entry.0[ProcEntryWords::PATH_BASENAME]]
	};

	let mut map_entries = vec![];
	let mut relinked = vec![];
	for sub in 0..runtime_tables::type_count().map_err(fail)? {
		if !type_and_ancestors(sub).contains(&type_id) {
			continue;
		}
		for (k, &key) in keys.iter().enumerate() {
			if k == 1 && key == keys[0] {
				continue;
			}
			let mut procs = staged.named_procs(sub, key).map_err(fail)?;
			let before = procs.len();
			procs.retain(|&p| p != id);
			if procs.len() != before {
				staged.maps.insert((sub, key), procs.clone());
				map_entries.push((sub, key, procs));
			}
		}
		for proc in staged.own(sub, field).map_err(fail)? {
			if proc != id && staged.entry(proc).is_some_and(|e| e.0[ProcEntryWords::NEXT_OVERRIDE] == id) && !relinked.contains(&proc) {
				relinked.push(proc);
			}
		}
	}

	// New procs in this reload see it gone
	for &proc in &relinked {
		if let Some(e) = staged.entries.get_mut(&proc) {
			e.0[ProcEntryWords::NEXT_OVERRIDE] = next;
		} else {
			staged.next_override.insert(proc, next);
		}
	}
	staged.removed.insert(id);

	Ok(PreparedRemoval {
		id,
		type_id,
		field,
		map_entries,
		relinked,
		next,
		on_world: None
	})
}

fn prepare_new_proc(
	new: &HotReloadNewProc,
	id: u32,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars,
	staged: &mut StagedProcs
) -> Result<PreparedNewProc, String> {
	let shown = describe(&new.code.proc);
	let fail = |e: String| format!("{}: {}", shown, e);

	// Copied from a plain proc, like BYOND copies an existing proc for the
	// init procs it makes
	let template = proc_template().map_err(fail)?;

	let bytecode = assemble(&shown, &new.code, &[], pending, world_globals, added)?;

	let string = |text: &str| -> Result<u32, String> {
		let string = StringRef::new(text).map_err(|_| fail(format!("couldn't make the string {}", text)))?;
		let id = string.get_id().0;
		// The proc table entry holds it from now on
		std::mem::forget(string);
		Ok(id)
	};

	// As DreamMaker writes them: `proc/` or `verb/` where the proc is
	// declared
	let path = match (new.type_path.as_str(), new.declared, new.verb) {
		("", true, _) => format!("/proc/{}", new.name),
		(ty, true, false) => format!("{}/proc/{}", ty, new.name),
		(ty, true, true) => format!("{}/verb/{}", ty, new.name),
		(ty, false, _) => format!("{}/{}", ty, new.name)
	};

	let mut entry = template;
	entry.0[ProcEntryWords::PATH] = string(&path)?;
	entry.0[ProcEntryWords::PATH_BASENAME] = string(&new.name)?;
	let settings = new.code.settings.clone().unwrap_or_default();
	for (word, value) in settings_words(&entry, &settings, &new.name).map_err(fail)? {
		entry.0[word] = value;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode).map_err(fail)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&new.code.locals, template.0[ProcEntryWords::LOCALS]).map_err(fail)?;
	entry.0[ProcEntryWords::PARAMETERS] = if new.code.params.is_empty() {
		template.0[ProcEntryWords::PARAMETERS]
	} else {
		params_table(&new.code.params, NONE, pending, world_globals, added)
			.map_err(fail)?
			.unwrap()
	};
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = NONE;

	// World procs use a separate name map. Insert between the adjacent
	// definitions when committing the addition (see `edit_world_names`).
	let at = new.code.proc.override_id as usize;
	let mut on_world = None;
	if new.type_path == "/world" && !new.verb {
		let definitions = staged.world_definitions(&strip_path(&new.code.proc.path));
		let at = at.min(definitions.len());
		let below = at.checked_sub(1).map_or(NONE, |below| definitions[below]);
		let new_ids: Vec<u32> = staged.entries.keys().copied().collect();
		let definitions = staged.world_definitions(&strip_path(&new.code.proc.path));
		// The world has this one already: new ones go in in order
		let above = definitions[at..].iter().copied().find(|p| !new_ids.contains(p));
		definitions.insert(at, id);
		entry.0[ProcEntryWords::NEXT_OVERRIDE] = below;
		staged.entries.insert(id, entry);
		on_world = Some(WorldEdit::Add {
			id,
			keys: world_keys(&entry),
			below,
			above
		});
	}

	let on_type = if new.type_path.is_empty() || on_world.is_some() {
		None
	} else if new.verb {
		Some(new_verb_on_type(&new.type_path, id, at, &mut entry, staged).map_err(fail)?)
	} else {
		let type_id = find_type(&new.type_path).map_err(fail)?;
		let keys = [entry.0[ProcEntryWords::NAME], entry.0[ProcEntryWords::PATH_BASENAME]];

		// Find the predecessor at this insertion point, including inherited
		// definitions when inserting before the type's first override.
		let own = own_definitions(staged, type_id, type_fields::PROCS, ProcEntryWords::NAME, keys[0]).map_err(fail)?;
		let overridden = match (own.get(at), at.checked_sub(1).and_then(|below| own.get(below))) {
			(Some(_), Some(&below)) => below,
			(Some(&first), None) => staged.entry(first).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]),
			(None, _) => staged.named_procs(type_id, keys[0]).map_err(fail)?.first().copied().unwrap_or(NONE)
		};
		entry.0[ProcEntryWords::NEXT_OVERRIDE] = overridden;

		// Insert into this type and every subtype, preserving more derived
		// overrides before it and its predecessors after it.
		let mut map_entries = vec![];
		let mut relinked = vec![];
		for sub in 0..runtime_tables::type_count().map_err(fail)? {
			let line = type_and_ancestors(sub);
			if !line.contains(&type_id) {
				continue;
			}

			for (k, &key) in keys.iter().enumerate() {
				if k == 1 && key == keys[0] {
					continue;
				}
				let mut procs = staged.named_procs(sub, key).map_err(fail)?;
				let at = procs.iter().position(|&p| p == overridden).unwrap_or(procs.len());
				procs.insert(at, id);
				map_entries.push((sub, key, procs));
			}

			// Redirect the next definition's `..()` through the inserted proc.
			for proc in staged.own(sub, type_fields::PROCS).map_err(fail)? {
				if let Some(e) = staged.entry(proc) {
					if e.0[ProcEntryWords::NAME] == keys[0] && e.0[ProcEntryWords::NEXT_OVERRIDE] == overridden && !relinked.contains(&proc) {
						relinked.push(proc);
					}
				}
			}
		}

		// Procs added after this one in the same reload see it
		for (sub, key, procs) in &map_entries {
			staged.maps.insert((*sub, *key), procs.clone());
		}
		for &proc in &relinked {
			if let Some(e) = staged.entries.get_mut(&proc) {
				e.0[ProcEntryWords::NEXT_OVERRIDE] = id;
			} else {
				staged.next_override.insert(proc, id);
			}
		}
		staged.entries.insert(id, entry);
		staged.own.entry((type_id, type_fields::PROCS)).or_default().push(id);

		Some(NewProcOnType {
			type_id,
			field: type_fields::PROCS,
			map_entries,
			relinked,
			after: None
		})
	};

	Ok(PreparedNewProc {
		entry,
		id,
		place: new.code.proc.clone(),
		local_names: new.code.locals.clone(),
		on_type,
		on_world
	})
}

/// Append procs to `PROCS`; prepend verbs to `VERBS`.
/// DreamMaker stores verbs newest first and calls the first match.
fn add_to_list(list: &mut Vec<u32>, field: usize, proc: u32) {
	if list.contains(&proc) {
		return;
	}
	if field == type_fields::VERBS {
		list.insert(0, proc);
	} else {
		list.push(proc);
	}
}

/// Insert a verb at definition index `at`. Verbs use type verb lists,
/// not proc name maps. `..()` resolves the preceding local definition
/// or the nearest ancestor's latest definition.
fn new_verb_on_type(type_path: &str, id: u32, at: usize, entry: &mut ProcEntryWords, staged: &mut StagedProcs) -> Result<NewProcOnType, String> {
	let type_id = find_type(type_path)?;
	let name = entry.0[ProcEntryWords::PATH_BASENAME];

	let own = own_definitions(staged, type_id, type_fields::VERBS, ProcEntryWords::PATH_BASENAME, name)?;
	let below = match at.checked_sub(1).and_then(|below| own.get(below)) {
		Some(&below) => below,
		None => {
			let mut overridden = NONE;
			for t in type_and_ancestors(type_id).into_iter().skip(1) {
				if let Some(&newest) = own_definitions(staged, t, type_fields::VERBS, ProcEntryWords::PATH_BASENAME, name)?.last() {
					overridden = newest;
					break;
				}
			}
			overridden
		}
	};
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = below;

	// An existing successor now overrides this verb. Otherwise, insert
	// as the newest definition and redirect subtype override chains.
	let mut relinked = vec![];
	let after = own.get(at).copied();
	match after {
		Some(above) => relinked.push(above),
		None => {
			for sub in 0..runtime_tables::type_count()? {
				if sub == type_id || !type_and_ancestors(sub).contains(&type_id) {
					continue;
				}
				for proc in staged.own(sub, type_fields::VERBS)? {
					let calls_below = staged
						.entry(proc)
						.is_some_and(|e| e.0[ProcEntryWords::PATH_BASENAME] == name && e.0[ProcEntryWords::NEXT_OVERRIDE] == below);
					if below != NONE && calls_below && !relinked.contains(&proc) {
						relinked.push(proc);
					}
				}
			}
		}
	}

	for &proc in &relinked {
		if let Some(e) = staged.entries.get_mut(&proc) {
			e.0[ProcEntryWords::NEXT_OVERRIDE] = id;
		} else {
			staged.next_override.insert(proc, id);
		}
	}
	staged.entries.insert(id, *entry);
	staged.own.entry((type_id, type_fields::VERBS)).or_default().push(id);
	if let Some(after) = after {
		staged.listed_after.insert(id, after);
	}

	Ok(NewProcOnType {
		type_id,
		field: type_fields::VERBS,
		map_entries: vec![],
		relinked,
		after
	})
}

/// The keys a world proc goes under in the world's name map: its name, and
/// its path's name where that's another
fn world_keys(entry: &ProcEntryWords) -> Vec<u32> {
	let mut keys = vec![entry.0[ProcEntryWords::NAME]];
	if entry.0[ProcEntryWords::PATH_BASENAME] != keys[0] {
		keys.push(entry.0[ProcEntryWords::PATH_BASENAME]);
	}
	keys
}

/// Which of a proc's same-named static locals a scope (`proc:PATH#N`) means
fn static_number(scope: &str) -> usize {
	scope.rsplit_once('#').and_then(|(_, n)| n.parse().ok()).unwrap_or(0)
}

/// Allocate missing static locals by comparing scopes and globals used
/// in the current bytecode. Initialize their global slots.
fn new_statics(
	code: &HotReloadProc,
	proc_globals: &HashMap<Vec<u8>, Vec<u32>>,
	added: &mut AddedVars
) -> Result<HashMap<(String, Vec<u8>), u32>, String> {
	let mut out = HashMap::new();
	for local in &code.statics {
		let name = local.name.as_bytes().to_vec();
		if proc_globals.get(&name).is_some_and(|ids| ids.len() > static_number(&local.scope)) {
			continue;
		}
		let value = match (&local.value, &local.init) {
			(Some(value), _) => const_value(value).map_err(|e| format!("static local {}: {}", local.name, e))?,
			(None, Some(_)) => (0, 0),
			(None, None) => return Err(format!("static local {} has no initial value to start with", local.name))
		};
		let id = added.add(leaked_string(&local.name)?, value);
		if let (None, Some(init)) = (&local.value, &local.init) {
			added.run_once.push((id, format!("static local {}", local.name), (**init).clone()));
		}
		out.insert((local.scope.clone(), name), id);
	}
	Ok(out)
}

/// Add missing types and populate their records. Reuse types left by an
/// earlier failed reload. Return the number added.
pub fn add_types(new_types: &[NewType]) -> Result<usize, Vec<String>> {
	if new_types.is_empty() {
		return Ok(0);
	}
	runtime_tables::available().map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;
	code_changed();

	let mut added = 0;
	for new in new_types {
		if find_type(&new.path).is_ok() {
			continue;
		}
		// Its subtypes can't be added without it
		add_type(new).map_err(|e| vec![format!("{}: {}", new.path, e)])?;
		added += 1;
	}
	Ok(added)
}

fn add_type(new: &NewType) -> Result<(), String> {
	// Everything is worked out before the type exists
	let parent = find_type(&new.parent)?;
	let mut writes = vec![];
	for (field, value) in &new.fields {
		writes.extend(builtin_writes(field, value).map_err(|e| format!("{}: {}", field, e))?);
	}
	let id = runtime_tables::add_type(parent, leaked_string(&new.path)?)?;
	write_builtins(id, &writes)
}

/// A change to a type's table entry that sets a built-in var.
#[derive(Clone, Debug)]
enum BuiltinWrite {
	Word(usize, u32),
	Half(usize, u16),
	Byte(usize, u8),
	/// Bits of a word: offset, mask, bits
	Bits(usize, u32, u32),
	/// An entry of the type's list of built-in values: the var name's string
	/// id, and the value as bytecode has it
	Listed(u32, Vec<u32>),
	/// A mob type's `key` (a string id), `sight`, `see_in_dark` or
	/// `see_invisible`, in the mob table
	Mob(&'static str, u32)
}

/// Built-in variable storage, determined by comparing probe types
/// (see `type_fields`).
fn builtin_writes(var: &str, value: &ConstValue) -> Result<Vec<BuiltinWrite>, String> {
	use BuiltinWrite::*;
	let number = || match value {
		ConstValue::Number(n) => Ok(*n),
		ConstValue::Null => Ok(0.0),
		_ => Err("expected a number".to_owned())
	};
	let string = || match value {
		ConstValue::Null => Ok(NONE),
		ConstValue::String(text) => leaked_string(text),
		_ => Err("expected text".to_owned())
	};
	let bits = |offset: usize, mask: u32, on: bool| Bits(offset, mask, if on { mask } else { 0 });
	Ok(match var {
		"name" => vec![Word(type_fields::NAME, string()?)],
		"desc" => vec![Word(type_fields::DESC, string()?)],
		"icon_state" => vec![Word(type_fields::ICON_STATE, string()?)],
		"text" => vec![Word(type_fields::TEXT, string()?)],
		"suffix" => vec![Word(type_fields::SUFFIX, string()?)],
		"maptext" => vec![Word(type_fields::MAPTEXT, string()?)],
		"icon" => vec![Word(type_fields::ICON, match value {
			ConstValue::Null => NONE,
			ConstValue::Resource(file) => AssembleEnv
				.get_resource_index(file)
				.ok_or_else(|| format!("couldn't find the file {}", file))?,
			_ => return Err("expected a resource file".to_owned())
		})],
		"layer" => vec![Word(type_fields::LAYER, number()?.to_bits())],
		"dir" => vec![Byte(type_fields::DIR, number()? as u8)],
		"maptext_x" => vec![Half(type_fields::MAPTEXT_X, number()? as i16 as u16)],
		"maptext_y" => vec![Half(type_fields::MAPTEXT_Y, number()? as i16 as u16)],
		"maptext_width" => vec![Half(type_fields::MAPTEXT_WIDTH, number()? as u16)],
		"maptext_height" => vec![Half(type_fields::MAPTEXT_HEIGHT, number()? as u16)],
		"opacity" => vec![bits(type_fields::FLAGS, 0x1, number()? != 0.0)],
		"density" => vec![bits(type_fields::FLAGS, 0x2, number()? != 0.0)],
		"mouse_drop_zone" => vec![bits(type_fields::FLAGS, 0x100, number()? != 0.0)],
		"gender" => {
			let gender = match value {
				ConstValue::String(g) if g == "neuter" => 0,
				ConstValue::String(g) if g == "male" => 1,
				ConstValue::String(g) if g == "female" => 2,
				ConstValue::String(g) if g == "plural" => 3,
				_ => return Err("has to be \"neuter\", \"male\", \"female\" or \"plural\"".to_owned())
			};
			vec![Bits(type_fields::FLAGS, 0xC0, gender << 6)]
		}
		// 1, the default, is 0
		"mouse_opacity" => vec![Bits(type_fields::FLAGS, 0x3000, ((number()? as i32 - 1) as u32 & 3) << 12)],
		"animate_movement" => {
			let bits = match number()? as i32 {
				0 => 0xC400,
				1 => 0,
				2 => 0x4000,
				3 | -1 => 0x8000,
				_ => return Err("has to be -1, 0, 1, 2 or 3".to_owned())
			};
			vec![Bits(type_fields::FLAGS, 0xC400, bits)]
		}
		"appearance_flags" => {
			let flags = number()? as u32;
			if flags >= 1 << 12 {
				return Err("flags cannot be stored in the type record".to_owned());
			}
			vec![
				Bits(type_fields::FLAGS, 0x7FF << 21, (flags & 0x7FF) << 21),
				Bits(type_fields::FLAGS_HIGH, 1, flags >> 11),
			]
		}
		"alpha" | "blend_mode" | "color" | "invisibility" | "infra_luminosity" | "luminosity" | "pixel_x" | "pixel_y" | "pixel_w" | "pixel_z"
		| "plane" | "render_source" | "render_target" | "bound_x" | "bound_y" | "bound_width" | "bound_height" | "step_size" | "step_x"
		| "step_y" | "glide_size" | "screen_loc" | "vis_flags" | "mouse_drag_pointer" | "mouse_drop_pointer" | "mouse_over_pointer" | "override" => {
			let (tag, data) = const_value(value)?;
			// As `PushVal` has it: a number's two halves after its tag, or
			// the top byte of the data in the tag word
			let words = if tag == 0x2A {
				vec![tag, data >> 16, data & 0xFFFF]
			} else {
				vec![tag | ((data & 0xFF_0000) >> 8), data & 0xFFFF]
			};
			vec![Listed(leaked_string(var)?, words)]
		}
		"key" => vec![Mob("key", string()?)],
		"sight" => vec![Mob("sight", number()? as u32)],
		"see_in_dark" => vec![Mob("see_in_dark", number()? as u32)],
		"see_invisible" => vec![Mob("see_invisible", number()? as u32)],
		_ => return Err("built-in variable cannot be changed by hot reload".to_owned())
	})
}

/// Sets built-in vars on a type.
fn write_builtins(type_id: u32, writes: &[BuiltinWrite]) -> Result<(), String> {
	use runtime_tables::{set_type_byte, set_type_field, set_type_half, type_field};
	for write in writes {
		match write {
			BuiltinWrite::Word(offset, word) => set_type_field(type_id, *offset, *word)?,
			BuiltinWrite::Half(offset, half) => set_type_half(type_id, *offset, *half)?,
			BuiltinWrite::Byte(offset, byte) => set_type_byte(type_id, *offset, *byte)?,
			BuiltinWrite::Bits(offset, mask, bits) => {
				let word = type_field(type_id, *offset)?;
				set_type_field(type_id, *offset, (word & !mask) | bits)?;
			}
			BuiltinWrite::Listed(name, value) => {
				// The others, then this one
				let list = runtime_tables::misc_words(type_field(type_id, type_fields::BUILTIN_VALUES)?).unwrap_or_default();
				let mut words = vec![];
				let mut at = 0;
				while at < list.len() {
					let len = if list.get(at + 1).is_some_and(|tag| tag & 0xFF == 0x2A) { 4 } else { 3 };
					let entry = &list[at..(at + len).min(list.len())];
					if entry[0] != *name {
						words.extend_from_slice(entry);
					}
					at += len;
				}
				words.push(*name);
				words.extend_from_slice(value);
				set_type_field(type_id, type_fields::BUILTIN_VALUES, runtime_tables::intern_misc(&words)?)?;
			}
			BuiltinWrite::Mob(var, value) => runtime_tables::set_mob_type(type_id, |mob| match *var {
				"key" => mob.key = *value,
				"sight" => mob.sight = *value,
				"see_in_dark" => mob.see_in_dark = *value as u8,
				_ => mob.see_invisible = *value as u8
			})?
		}
	}
	Ok(())
}

/// Convert a constant to tag/data words. The destination retains any
/// string reference permanently.
fn const_value(value: &ConstValue) -> Result<(u32, u32), String> {
	Ok(match value {
		ConstValue::Null => (0, 0),
		ConstValue::Number(n) => (0x2A, n.to_bits()),
		ConstValue::String(text) => (0x06, leaked_string(text)?),
		ConstValue::Path(path) => {
			let (tag, data) = AssembleEnv
				.get_type(path)
				.ok_or_else(|| format!("there's no type {} in the running world", path))?;
			(tag as u32, data)
		}
		ConstValue::Resource(file) => (
			0x0C,
			AssembleEnv
				.get_resource_index(file)
				.ok_or_else(|| format!("couldn't find the file {}", file))?
		)
	})
}

/// Intern a string and retain its reference permanently.
fn leaked_string(text: &str) -> Result<u32, String> {
	let string = StringRef::new(text).map_err(|_| format!("couldn't make the string {}", text))?;
	let id = string.get_id().0;
	std::mem::forget(string);
	Ok(id)
}

fn prepare_new_var(new: &NewVar, added: &mut AddedVars, type_vars: &mut TypeVars, pending: &mut Pending) -> Result<(), String> {
	const STATIC: u32 = 1;
	let fail = |e: String| format!("{}/var/{}: {}", new.type_path, new.var, e);

	let id = added.next_id();
	let value = const_value(&new.value).map_err(fail)?;
	let name = leaked_string(&new.var).map_err(fail)?;
	type_vars.new_names.insert(id, name);
	if let Some(init) = &new.init {
		added.run_once.push((id, format!("{}/var/{}", new.type_path, new.var), (**init).clone()));
	}

	if !new.type_path.is_empty() {
		let type_id = find_type(&new.type_path).map_err(fail)?;
		for sub in type_vars.subtree(type_id).map_err(fail)? {
			let declared = type_vars.declared(sub).map_err(fail)?.clone();
			let names: Vec<u32> = declared.chunks(2).map(|pair| type_vars.name_of(pair[0])).collect();
			match names.binary_search(&name) {
				Ok(_) => return Err(fail(format!("variable already exists on type {}", sub))),
				Err(at) => {
					type_vars.declared(sub).map_err(fail)?.splice(at * 2..at * 2, [id, new.flags]);
				}
			}

			if new.flags & STATIC == 0 {
				let table = type_vars.table(sub).map_err(fail)?;
				let at = table.chunks(4).position(|entry| entry[0] >= name).unwrap_or(table.len() / 4);
				if table.get(at * 4) == Some(&name) {
					return Err(fail(format!("variable already exists on type {}", sub)));
				}
				table.splice(at * 4..at * 4, [name, id, value.0, value.1]);
			}
		}
	}

	if new.type_path.is_empty() || new.flags & STATIC != 0 {
		pending.globals.insert((new.type_path.clone(), new.var.as_bytes().to_vec()), id);
	}

	added.vars.push(PreparedNewVar { id, name, value });
	Ok(())
}

/// Update variable declaration flags on a type and its subtypes.
/// Static variables leave instance tables and become available as globals
/// to code in this reload.
fn prepare_redeclared(
	var: &Redeclared,
	added: &mut AddedVars,
	type_vars: &mut TypeVars,
	pending: &mut Pending,
	global_values: &mut Vec<(u32, (u32, u32))>
) -> Result<(), String> {
	const STATIC: u32 = 1;
	let fail = |e: String| format!("{}/var/{}: {}", var.type_path, var.var, e);

	// A global const's new value, set now: the one var of that name
	if var.type_path.is_empty() {
		let ids = variable_ids(var.var.as_bytes());
		let [id] = ids[..] else {
			return Err(fail(format!("expected one matching runtime variable; found {}", ids.len())));
		};
		if let Some(init) = &var.init {
			added.run_once.push((id, format!("/var/{}", var.var), (**init).clone()));
		} else if let Some(value) = &var.value {
			global_values.push((id, const_value(value).map_err(fail)?));
		}
		return Ok(());
	}

	let type_id = find_type(&var.type_path).map_err(fail)?;
	let name = StringRef::new(&var.var)
		.map_err(|_| fail("couldn't make its name".to_owned()))?
		.get_id()
		.0;
	let declared = type_vars.declared(type_id).map_err(fail)?.clone();
	let id = declared
		.chunks(2)
		.map(|pair| pair[0])
		.find(|&id| type_vars.name_of(id) == name)
		.ok_or_else(|| fail("variable not found in the running world".to_owned()))?;

	for sub in type_vars.subtree(type_id).map_err(fail)? {
		if let Some(pair) = type_vars.declared(sub).map_err(fail)?.chunks_mut(2).find(|pair| pair[0] == id) {
			pair[1] = var.flags;
		}
		let table = type_vars.table(sub).map_err(fail)?;
		let at = table.chunks(4).position(|entry| entry[0] >= name).unwrap_or(table.len() / 4);
		let there = table.get(at * 4) == Some(&name);
		if var.flags & STATIC != 0 {
			if there {
				table.drain(at * 4..at * 4 + 4);
			}
		} else if !there {
			// The defaults follow
			table.splice(at * 4..at * 4, [name, id, 0, 0]);
		}
	}

	if var.flags & STATIC != 0 {
		pending.globals.insert((var.type_path.clone(), var.var.as_bytes().to_vec()), id);
		if let Some(init) = &var.init {
			added.run_once.push((id, format!("{}/var/{}", var.type_path, var.var), (**init).clone()));
		} else if let Some(value) = &var.value {
			global_values.push((id, const_value(value).map_err(fail)?));
		}
	}
	Ok(())
}

fn prepare_var_default(default: &HotReloadVarDefault, type_vars: &mut TypeVars) -> Result<(), String> {
	let fail = |e: String| format!("var {}: {}", default.var, e);

	let value = const_value(&default.value).map_err(fail)?;
	let key = StringRef::new(&default.var)
		.map_err(|_| fail("couldn't make its name".to_owned()))?
		.get_id()
		.0;

	for path in &default.types {
		let type_id = find_type(path).map_err(fail)?;
		let table = type_vars.table(type_id).map_err(fail)?;
		let Some(entry) = table.chunks_mut(4).find(|entry| entry[0] == key) else {
			return Err(fail(format!("variable not found on runtime type {}", path)));
		};
		entry[2] = value.0;
		entry[3] = value.1;
	}
	Ok(())
}

/// Commit a prepared reload. Temporarily remove affected breakpoints,
/// then restore them at their remapped source lines and update their metadata.
pub fn apply<K: Ord + std::hash::Hash + Copy, T>(
	prepared: Prepared,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	moved_breakpoints: &mut MovedBreakpoints<K>,
	warnings: &mut Vec<String>
) {
	code_changed();
	// The id each new var really got, if it got one
	let mut got = HashMap::new();
	for new in &prepared.new_vars {
		let id = match runtime_tables::add_variable(new.name, new.value.0 as u8, new.value.1) {
			Ok(id) => {
				if id != new.id {
					warnings.push(format!("new variable ID mismatch: got {}, expected {}", id, new.id));
				}
				Some(id)
			}
			Err(e) => {
				warnings.push(format!("couldn't add a var: {}", e));
				None
			}
		};
		got.insert(new.id, id);
	}
	// Register new globals by their actual IDs for expression lookup.
	// Existing globals retain their IDs.
	let globals = prepared
		.new_globals
		.iter()
		.filter_map(|(name, id)| Some((name.clone(), got.get(id).copied().unwrap_or(Some(*id))?)));
	ADDED_GLOBALS.lock().unwrap().extend(globals);

	let procs_before = runtime_tables::proc_count();

	for removal in &prepared.removed_procs {
		if let Some(edit) = &removal.on_world {
			if let Err(e) = edit_world_names(edit.clone()) {
				warnings.push(format!("could not unlink world proc: {}", e));
			}
		}
		if removal.type_id == NONE {
			continue;
		}
		let procs = runtime_tables::type_field(removal.type_id, removal.field).unwrap_or(NONE);
		let mut list = runtime_tables::misc_words(procs).unwrap_or_default();
		list.retain(|&proc| proc != removal.id);
		// (BYOND makes empty misc entries poorly, and has none for no procs)
		let list = if list.is_empty() { Ok(NONE) } else { runtime_tables::intern_misc(&list) };
		match list {
			Ok(list) => {
				let _ = runtime_tables::set_type_field(removal.type_id, removal.field, list);
			}
			Err(e) => warnings.push(format!("could not unlink proc from type list: {}", e))
		}
		for (sub, key, procs) in &removal.map_entries {
			if let Err(e) = runtime_tables::set_named_procs(*sub, *key, procs) {
				warnings.push(format!("could not unlink proc from type name map: {}", e));
			}
		}
		for &proc in &removal.relinked {
			runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, removal.next);
		}
	}

	// Install all entries before relinking `..()` chains.
	// Force fresh IDs when re-adding removed procs.
	for new in &prepared.new_procs {
		match runtime_tables::append_proc(&new.entry) {
			Ok(id) if id.0 != new.id => {
				// Code in this reload calls it by the id it expected
				warnings.push(format!("new proc ID mismatch: got {}, expected {}", id.0, new.id));
			}
			Ok(_) => {}
			Err(e) => warnings.push(format!("couldn't add a proc: {}", e))
		}
	}

	for new in &prepared.new_procs {
		let id = new.id;
		mark_reloaded(id, Reloaded::Added);
		if let Some(on_type) = &new.on_type {
			let procs = runtime_tables::type_field(on_type.type_id, on_type.field).unwrap_or(NONE);
			let mut list = runtime_tables::misc_words(procs).unwrap_or_default();
			match on_type.after.and_then(|after| list.iter().position(|&p| p == after)) {
				Some(at) if !list.contains(&id) => list.insert(at + 1, id),
				_ => add_to_list(&mut list, on_type.field, id)
			}
			match runtime_tables::intern_misc(&list) {
				Ok(list) => {
					let _ = runtime_tables::set_type_field(on_type.type_id, on_type.field, list);
				}
				Err(e) => warnings.push(format!("couldn't add a proc to its type's list: {}", e))
			}

			for (sub, key, procs) in &on_type.map_entries {
				if let Err(e) = runtime_tables::set_named_procs(*sub, *key, procs) {
					warnings.push(format!("couldn't add a proc to a type's proc names: {}", e));
				}
			}

			for &proc in &on_type.relinked {
				runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, id);
			}
		}

		if let Some(edit) = &new.on_world {
			if let Err(e) = edit_world_names(edit.clone()) {
				warnings.push(format!("couldn't add a proc to the world's procs: {}", e));
			}
		}

		if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			let (ptr, _) = unsafe { proc.bytecode_mut_ptr() };
			auxtools::debug::set_local_names(ptr, new.local_names.clone());
		}
	}
	for (expected, instance, entry) in &prepared.instances {
		let added =
			runtime_tables::add_proc(entry).and_then(|init| runtime_tables::add_instance(runtime_tables::Instance { init: init.0, ..*instance }));
		match added {
			Ok(id) if id != *expected => warnings.push(format!("prefab ID mismatch: got {}, expected {}", id, expected)),
			Ok(_) => {}
			Err(e) => warnings.push(format!("couldn't add a prefab: {}", e))
		}
	}
	// (BYOND gives a proc the id of another with the same entry, which is
	// as good)
	for (expected, entry) in &prepared.in_lists {
		match runtime_tables::add_proc(entry).and_then(runtime_tables::add_in_list) {
			Ok(index) if index != *expected => warnings.push(format!("input list index mismatch: got {}, expected {}", index, expected)),
			Ok(_) => {}
			Err(e) => warnings.push(format!("couldn't add an `in` list: {}", e))
		}
	}
	for init in &prepared.init_procs {
		let id = match &init.proc {
			None => NONE,
			Some((entry, local_names)) => {
				// BYOND gives a type the same init proc as another with the
				// same code, and nothing else refers to it by id
				let id = match runtime_tables::add_proc(entry) {
					Ok(id) => id,
					Err(e) => {
						warnings.push(format!("couldn't add an init proc: {}", e));
						continue;
					}
				};
				if let Some(proc) = Proc::from_id(id) {
					let (ptr, _) = unsafe { proc.bytecode_mut_ptr() };
					auxtools::debug::set_local_names(ptr, local_names.clone());
				}
				id.0
			}
		};
		let _ = runtime_tables::set_type_field(init.type_id, type_fields::INIT_PROC, id);
	}

	if runtime_tables::proc_count() > procs_before {
		Proc::add_to_table(raw_types::procs::ProcId(procs_before));
	}
	renumber(&prepared.removed_procs, &prepared.new_procs);

	let mut renames = prepared.renames;
	for p in prepared.code {
		renames.extend(p.rename);
		apply_code(p, breakpoints, moved_breakpoints, warnings);
	}
	for r in renames {
		if let Err(e) = rename(r) {
			warnings.push(format!("could not update dynamic lookup for renamed proc: {}", e));
		}
	}
	for (proc, word, value) in prepared.entry_words {
		runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), word, value);
	}
	for (type_id, field, words) in prepared.type_fields {
		let _ = runtime_tables::set_type_field(type_id, field, words);
	}
	for (type_id, writes) in &prepared.builtin_writes {
		if let Err(e) = write_builtins(*type_id, writes) {
			warnings.push(format!("couldn't set a built-in var: {}", e));
		}
	}
	for &type_id in &prepared.appearances {
		let _ = runtime_tables::forget_type_appearance(type_id);
	}

	for (id, (tag, data)) in prepared.global_values {
		if let Err(e) = runtime_tables::set_global_words(id, tag as u8, data) {
			warnings.push(format!("couldn't set a static var's value: {}", e));
		}
	}

	// Run computed initializers after installing the remaining changes,
	// using the expression stub.
	for (id, name, bytecode) in prepared.run_once {
		let Some(stub) = Proc::find("/proc/auxtools_expr_stub") else {
			warnings.push(format!("{}: missing /proc/auxtools_expr_stub for initializer evaluation", name));
			continue;
		};
		stub.set_bytecode(bytecode);
		match stub.call(&[]) {
			Ok(value) => {
				if let Err(e) = runtime_tables::set_global_value(id, value) {
					warnings.push(format!("{}: {}", name, e));
				}
			}
			Err(e) => warnings.push(format!("{}: initializer evaluation failed: {}", name, e.message))
		}
	}
}

fn apply_code<K: Ord + std::hash::Hash + Copy, T>(
	p: PreparedCode,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	moved_breakpoints: &mut MovedBreakpoints<K>,
	warnings: &mut Vec<String>
) {
	let (old_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
	let old_bytecode = original_bytecode(&p.proc);
	mark_reloaded(p.proc.id.0, if p.lines.is_some() { Reloaded::Lines } else { Reloaded::Code });

	// Retain old local names for frames still running pre-reload bytecode.
	if !auxtools::debug::has_local_names(old_ptr) {
		let names = auxtools::debug::local_names_for(&p.proc, old_ptr);
		auxtools::debug::set_local_names(old_ptr, names);
	}

	let hooked: Vec<(u32, u32, Option<BTreeMap<K, T>>)> = hooked_offsets(&p.proc)
		.into_iter()
		.filter_map(|offset| {
			let line = line_at(&old_bytecode, offset)?;
			let known = breakpoints.remove(&(p.proc.id, offset as u16));
			let _ = unhook_instruction(&p.proc, offset);
			Some((offset, line, known))
		})
		.collect();

	let local_names = p.local_names.unwrap_or_else(|| auxtools::debug::local_names_for(&p.proc, old_ptr));
	auxtools::remember_bytecode(old_ptr, &old_bytecode);

	runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::BYTECODE, p.bytecode_id);
	if let Some(locals) = p.locals_id {
		runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::LOCALS, locals);
	}
	if let Some(params) = p.params_id {
		runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::PARAMETERS, params);
	}
	for (word, value) in p.settings {
		runtime_tables::set_proc_entry_word(p.proc.id, word, value);
	}

	let (new_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
	auxtools::debug::set_local_names(new_ptr, local_names);

	for (old_offset, line, known) in hooked {
		let line = match &p.lines {
			Some(lines) => map_line(line, lines),
			None => line
		};

		// Preserve each client's original offset, including prior reload
		// mappings.
		let mut known_as: Vec<(raw_types::procs::ProcId, u32, K)> = moved_breakpoints
			.iter()
			.filter(|((proc, ..), at)| *proc == p.proc.id && **at == old_offset)
			.map(|(key, _)| *key)
			.collect();
		for key in &known_as {
			moved_breakpoints.remove(key);
		}
		for client in known.iter().flat_map(BTreeMap::keys) {
			if !known_as.iter().any(|(.., other)| other == client) {
				known_as.push((p.proc.id, old_offset, *client));
			}
		}

		let Some(offset) = offset_of_line(&p.bytecode, line) else {
			warnings.push(format!(
				"{}: removed breakpoint on line {}: no executable code after reload",
				p.name, line
			));
			continue;
		};

		if hook_instruction(&p.proc, offset).is_err() {
			warnings.push(format!("{}: could not restore breakpoint on line {}", p.name, line));
			continue;
		}

		if let Some(known) = known {
			breakpoints.insert((p.proc.id, offset as u16), known);
		}
		for key in known_as {
			if key.1 != offset {
				moved_breakpoints.insert(key, offset);
			}
		}
	}
}

/// Adapt debug file and line instructions to the running world.
fn adapt_debug_info(nodes: &[Node], original: &[u32]) -> Vec<Node> {
	let is_debug_info = |node: &Node| matches!(node, Node::Instruction(Instruction::DbgFile(_) | Instruction::DbgLine(_), _));

	// DreamMaker only writes them for worlds built with DEBUG. A new proc has
	// them if the world's procs do.
	if original.is_empty() && world_has_debug_info() {
		return nodes.to_vec();
	}
	if original.first() != Some(&Opcode::DbgFile.word()) || original.len() < 2 {
		return nodes.iter().filter(|node| !is_debug_info(node)).cloned().collect();
	}

	// Preserve DreamMaker's debug file names, which can differ from
	// project-relative paths for nested includes.
	let old_name = unsafe { StringRef::from_id(raw_types::strings::StringId(original[1])) }.data().to_vec();
	let mut suffix = b"/".to_vec();
	suffix.extend_from_slice(&old_name);

	nodes
		.iter()
		.map(|node| match node {
			Node::Instruction(Instruction::DbgFile(name), ()) if name.0 != old_name && name.0.ends_with(&suffix) => {
				Node::Instruction(Instruction::DbgFile(dmasm::DMString(old_name.clone())), ())
			}
			_ => node.clone()
		})
		.collect()
}

// Changes to the world's proc names waiting for them to be built (see
// `edit_world_names`)
static WAITING_WORLD_EDITS: Mutex<Vec<WorldEdit>> = Mutex::new(Vec::new());

/// Generation for code and macro changes that invalidate compiled evaluations.
static CODE_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Invalidate compiled evaluation caches after code or macro changes.
pub fn code_changed() {
	CODE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Current code and macro generation.
pub fn code_generation() -> u32 {
	CODE_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// The global vars that reloads have added to this world, by name, with
/// their var ids
static ADDED_GLOBALS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

/// Look up globals added by reload. They are absent from `global.vars`,
/// which BYOND constructs at load time, so expressions use their IDs.
pub fn added_global(name: &str) -> Option<u32> {
	ADDED_GLOBALS.lock().unwrap().get(name).copied()
}

/// The names of the global vars hot reloads have added
pub fn added_globals() -> Vec<String> {
	ADDED_GLOBALS.lock().unwrap().keys().cloned().collect()
}

/// What reloads have done to procs in this world, by the procs' ids
static RELOADED: Mutex<BTreeMap<u32, Reloaded>> = Mutex::new(BTreeMap::new());

/// Most significant reload change to a proc since world load.
pub fn reloaded(id: u32) -> Option<Reloaded> {
	RELOADED.lock().unwrap().get(&id).copied()
}

/// Record a proc reload without downgrading its previous change classification.
fn mark_reloaded(id: u32, how: Reloaded) {
	let mut reloaded = RELOADED.lock().unwrap();
	let was = reloaded.entry(id).or_insert(how);
	*was = (*was).max(how);
}

/// Debugger helper overrides remain most derived; insert reloaded world
/// procs below them.
static PINNED_WORLD_PROCS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn pin_world_proc(id: u32) {
	PINNED_WORLD_PROCS.lock().unwrap().push(id);
}

/// Update the world proc name map after BYOND builds it. The map is built
/// after global initialization and before `world/New`; earlier edits would
/// be overwritten. Queue them until the next proc call after construction.
fn edit_world_names(edit: WorldEdit) -> Result<(), String> {
	if runtime_tables::world_names_built()? {
		return edit_built_world_names(&edit);
	}
	WAITING_WORLD_EDITS.lock().unwrap().push(edit);
	auxtools::at_proc_calls(make_waiting_world_edits);
	Ok(())
}

fn make_waiting_world_edits() -> bool {
	if !runtime_tables::world_names_built().unwrap_or(false) {
		return false;
	}
	for edit in std::mem::take(&mut *WAITING_WORLD_EDITS.lock().unwrap()) {
		if let Err(e) = edit_built_world_names(&edit) {
			eprintln!("auxtools: couldn't change the world's procs: {}", e);
		}
	}
	true
}

fn edit_built_world_names(edit: &WorldEdit) -> Result<(), String> {
	let set_next = |proc: u32, next: u32| runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, next);
	let next_of = |proc: u32| runtime_tables::proc_entry(raw_types::procs::ProcId(proc)).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]);
	match edit {
		WorldEdit::Add { id, keys, below, above } => {
			let pinned = PINNED_WORLD_PROCS.lock().unwrap().clone();
			set_next(*id, *below);
			for (k, &key) in keys.iter().enumerate() {
				// Most derived first
				let mut procs = runtime_tables::world_named_procs(key)?;
				procs.retain(|proc| proc != id);
				let at = match above.and_then(|above| procs.iter().position(|&p| p == above)) {
					Some(at) => at + 1,
					None => procs.iter().take_while(|proc| pinned.contains(proc)).count()
				};
				if k == 0 {
					if let Some(&over) = at.checked_sub(1).and_then(|i| procs.get(i)) {
						set_next(over, *id);
					}
				}
				procs.insert(at, *id);
				runtime_tables::set_world_named_procs(key, &procs)?;
			}
		}
		WorldEdit::Remove { id, keys } => {
			let next = next_of(*id);
			for &key in keys {
				let mut procs = runtime_tables::world_named_procs(key)?;
				for &proc in &procs {
					if proc != *id && next_of(proc) == *id {
						set_next(proc, next);
					}
				}
				procs.retain(|proc| proc != id);
				runtime_tables::set_world_named_procs(key, &procs)?;
			}
		}
	}
	Ok(())
}

/// Return staged definitions of a type's proc in source order.
/// Proc override chains link through `..()`; verb definitions are stored
/// newest first and resolved by name.
fn own_definitions(staged: &StagedProcs, type_id: u32, field: usize, word: usize, name: u32) -> Result<Vec<u32>, String> {
	let own: Vec<u32> = staged
		.own(type_id, field)?
		.into_iter()
		.filter(|&p| staged.entry(p).is_some_and(|e| e.0[word] == name))
		.collect();
	if field == type_fields::VERBS {
		return Ok(own.into_iter().rev().collect());
	}
	let next = |p: u32| staged.entry(p).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]);
	// The last is the one no other's `..()` goes to
	let mut chain = vec![];
	let mut at = own.iter().copied().find(|&p| !own.iter().any(|&q| next(q) == p));
	while let Some(p) = at {
		chain.push(p);
		at = Some(next(p)).filter(|n| own.contains(n) && !chain.contains(n));
	}
	chain.reverse();
	Ok(chain)
}

/// Renumber affected override paths to match source order after additions
/// and removals.
fn renumber(removed: &[PreparedRemoval], added: &[PreparedNewProc]) {
	let mut paths: Vec<String> = vec![];
	let removed_ids: HashSet<u32> = removed.iter().map(|r| r.id).collect();
	let added_ids: HashSet<u32> = added.iter().map(|a| a.id).collect();
	for id in removed.iter().map(|r| r.id).chain(added_ids.iter().copied()) {
		if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			if !paths.contains(&proc.path) {
				paths.push(proc.path.clone());
			}
		}
	}
	for path in paths {
		let mut procs: Vec<Proc> = Proc::overrides(path.as_str())
			.into_iter()
			.filter(|p| !removed_ids.contains(&p.id.0) && !added_ids.contains(&p.id.0))
			.collect();
		let mut new: Vec<&PreparedNewProc> = added.iter().filter(|a| strip_path(&a.place.path) == path).collect();
		new.sort_by_key(|a| a.place.override_id);
		for a in new {
			if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(a.id)) {
				let at = (a.place.override_id as usize).min(procs.len());
				procs.insert(at, proc);
			}
		}
		Proc::set_overrides(path.as_str(), procs);
	}
}

/// Copy the expression stub's proc entry for new procs, falling back to
/// stddef.dm's `/icon/proc/Width`. The launcher's stub is based on Width.
fn proc_template() -> Result<ProcEntryWords, String> {
	Proc::find("/proc/auxtools_expr_stub")
		.or_else(|| Proc::find("/icon/proc/Width"))
		.and_then(|proc| runtime_tables::proc_entry(proc.id))
		.ok_or_else(|| "couldn't find a proc entry to copy (/proc/auxtools_expr_stub or /icon/proc/Width)".to_owned())
}

// Worked out once per world
static WORLD_HAS_DEBUG_INFO: Mutex<Option<bool>> = Mutex::new(None);

/// Detect a DEBUG build from the world's original procs' initial DbgFile.
/// The expression stub may have been recompiled, so it is not a reliable probe.
fn world_has_debug_info() -> bool {
	let mut known = WORLD_HAS_DEBUG_INFO.lock().unwrap();
	*known.get_or_insert_with(|| {
		(0..)
			.map_while(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.any(|proc| original_bytecode(&proc).first() == Some(&Opcode::DbgFile.word()))
	})
}

#[shutdown]
fn forget_world() {
	ADDED_GLOBALS.lock().unwrap().clear();
	RELOADED.lock().unwrap().clear();
	*WORLD_HAS_DEBUG_INFO.lock().unwrap() = None;
	WAITING_WORLD_EDITS.lock().unwrap().clear();
	PINNED_WORLD_PROCS.lock().unwrap().clear();
}

/// `bytecode` with its DbgLines renumbered, or `None` when none changes.
fn remap_lines(bytecode: &[u32], lines: &[(u32, u32)]) -> Option<Vec<u32>> {
	let scanned = dmasm::scan(bytecode).ok()?;
	let mut out = bytecode.to_vec();
	let mut changed = false;

	for ins in scanned.instructions {
		if ins.opcode == Opcode::DbgLine {
			let at = ins.offset as usize + 1;
			let line = map_line(out[at], lines);
			changed |= line != out[at];
			out[at] = line;
		}
	}

	changed.then_some(out)
}

/// Remap a line using the nearest preceding mapping when no exact entry exists.
fn map_line(line: u32, lines: &[(u32, u32)]) -> u32 {
	let mut sorted = lines.to_vec();
	sorted.sort_unstable();

	let known = match sorted.binary_search_by_key(&line, |(old, _)| *old) {
		Ok(i) => return sorted[i].1,
		Err(0) => sorted.first(),
		Err(i) => sorted.get(i - 1)
	};

	match known {
		Some(&(old, new)) => (line as i64 + new as i64 - old as i64) as u32,
		None => line
	}
}

/// The line the instruction at `offset` is on.
fn line_at(bytecode: &[u32], offset: u32) -> Option<u32> {
	let mut line = None;
	for ins in dmasm::scan(bytecode).ok()?.instructions {
		if ins.offset > offset {
			break;
		}
		if ins.opcode == Opcode::DbgLine {
			line = ins.bytecode.get(1).copied();
		}
	}
	line
}

/// Resolve a line breakpoint to the instruction after its DbgLine,
/// matching the server's `Offset` request.
fn offset_of_line(bytecode: &[u32], line: u32) -> Option<u32> {
	let instructions = dmasm::scan(bytecode).ok()?.instructions;
	let at = instructions
		.iter()
		.position(|ins| ins.opcode == Opcode::DbgLine && ins.bytecode.get(1) == Some(&line))?;
	instructions.get(at + 1).map(|ins| ins.offset)
}

fn describe_assemble_error(e: &dmasm::assembler::AssembleError) -> String {
	use dmasm::assembler::AssembleError::*;
	let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
	match e {
		ProcNotFound(path) => format!("proc not found: {} in the running world", path),
		StringNotFound(s) => format!("couldn't make the string {:?}", text(s)),
		VariableNameNotFound(name) => format!("variable not found in the running world: {}", text(name)),
		TypeNotFound(path) => format!("type not found in the running world: {}", path),
		ResourceNotFound(path) => format!("couldn't find the file {}", path),
		InstanceNotFound(path) => format!("prefab instance not found: {}{{...}}", path),
		LabelNotFound(label) => format!("undefined label in compiled code: {}", label)
	}
}

/// Every variable id whose name is `name`.
fn variable_ids(name: &[u8]) -> Vec<u32> {
	let Ok(string) = StringRef::from_raw(name) else {
		return vec![];
	};
	let id = string.get_id();

	unsafe {
		let entries = *raw_types::funcs::VARIABLE_NAME_ENTRIES;
		(0..*raw_types::funcs::VARIABLE_NAME_COUNT)
			.filter(|&i| *entries.add(i as usize) == id)
			.collect()
	}
}

/// Group referenced globals by name, sorting IDs within each group.
/// Same-named static locals have IDs in declaration order.
fn globals_in(bytecode: &[u32]) -> HashMap<Vec<u8>, Vec<u32>> {
	let mut out: HashMap<Vec<u8>, Vec<u32>> = HashMap::new();
	let (nodes, _) = dmasm::disassembler::disassemble(bytecode, &mut LenientDisassembleEnv);

	for node in &nodes {
		let Node::Instruction(ins, _) = node else { continue };
		for var in ins.variables() {
			for global in var.globals() {
				let Some(id) = global.id else { continue };
				let ids = out.entry(global.name.0.clone()).or_default();
				if let Err(at) = ids.binary_search(&id) {
					ids.insert(at, id);
				}
			}
		}
	}

	out
}

/// Index global variable users by disassembling all procs.
/// Build lazily, only when a name is ambiguous.
#[derive(Default)]
struct WorldGlobals(Option<HashMap<Vec<u8>, Vec<(u32, String)>>>);

impl WorldGlobals {
	fn users(&mut self, name: &[u8]) -> &[(u32, String)] {
		let map = self.0.get_or_insert_with(|| {
			let mut map: HashMap<Vec<u8>, Vec<(u32, String)>> = HashMap::new();
			let mut id = 0;
			while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
				for (name, ids) in globals_in(&original_bytecode(&proc)) {
					map.entry(name).or_default().extend(ids.into_iter().map(|id| (id, proc.path.clone())));
				}
				id += 1;
			}
			map
		});
		map.get(name).map_or(&[], |users| users.as_slice())
	}
}

/// Assembly environment combining runtime lookups with the existing proc's
/// global variable IDs.
struct ReloadEnv<'a> {
	proc_globals: HashMap<Vec<u8>, Vec<u32>>,
	world_globals: &'a mut WorldGlobals,
	/// What this reload adds
	pending: &'a Pending,
	/// The proc's static locals the running world doesn't have yet, by scope
	/// and name
	new_statics: HashMap<(String, Vec<u8>), u32>,
	instances: &'a mut StagedInstances,
	/// Lookup failure details more specific than the assembler error.
	error: Option<String>
}

impl dmasm::assembler::AssembleEnv for ReloadEnv<'_> {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		AssembleEnv.get_string_index(string)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		AssembleEnv.get_variable_name_index(name)
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		if let Some(&id) = self.pending.procs.get(&strip_path(path)) {
			return Some(id);
		}
		AssembleEnv.get_proc_index(path)
	}

	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		// A reference to a proc this reload adds
		if let Some(&id) = self.pending.procs.get(&strip_path(path)) {
			return Some((raw_types::values::ValueTag::ProcRef as u8, id));
		}
		AssembleEnv.get_type(path)
	}

	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		AssembleEnv.get_resource_index(path)
	}

	fn get_instance(&mut self, path: &str, init: &[u32]) -> Option<u32> {
		let Some(value) = AssembleEnv.get_type(path) else {
			self.error = Some(format!("there's no type {} in the running world", path));
			return None;
		};
		match self.instances.find_or_add(path, value, init) {
			Ok(id) => Some(id),
			Err(e) => {
				self.error = Some(format!("{}{{...}}: {}", path, e));
				None
			}
		}
	}

	fn get_global_index(&mut self, name: &[u8], scope: Option<&str>) -> Option<u32> {
		let shown = String::from_utf8_lossy(name).into_owned();

		// Resolve an existing static local by its proc scope and declaration
		// index.
		if let Some(scope) = scope.filter(|s| s.starts_with("proc:")) {
			let n = static_number(scope);
			let id = self.proc_globals.get(name).and_then(|ids| ids.get(n)).copied();
			let id = id.or_else(|| self.new_statics.get(&(scope.to_owned(), name.to_vec())).copied());
			if id.is_none() {
				self.error = Some(format!(
					"static local {}: cannot locate its storage in the running world; rebuild to use it",
					shown
				));
			}
			return id;
		}

		// A var this reload adds
		let key = (scope.unwrap_or_default().to_owned(), name.to_vec());
		if let Some(&id) = self.pending.globals.get(&key) {
			return Some(id);
		}

		let candidates = variable_ids(name);
		match candidates.len() {
			0 => {
				self.error = Some(format!("there's no global var {} in the running world", shown));
				return None;
			}
			1 => return Some(candidates[0]),
			_ => {}
		}

		// Prefer the same-named variable already referenced by this proc.
		if let Some([id]) = self.proc_globals.get(name).map(Vec::as_slice) {
			return Some(*id);
		}

		// Otherwise use the unique global or unique static referenced by
		// code on the declaring type.
		let users = self.world_globals.users(name);
		let ids: BTreeSet<u32> = users.iter().map(|(id, _)| *id).collect();
		if ids.len() == 1 {
			return ids.first().copied();
		}
		if let Some(ty) = scope.filter(|s| !s.is_empty()) {
			let prefix = format!("{}/", ty);
			let ids: BTreeSet<u32> = users.iter().filter(|(_, path)| path.starts_with(&prefix)).map(|(id, _)| *id).collect();
			if ids.len() == 1 {
				return ids.first().copied();
			}
		}

		self.error = Some(format!(
			"ambiguous variable: {} runtime slots named {}; no unique reference in {}",
			candidates.len(),
			shown,
			scope.filter(|s| !s.is_empty()).unwrap_or("the world")
		));
		None
	}
}

/// Disassemble to the end, retaining unnamed values as raw operands.
struct LenientDisassembleEnv;

impl dmasm::disassembler::DisassembleEnv for LenientDisassembleEnv {
	fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv.get_string_data(index)
	}

	fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv.get_variable_name(index)
	}

	fn get_proc_name(&mut self, index: u32) -> Option<String> {
		crate::DisassembleEnv.get_proc_name(index).or_else(|| Some(format!("<proc {}>", index)))
	}

	fn value_to_string_data(&mut self, tag: u32, data: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv
			.value_to_string_data(tag, data)
			.or_else(|| Some(format!("<value {:#x} {}>", tag, data).into_bytes()))
	}
}

#[cfg(test)]
mod tests {
	use super::{map_line, remap_lines};

	#[test]
	fn lines_move_with_the_closest_known_line_above() {
		let lines = [(10, 12), (14, 20)];
		assert_eq!(map_line(10, &lines), 12);
		assert_eq!(map_line(14, &lines), 20);
		assert_eq!(map_line(11, &lines), 13);
		assert_eq!(map_line(16, &lines), 22);
		assert_eq!(map_line(5, &lines), 7);
		assert_eq!(map_line(5, &[]), 5);
	}

	#[test]
	fn renumbers_dbgline_operands_only() {
		// DbgLine 10; PushInt 10; DbgLine 11; Ret
		let dbgline = dmasm::Opcode::DbgLine.word();
		let push_int = dmasm::Opcode::PushInt.word();
		let ret = dmasm::Opcode::Ret.word();
		let bytecode = [dbgline, 10, push_int, 10, dbgline, 11, ret];
		let moved = remap_lines(&bytecode, &[(10, 15), (11, 16)]).unwrap();
		assert_eq!(moved, [dbgline, 15, push_int, 10, dbgline, 16, ret]);
		assert_eq!(remap_lines(&bytecode, &[(10, 10), (11, 11)]), None);
	}
}
