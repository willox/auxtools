//! Parse source and watch files for hot reload.
//! The source worker uses `dmasm::reload` to compile changes; the adapter
//! applies them and reports the result. SpacemanDMM contexts are thread-bound,
//! so all parsing stays on this worker thread.

use std::{
	cell::{Cell, RefCell},
	collections::{HashMap, HashSet},
	ffi::{OsStr, OsString},
	path::{Path, PathBuf},
	sync::{mpsc, Arc},
	time::{Duration, Instant, SystemTime}
};

use auxtools_cli::server_types::{
	HotReloadInitProc, HotReloadLines, HotReloadNewProc, HotReloadProc, HotReloadRequest, HotReloadResource, HotReloadVarDefault, Macro, ProcRef,
	ReloadedResource, RemovedVar, ScriptRequest, SkinRequest
};
use dmasm::reload::{diff, Changes, ParseOptions, ProcKey, Snapshot};

use crate::{adapter::Input, dap::Output, resources::Resources, script::Script, skin::Skin};

/// How often to look for changed files
const INTERVAL: Duration = Duration::from_millis(500);
/// Limit polling to 1 / LOOKING_SHARE of the worker's time.
const LOOKING_SHARE: u32 = 5;
/// Allowance for Linux file timestamps lagging the clock by a coarse timer
/// tick.
const STAMP_LAG: Duration = Duration::from_millis(50);

pub struct Options {
	pub dme: PathBuf,
	/// The running .dmb; rebuilds establish a new source baseline.
	pub dmb: PathBuf,
	/// DreamMaker's command line defines, as `(name, value)`
	pub defines: Vec<(String, Option<String>)>,
	pub hot_reload: bool
}

/// Events from the source worker to the adapter.
pub enum Event {
	/// Running code changed; breakpoint lines may need remapping.
	Changed,
	Failed(String),
	/// Source synchronization is complete; the connected world can run.
	Ready,
	/// The files are being watched: a change from now on is reloaded
	Watching,
	/// Changes prepared for `world`, identified by `Command::Connected`.
	/// The adapter replies with `Command::Reloaded`. `notes` become journal
	/// warnings.
	Reload {
		request: Box<HotReloadRequest>,
		summary: Summary,
		notes: Vec<String>,
		world: u64
	},
	/// Rejected changes and diagnostics for the journal.
	NotReloaded {
		summary: String,
		errors: Vec<String>,
		warnings: Vec<String>
	},
	/// Source edits with no runtime changes, for the journal.
	NothingChanged {
		warnings: Vec<String>
	},
	/// Source macros sent after parsing and whenever reload changes them.
	Macros(Vec<Macro>),
	/// A new skin for `world`. The adapter replies with `Command::Reloaded`.
	Skin {
		request: SkinRequest,
		/// Relative to the `.dme` directory.
		file: PathBuf,
		/// Files referenced by this skin but absent from the built skin.
		images: Vec<String>,
		/// Restore after reboot, before clients log in.
		restored: bool,
		world: u64
	},
	/// Client script update. The adapter replies with `Command::Reloaded`.
	Script {
		request: ScriptRequest,
		/// Restore after reboot, before clients log in.
		restored: bool,
		world: u64
	}
}

/// Commands from the adapter to the source worker.
pub enum Command {
	/// A world connected with this BYOND version and connection generation.
	Connected { byond: (u32, u32), world: u64 },
	/// Whether the last reload was applied
	Reloaded(bool),
	/// Resources cached by the last reload; sent before `Reloaded`.
	Resources(Vec<ReloadedResource>)
}

pub fn start(mut options: Options, out: Arc<Output>, inputs: mpsc::Sender<Input>) -> mpsc::Sender<Command> {
	// The client knows files by their full paths
	if let Ok(dme) = std::path::absolute(&options.dme) {
		options.dme = dme;
	}
	let (commands, receiver) = mpsc::channel();
	std::thread::Builder::new()
		.name("code".to_owned())
		.spawn(move || {
			let code = Code {
				root: options.dme.parent().map(Path::to_owned).unwrap_or_default(),
				parse_options: ParseOptions {
					defines: options.defines.clone(),
					byond: None
				},
				options,
				out,
				inputs,
				commands: receiver,
				reconnected: Cell::new(false),
				world: Cell::new(0),
				parsed: Cell::new(false),
				parse_started: Cell::new(SystemTime::UNIX_EPOCH),
				macros: RefCell::new(None),
				resource_dirs: RefCell::new(vec![]),
				reloaded_resources: RefCell::new(vec![]),
				named_resources: RefCell::new(vec![])
			};
			code.run();
		})
		.expect("couldn't start the code's thread");
	commands
}

struct Code {
	options: Options,
	parse_options: ParseOptions,
	root: PathBuf,
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,
	commands: mpsc::Receiver<Command>,
	/// A reboot occurred while awaiting a reload response.
	reconnected: Cell<bool>,
	/// Connection generation used to prepare reloads.
	world: Cell<u64>,
	/// A full parse occurred since the last heap trim.
	parsed: Cell<bool>,
	/// Start time of the last full parse.
	parse_started: Cell<SystemTime>,
	/// The macros the world was last given
	macros: RefCell<Option<Vec<Macro>>>,
	/// `FILE_DIR` search paths for resource lookups.
	resource_dirs: RefCell<Vec<String>>,
	/// Resource IDs reported by the last reload.
	reloaded_resources: RefCell<Vec<ReloadedResource>>,
	/// Resource names referenced by the last reload's code.
	named_resources: RefCell<Vec<String>>
}

/// Track running source and pending disk changes.
struct Watching {
	/// Source baseline from the first connection or latest `.dmb` rebuild.
	built: Snapshot,
	/// Files changed since `built`
	edited_since_built: HashSet<PathBuf>,
	dmb_modified: Option<SystemTime>,
	/// Current running source when it differs from `built`.
	applied: Option<Snapshot>,
	files: Watched,
	skin: Skin,
	script: Script,
	/// Watched resource files and their current IDs.
	resources: Resources,
	/// Original resource IDs, used after reboot.
	built_resources: Resources
}

impl Code {
	fn log(&self, message: impl Into<String>) {
		self.out.console(format!("Hot reload: {}", message.into()));
	}

	/// Schedule source synchronization for a newly connected world.
	fn connected(&self, world: u64) {
		self.world.set(world);
		self.reconnected.set(true);
	}

	fn send(&self, event: Event) -> bool {
		self.inputs.send(Input::Code(event)).is_ok()
	}

	fn parse(&self) -> Result<Snapshot, String> {
		self.parsed.set(true);
		self.parse_started.set(SystemTime::now());
		Snapshot::parse(&self.options.dme, &self.parse_options)
	}

	fn run(mut self) {
		// Parse after connecting: DM_VERSION and conditional compilation depend
		// on the running world's BYOND build.
		let byond = loop {
			match self.commands.recv() {
				Ok(Command::Connected { byond, world }) => {
					self.world.set(world);
					break byond;
				}
				Ok(Command::Reloaded(_) | Command::Resources(_)) => {}
				Err(_) => return
			}
		};
		self.parse_options.byond = Some(byond);
		// Record this before parsing so a rebuild during the parse is detected
		// as a new baseline on the next connection.
		let dmb_modified = self.dmb_modified();

		let started = Instant::now();
		let built = match self.parse() {
			Ok(snapshot) => snapshot,
			Err(e) => {
				self.send(Event::Failed(e));
				return;
			}
		};
		if !self.send(Event::Changed) || !self.send(Event::Ready) || !self.options.hot_reload {
			return;
		}

		self.warn_of_errors(&built);
		let files = files(&built, &self.options.dme);
		self.warn_of_later_edits(&files, dmb_modified);
		let resources = self.read_resources(&files);
		let script = Script::built(built.client_script());
		let mut watched = files.clone();
		watched.extend(resources.files.keys().cloned());
		let mut watching = Watching {
			files: Watched::new(&self.root, &watched, self.parse_started.get()),
			skin: Skin::built(&self.root, built.skin()),
			script,
			dmb_modified,
			edited_since_built: HashSet::new(),
			applied: None,
			built,
			built_resources: Resources {
				files: resources.files.clone(),
				dirs: resources.dirs.clone()
			},
			resources
		};
		let took = started.elapsed();
		// Send macros before reporting readiness.
		if !self.give_macros() {
			return;
		}
		self.log(format!(
			"watching {} files with {} procs (read in {:.1} s)",
			files.len(),
			watching.built.procs().count(),
			took.as_secs_f32()
		));
		if !self.send(Event::Watching) {
			return;
		}
		self.watch(&mut watching);
	}

	fn dmb_modified(&self) -> Option<SystemTime> {
		std::fs::metadata(&self.options.dmb).and_then(|m| m.modified()).ok()
	}

	fn watch(&self, watching: &mut Watching) {
		loop {
			let wait = INTERVAL.max(watching.files.took * (LOOKING_SHARE - 1));
			match self.commands.recv_timeout(wait) {
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Ok(Command::Reloaded(_) | Command::Resources(_)) => {}
				Err(mpsc::RecvTimeoutError::Disconnected) => return,
				Err(mpsc::RecvTimeoutError::Timeout) => {
					if !self.look(watching) {
						return;
					}
				}
			}
			while self.reconnected.take() {
				if !self.catch_up(watching) {
					return;
				}
			}
			if self.parsed.take() {
				give_back_memory();
			}
		}
	}

	/// Reload disk changes. Returns false if the adapter disconnected.
	fn look(&self, watching: &mut Watching) -> bool {
		let mut changed = watching.files.changed(&self.root);
		if changed.is_empty() {
			return true;
		}

		// Editors save in steps, and a checkout touches many files
		loop {
			std::thread::sleep(Duration::from_millis(200));
			let more = watching.files.changed(&self.root);
			if more.is_empty() {
				break;
			}
			changed.extend(more);
		}

		let mut shown: Vec<String> = changed.iter().map(|f| f.display().to_string()).collect();
		shown.sort();
		self.log(format!("changed: {}", shown.join(", ")));

		watching.skin.take_edit(&mut changed);
		// Resource files, then code, then the skin and the client script
		let resource_files: Vec<PathBuf> = changed.iter().filter(|f| watching.resources.files.contains_key(*f)).cloned().collect();
		if !resource_files.is_empty() {
			changed.retain(|f| !watching.resources.files.contains_key(f));
			if !self.reload_resources(watching, &resource_files) {
				return false;
			}
		}
		if !changed.is_empty() && !self.reload_code(watching, changed) {
			return false;
		}
		self.reload_skin(watching, false) && self.reload_script(watching, false)
	}

	/// Reload the changed DM files. Return false on adapter disconnect.
	fn reload_code(&self, watching: &mut Watching, changed: HashSet<PathBuf>) -> bool {
		// Reparse a single file when only its proc bodies changed.
		if let [file] = changed.iter().collect::<Vec<_>>().as_slice() {
			let file = (*file).clone();
			// Use the last applied snapshot, including incremental file parses.
			let current = match watching.applied.as_mut() {
				Some(applied) => applied,
				None => &mut watching.built
			};
			match current.reload_file(&self.root, &file) {
				Ok(reload) => {
					watching.edited_since_built.insert(file);
					return match self.reload(&reload.changes, current, current, vec![], false) {
						None => false,
						Some(true) => {
							let current = match watching.applied.as_mut() {
								Some(applied) => applied,
								None => &mut watching.built
							};
							current.keep(reload);
							self.watch_named(watching);
							self.send(Event::Changed);
							true
						}
						Some(false) => true
					};
				}
				Err(why) => self.log(format!(
					"parsing the full project: {} requires more than a proc-body reload ({})",
					file.display(),
					why
				))
			}
		}

		let new = match self.parse() {
			Ok(new) => new,
			Err(e) => {
				self.log(format!("couldn't read the code: {}", e));
				return self.not_reloaded(e);
			}
		};
		watching.skin.parsed(new.skin());
		watching.script.parsed(new.client_script());
		let current = watching.applied.as_ref().unwrap_or(&watching.built);
		let mut changes = diff(current, &new, Some(&changed));
		refuse_hidden(current, &new, &mut changes);
		changes.settings_as_built(&watching.built, &new);
		watching.edited_since_built.extend(changed);

		// A file the new parse includes for the first time
		for file in files(&new, &self.options.dme) {
			watching.files.add(&self.root, &file, self.parse_started.get());
		}

		match self.reload(&changes, &new, watching.applied.as_ref().unwrap_or(&watching.built), vec![], false) {
			None => false,
			Some(true) => {
				self.watch_named(watching);
				self.send(Event::Changed);
				watching.applied = Some(new);
				watching.built.clear_patches();
				self.give_macros()
			}
			Some(false) => true
		}
	}

	/// Restore reloads after a reboot, or adopt a rebuilt `.dmb` as the
	/// baseline. Send `Event::Ready` when the world can resume. Return false on
	/// disconnect.
	fn catch_up(&self, watching: &mut Watching) -> bool {
		let dmb_modified = self.dmb_modified();
		if dmb_modified != watching.dmb_modified {
			// The rebuilt world can run while we parse its source.
			if !self.ready() {
				return false;
			}
			self.log("using the rebuilt .dmb as the source baseline");
			watching.dmb_modified = dmb_modified;
			// Include pending edits in the baseline rather than reloading them.
			watching.files.changed(&self.root);
			match self.parse() {
				Ok(built) => {
					self.warn_of_errors(&built);
					let files = files(&built, &self.options.dme);
					self.warn_of_later_edits(&files, dmb_modified);
					let resources = self.read_resources(&files);
					for file in files.iter().chain(resources.files.keys()) {
						watching.files.add(&self.root, file, self.parse_started.get());
					}
					watching.skin = Skin::built(&self.root, built.skin());
					watching.script = Script::built(built.client_script());
					watching.built_resources = Resources {
						files: resources.files.clone(),
						dirs: resources.dirs.clone()
					};
					watching.resources = resources;
					watching.built = built;
				}
				Err(e) => self.log(format!("couldn't read the code: {}", e))
			}
			watching.applied = None;
			watching.edited_since_built.clear();
			return self.send(Event::Changed) && self.give_macros();
		}

		// A reboot resets the resource cache. Restore edits once the world
		// has loaded its own resources.
		watching.applied = None;
		watching.built.clear_patches();
		// Restore the skin before clients log in.
		watching.skin.rebooted();
		if !self.reload_skin(watching, true) {
			return false;
		}
		watching.resources.restarted(&watching.built_resources);
		watching.script.rebooted();
		if !self.reload_script(watching, true) {
			return false;
		}
		let resource_files: Vec<PathBuf> = watching
			.resources
			.edited_since_built(&self.root)
			.into_iter()
			.map(|(path, _)| path)
			.collect();
		if watching.edited_since_built.is_empty() {
			return self.restore_resources(watching, &resource_files) && self.ready();
		}

		self.log("world rebooted; restoring source changes since the build");
		let current = match self.parse() {
			Ok(current) => current,
			Err(e) => {
				self.log(format!("couldn't read the code: {}", e));
				return self.not_reloaded(e) && self.ready();
			}
		};
		let mut changes = diff(&watching.built, &current, Some(&watching.edited_since_built));
		refuse_hidden(&watching.built, &current, &mut changes);
		restarted(&mut changes);
		let Some(applied) = self.reload(&changes, &current, &watching.built, vec![], true) else {
			return false;
		};
		if applied {
			watching.applied = Some(current);
			watching.built.clear_patches();
			self.watch_named(watching);
		}
		if !self.restore_resources(watching, &resource_files) || !self.ready() {
			return false;
		}
		// Send evaluation macros after releasing the world.
		!applied || self.give_macros()
	}

	/// Read resources from the world's `.dmb` and `.rsc`. Warn about edits
	/// since the build; leave them unchanged until the next save.
	fn read_resources(&self, sources: &[PathBuf]) -> Resources {
		let resources = match Resources::read(&self.root, &self.options.dmb, sources) {
			Ok(resources) => resources,
			Err(e) => {
				self.log(format!("resource watching unavailable: {}", e));
				Resources::default()
			}
		};
		*self.resource_dirs.borrow_mut() = resources.dirs.clone();
		let mut edited: Vec<String> = resources
			.edited_since_built(&self.root)
			.iter()
			.map(|(path, _)| path.display().to_string())
			.collect();
		if !edited.is_empty() {
			edited.sort();
			self.log(format!(
				"resource files changed before launch: {}. The world uses the built versions; rebuild or save the files again to apply edits",
				edited.join(", ")
			));
		}
		resources
	}

	/// Restore resource edits after reboot. The world applies them once its
	/// data is loaded, before its atoms' New(). Return false on disconnect.
	fn restore_resources(&self, watching: &mut Watching, files: &[PathBuf]) -> bool {
		if files.is_empty() {
			return true;
		}
		self.log("restoring resource files changed since the build");
		self.reload_resources(watching, files)
	}

	/// Reload edited resources and recompile their code and default users.
	/// The world caches new IDs and updates appearances using the old IDs.
	/// Return false on disconnect.
	fn reload_resources(&self, watching: &mut Watching, files: &[PathBuf]) -> bool {
		let changed = watching.resources.changed(&self.root, files);
		if changed.is_empty() {
			return true;
		}
		let current = watching.applied.as_ref().unwrap_or(&watching.built);
		let mut changes = Changes::default();
		let mut resources = vec![];
		for (path, _) in &changed {
			let file = &watching.resources.files[path];
			merge(&mut changes, current.resource_users(&file.names));
			let (ids, sharing) = watching.resources.replaced(path);
			if !sharing.is_empty() {
				let sharing: Vec<String> = sharing.iter().map(|other| other.display().to_string()).collect();
				self.log(format!(
					"{} shares a resource ID with {} (same bytes); appearances are unchanged. Rebuild to separate them",
					path.display(),
					sharing.join(", ")
				));
			}
			resources.push(HotReloadResource {
				path: path.to_string_lossy().replace('\\', "/"),
				ids
			});
		}
		self.reloaded_resources.borrow_mut().clear();
		match self.reload(&changes, current, current, resources, false) {
			None => false,
			Some(true) => {
				let reloaded = self.reloaded_resources.take();
				for (path, crc) in &changed {
					let name = path.to_string_lossy().replace('\\', "/");
					match reloaded.iter().find(|r| r.path == name) {
						Some(r) => watching.resources.reloaded(path, r.id, *crc),
						// A loading world defers the reload and tracks the
						// resource ID itself.
						None => watching.resources.shown(path, *crc)
					}
				}
				self.send(Event::Changed)
			}
			Some(false) => true
		}
	}

	/// Watch new resource references absent from the build. Later saves
	/// reload those files too.
	fn watch_named(&self, watching: &mut Watching) {
		for name in self.named_resources.take() {
			if let Some(file) = watching.resources.named(&self.root, &name) {
				watching.files.add(&self.root, &file, self.parse_started.get());
			}
		}
	}

	/// Warn that parse errors make the source baseline incomplete.
	fn warn_of_errors(&self, built: &Snapshot) {
		let errors = errors(built);
		if errors.is_empty() {
			return;
		}
		self.log("source baseline has parse errors; definitions hidden by them require a rebuild:");
		for error in placed(&errors, built).iter().take(5) {
			self.out.console(format!("    {}", error));
		}
		if errors.len() > 5 {
			self.out.console(format!("    and {} more", errors.len() - 5));
		}
	}

	/// Warn about files saved after the build. Hot reload assumes the initial
	/// source matches the world and only applies subsequent edits.
	fn warn_of_later_edits(&self, files: &[PathBuf], built: Option<SystemTime>) {
		let Some(built) = built else {
			return;
		};
		let mut later: Vec<String> = files
			.iter()
			.filter(|file| modified_time(&self.root, file).is_some_and(|time| time > built))
			.map(|file| file.display().to_string())
			.collect();
		if later.is_empty() {
			return;
		}
		later.sort();
		let more = later.len().saturating_sub(5);
		later.truncate(5);
		let mut named = later.join(", ");
		if more > 0 {
			named += &format!(" and {} more", more);
		}
		self.log(format!(
			"changed since the .dmb was built: {}. These edits may be missing from the world; rebuild to include them. Hot reload only applies \
			 later edits",
			named
		));
	}

	/// Release the world unless a newer connection needs synchronization first.
	/// Return false if sending `Event::Ready` fails.
	fn ready(&self) -> bool {
		while let Ok(command) = self.commands.try_recv() {
			if let Command::Connected { world, .. } = command {
				self.connected(world);
			}
		}
		self.reconnected.get() || self.send(Event::Ready)
	}

	/// Send macros if they changed. Returns false if the adapter disconnected.
	fn give_macros(&self) -> bool {
		let macros = match macros(&self.options.dme, &self.parse_options) {
			Ok(macros) => macros,
			Err(e) => {
				self.log(format!("source macros unavailable for evaluation: {}", e));
				return true;
			}
		};
		if self.macros.borrow().as_ref() == Some(&macros) {
			return true;
		}
		*self.macros.borrow_mut() = Some(macros.clone());
		self.send(Event::Macros(macros))
	}

	/// Journal a source-read failure. Returns false if the adapter
	/// disconnected.
	fn not_reloaded(&self, error: String) -> bool {
		self.send(Event::NotReloaded {
			summary: "couldn't read the code".to_owned(),
			errors: vec![error],
			warnings: vec![]
		})
	}

	/// Apply `changes` from `before` to `snapshot` through the adapter.
	/// Return `None` on disconnect. When `restarted`, replace the global init
	/// proc before global initializers run.
	fn reload(&self, changes: &Changes, snapshot: &Snapshot, before: &Snapshot, resources: Vec<HotReloadResource>, restarted: bool) -> Option<bool> {
		if !changes.refused.is_empty() {
			self.log("rejected; rebuild required for these changes:");
			let refused = placed(&changes.refused, snapshot);
			for why in &refused {
				self.out.console(format!("    {}", why));
			}
			let refused = Event::NotReloaded {
				summary: "rebuild required".to_owned(),
				errors: refused,
				warnings: changes.warnings.clone()
			};
			return self.send(refused).then_some(false);
		}
		let mut request = reload_request(changes);
		if restarted {
			request.global_init = changes.global_init.clone();
		}
		without_existing_statics(&mut request, before);
		*self.named_resources.borrow_mut() = resource_names(&request);
		let files: Vec<String> = resources.iter().map(|r| r.path.clone()).collect();
		request.resources = resources;
		request.resource_dirs = self.resource_dirs.borrow().clone();
		// The running world determines whether removed vars actually exist.
		let notes: Vec<String> = changes.warnings.iter().filter(|note| removed_var(note).is_none()).cloned().collect();
		if request.is_empty() && request.removed_vars.is_empty() {
			for why in &notes {
				self.out.console(format!("    note: {}", why));
			}
			self.log("no runtime changes");
			return self
				.send(Event::NothingChanged {
					warnings: changes.warnings.clone()
				})
				.then_some(true);
		}

		if !self.send(Event::Reload {
			request: Box::new(request),
			summary: Summary::of(changes).with_resources(files),
			notes,
			world: self.world.get()
		}) {
			return None;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => return Some(applied),
				Ok(Command::Resources(reloaded)) => *self.reloaded_resources.borrow_mut() = reloaded,
				// The adapter rejects reloads prepared before a reboot;
				// `catch_up` will prepare them for the new world.
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Err(_) => return None
			}
		}
	}

	/// Update the skin if it differs from the world's. Return false on
	/// disconnect.
	fn reload_skin(&self, watching: &mut Watching, restored: bool) -> bool {
		let Some(change) = watching.skin.change(&self.root) else {
			return true;
		};
		if !self.send(Event::Skin {
			request: change.request.clone(),
			file: change.file.clone(),
			images: change.images.clone(),
			restored,
			world: self.world.get()
		}) {
			return false;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => {
					if applied {
						watching.skin.applied(change);
					}
					return true;
				}
				Ok(Command::Resources(_)) => {}
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Err(_) => return false
			}
		}
	}

	/// Update the client script if it differs from the world's.
	/// Return false on disconnect.
	fn reload_script(&self, watching: &mut Watching, restored: bool) -> bool {
		if let Some(why) = watching.script.problem() {
			self.log(format!("client script: {}; the world keeps the script it has", why));
		}
		let Some(change) = watching.script.change() else {
			return true;
		};
		if !self.send(Event::Script {
			request: change.request.clone(),
			restored,
			world: self.world.get()
		}) {
			return false;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => {
					if applied {
						watching.script.applied(change);
					}
					return true;
				}
				Ok(Command::Resources(_)) => {}
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Err(_) => return false
			}
		}
	}
}

/// Use the current global and static initializers after reboot
/// (see `HotReloadRequest::global_init`).
pub fn restarted(changes: &mut Changes) {
	changes
		.warnings
		.retain(|note| !note.ends_with(": changed global initializer applies only after a restart"));
}

/// Resource literals referenced by a reload's code and defaults.
fn resource_names(request: &HotReloadRequest) -> Vec<String> {
	/// Collect resource names while resolving other assembly operands.
	#[derive(Default)]
	struct Named(Vec<String>);

	impl Named {
		fn value(&mut self, value: &dmasm::reload::ConstValue) {
			if let dmasm::reload::ConstValue::Resource(name) = value {
				self.add(name);
			}
		}

		fn add(&mut self, name: &str) {
			if !self.0.iter().any(|had| had == name) {
				self.0.push(name.to_owned());
			}
		}

		fn code(&mut self, nodes: &[dmasm::Node], statics: &[dmasm::reload::StaticVar]) {
			let _ = dmasm::assembler::assemble(nodes, self);
			for local in statics {
				if let Some(value) = &local.value {
					self.value(value);
				}
				if let Some(init) = &local.init {
					self.code(&init.nodes, &init.statics);
				}
			}
		}
	}

	impl dmasm::assembler::AssembleEnv for Named {
		fn get_string_index(&mut self, _: &[u8]) -> Option<u32> {
			Some(0)
		}

		fn get_variable_name_index(&mut self, _: &[u8]) -> Option<u32> {
			Some(0)
		}

		fn get_proc_index(&mut self, _: &str) -> Option<u32> {
			Some(0)
		}

		fn get_type(&mut self, _: &str) -> Option<(u8, u32)> {
			Some((0x20, 0))
		}

		fn get_global_index(&mut self, _: &[u8], _: Option<&str>) -> Option<u32> {
			Some(0)
		}

		fn get_resource_index(&mut self, path: &str) -> Option<u32> {
			self.add(path);
			Some(0)
		}

		fn get_instance(&mut self, _: &str, _: &[u32]) -> Option<u32> {
			Some(0)
		}
	}

	let mut named = Named::default();
	for code in request.procs.iter().chain(request.new_procs.iter().map(|new| &new.code)) {
		named.code(&code.nodes, &code.statics);
	}
	for code in request.init_procs.iter().filter_map(|init| init.code.as_ref()) {
		named.code(&code.nodes, &code.statics);
	}
	for default in request
		.var_defaults
		.iter()
		.map(|d| &d.value)
		.chain(request.builtin_defaults.iter().map(|d| &d.value))
	{
		named.value(default);
	}
	for new in &request.new_types {
		for (_, field) in &new.fields {
			named.value(field);
		}
	}
	for var in &request.new_vars {
		named.value(&var.value);
		if let Some(init) = &var.init {
			named.code(&init.nodes, &init.statics);
		}
	}
	for var in &request.redeclared {
		if let Some(value) = &var.value {
			named.value(value);
		}
		if let Some(init) = &var.init {
			named.code(&init.nodes, &init.statics);
		}
	}
	if let Some(init) = &request.global_init {
		named.code(&init.code.nodes, &init.code.statics);
	}
	named.0
}

/// Merge a resource file's required changes into the existing changes.
fn merge(changes: &mut Changes, more: Changes) {
	for reload in more.reloads {
		if !changes.reloads.iter().any(|r| r.proc == reload.proc) {
			changes.reloads.push(reload);
		}
	}
	changes.var_defaults.extend(more.var_defaults);
	changes.builtin_defaults.extend(more.builtin_defaults);
	for init in more.init_procs {
		if !changes.init_procs.iter().any(|i| i.type_path == init.type_path) {
			changes.init_procs.push(init);
		}
	}
	changes.warnings.extend(more.warnings);
	changes.refused.extend(more.refused);
}

/// Format unresolved snapshot errors without lines for comparison.
/// Recovered const initializers are excluded by dmasm.
fn errors(snapshot: &Snapshot) -> Vec<String> {
	snapshot
		.parse_errors()
		.into_iter()
		.map(|(error, _)| format!("parse error: {}", error))
		.collect()
}

/// Reject additions exposed by a repaired parse error: they may already exist
/// in the built world. Re-adding them could create unintended proc overrides.
fn refuse_hidden(old: &Snapshot, new: &Snapshot, changes: &mut Changes) {
	if changes.new_types.is_empty() && changes.new_procs.is_empty() && changes.new_vars.is_empty() {
		return;
	}
	let mut fixed = errors(old);
	for error in errors(new) {
		if let Some(at) = fixed.iter().position(|e| *e == error) {
			fixed.remove(at);
		}
	}
	for error in fixed {
		let error = error.strip_prefix("parse error: ").unwrap_or(&error);
		changes.refused.push(format!(
			"{}: newly visible definitions may have been hidden by this fixed parse error. Rebuild, then reboot before reloading",
			error
		));
	}
}

/// Attach source locations to diagnostics as `file:line: message`.
fn placed(errors: &[String], snapshot: &Snapshot) -> Vec<String> {
	let parse_errors: Vec<(String, String)> = snapshot
		.parse_errors()
		.into_iter()
		.map(|(error, placed)| (format!("parse error: {}", error), placed))
		.collect();
	errors
		.iter()
		.map(|error| {
			if let Some((_, placed)) = parse_errors.iter().find(|(text, _)| text == error) {
				return placed.clone();
			}
			// Also accept diagnostics that already include a line: `parse
			// error: file:line: message`.
			if let Some(placed) = error.strip_prefix("parse error: ").filter(|rest| {
				rest.split_once(": ")
					.and_then(|(place, _)| place.rsplit_once(':'))
					.is_some_and(|(_, line)| line.parse::<u32>().is_ok())
			}) {
				return placed.to_owned();
			}
			let file_line = error.split_once(": doesn't compile: line ").and_then(|(proc, rest)| {
				let (line, what) = rest.split_once(": ")?;
				let key = snapshot.procs().find(|key| key.to_string() == proc)?;
				Some(format!("{}:{}: {}: doesn't compile: {}", snapshot.proc_file(key)?, line, proc, what))
			});
			file_line.unwrap_or_else(|| error.clone())
		})
		.collect()
}

/// Reload summary that can exclude removals skipped by the running world.
pub struct Summary {
	/// Actions, display names with optional removed-proc paths, and plural
	/// labels.
	parts: Vec<(&'static str, Vec<(String, Option<String>)>, &'static str)>,
	line_moves: usize
}

impl Summary {
	fn of(changes: &Changes) -> Self {
		let named = |names: Vec<String>| names.into_iter().map(|name| (name, None)).collect();
		let var = |type_path: &str, var: &str| format!("{}/var/{}", type_path, var);
		let parts = vec![
			("added", named(changes.new_types.iter().map(|t| t.path.clone()).collect()), "types"),
			(
				"reparented",
				named(
					changes
						.reparented
						.iter()
						.map(|r| format!("{} (now under {})", r.type_path, r.parent))
						.collect()
				),
				"types"
			),
			(
				"recompiled users of",
				named(changes.consts.iter().chain(&changes.retyped).cloned().collect()),
				"vars"
			),
			("reloaded", named(changes.reloads.iter().map(|r| r.proc.to_string()).collect()), "procs"),
			(
				"updated settings for",
				named(changes.settings.iter().map(|c| c.proc.to_string()).collect()),
				"procs"
			),
			(
				"added",
				named(changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect()),
				"procs"
			),
			(
				"unlinked",
				changes
					.removed_procs
					.iter()
					.map(|p| (p.proc.to_string(), Some(p.proc.path.clone())))
					.collect(),
				"procs"
			),
			(
				"added",
				named(changes.new_vars.iter().map(|v| var(&v.type_path, &v.var)).collect()),
				"vars"
			),
			(
				"updated declarations for",
				named(changes.redeclared.iter().map(|v| var(&v.type_path, &v.var)).collect()),
				"vars"
			),
			(
				"set the default of",
				named(
					changes
						.var_defaults
						.iter()
						.chain(&changes.builtin_defaults)
						.map(|d| var(&d.types[0], &d.var))
						.collect()
				),
				"vars"
			),
			(
				"restored inherited defaults for",
				named(changes.builtin_inherited.iter().map(|d| var(&d.types[0], &d.var)).collect()),
				"vars"
			),
			(
				"rebuilt the init procs of",
				named(changes.init_procs.iter().map(|i| i.type_path.clone()).collect()),
				"types"
			),
		];
		Summary {
			parts,
			line_moves: changes.line_moves.len()
		}
	}

	/// List cached resource files before code changes.
	fn with_resources(mut self, files: Vec<String>) -> Self {
		if !files.is_empty() {
			self.parts
				.insert(0, ("reloaded", files.into_iter().map(|f| (f, None)).collect(), "resource files"));
		}
		self
	}

	/// Format the summary, excluding removed procs listed in `skipped`.
	pub fn text(&self, skipped: &[String]) -> String {
		let mut said: Vec<String> = self
			.parts
			.iter()
			.filter_map(|(done, names, what)| {
				let names: Vec<&str> = names
					.iter()
					.filter(|(_, path)| path.as_ref().is_none_or(|path| !skipped.contains(path)))
					.map(|(name, _)| name.as_str())
					.collect();
				match names.len() {
					0 => None,
					1..=5 => Some(format!("{} {}", done, names.join(", "))),
					n => Some(format!("{} {} {}", done, n, what))
				}
			})
			.collect();
		if self.line_moves > 0 {
			said.push(format!("remapped lines in {} additional procs", self.line_moves));
		}
		if said.is_empty() {
			return "no runtime changes".to_owned();
		}
		said.join("; ")
	}
}

/// Suffix used by dmasm's retained-definition diagnostics.
pub const RETAINED: &str = " was removed from source but retained in the running world";

/// Extract a removed type var for verification by the running world.
fn removed_var(note: &str) -> Option<RemovedVar> {
	let (type_path, var) = note.strip_suffix(RETAINED)?.split_once(": var ")?;
	type_path.starts_with('/').then(|| RemovedVar {
		type_path: type_path.to_owned(),
		var: var.to_owned()
	})
}

fn proc_ref(key: &ProcKey) -> ProcRef {
	ProcRef {
		path: key.path.clone(),
		override_id: key.override_id
	}
}

fn proc_code(r: &dmasm::reload::ProcReload) -> HotReloadProc {
	HotReloadProc {
		proc: proc_ref(&r.proc),
		nodes: r.nodes.clone(),
		locals: r.locals.clone(),
		params: r.params.clone(),
		settings: r.settings.clone(),
		statics: r.statics.clone()
	}
}

/// Exclude statics declared before the edit so their slots and values survive.
/// Unread statics are absent from proc bytecode; sending them again would
/// allocate duplicate slots and rerun their initializers.
fn without_existing_statics(request: &mut HotReloadRequest, before: &Snapshot) {
	for code in &mut request.procs {
		if code.statics.is_empty() {
			continue;
		}
		let key = ProcKey {
			path: code.proc.path.clone(),
			override_id: code.proc.override_id
		};
		// New procs have no existing statics.
		let Ok(old) = before.compile(&key) else {
			continue;
		};
		code.statics
			.retain(|local| !old.statics.iter().any(|had| had.name == local.name && had.scope == local.scope));
	}
}

/// What the debug server is sent for `changes`
pub fn reload_request(changes: &Changes) -> HotReloadRequest {
	HotReloadRequest {
		new_types: changes.new_types.clone(),
		procs: changes.reloads.iter().map(proc_code).collect(),
		settings: changes.settings.clone(),
		new_procs: changes
			.new_procs
			.iter()
			.map(|p| HotReloadNewProc {
				type_path: p.type_path.clone(),
				name: p.name.clone(),
				declared: p.declared,
				verb: p.verb,
				code: proc_code(&p.code)
			})
			.collect(),
		removed_procs: changes.removed_procs.clone(),
		builtin_defaults: changes.builtin_defaults.clone(),
		new_vars: changes.new_vars.clone(),
		redeclared: changes.redeclared.clone(),
		init_procs: changes
			.init_procs
			.iter()
			.map(|i| HotReloadInitProc {
				type_path: i.type_path.clone(),
				code: i.code.as_ref().map(proc_code)
			})
			.collect(),
		var_defaults: changes
			.var_defaults
			.iter()
			.map(|d| HotReloadVarDefault {
				var: d.var.clone(),
				value: d.value.clone(),
				types: d.types.clone()
			})
			.collect(),
		line_moves: changes
			.line_moves
			.iter()
			.map(|m| HotReloadLines {
				proc: proc_ref(&m.proc),
				lines: m.lines.clone()
			})
			.collect(),
		removed_vars: changes.warnings.iter().filter_map(|note| removed_var(note)).collect(),
		builtin_inherited: changes.builtin_inherited.clone(),
		reparented: changes.reparented.clone(),
		resources: vec![],
		resource_dirs: vec![],
		global_init: None
	}
}

/// The files a parse read, the .dme too
fn files(snapshot: &Snapshot, dme: &Path) -> Vec<PathBuf> {
	let mut files = snapshot.files();
	files.push(dme.to_owned());
	files.extend(snapshot.skin().map(Path::to_owned));
	files
}

/// Return freed parse memory to the OS; glibc can retain large unused heaps.
fn give_back_memory() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	// SAFETY: malloc_trim only releases unused memory.
	unsafe {
		libc::malloc_trim(0);
	}
}

fn modified_time(root: &Path, file: &Path) -> Option<SystemTime> {
	std::fs::metadata(root.join(file)).and_then(|m| m.modified()).ok()
}

/// Watched files and their modification times, grouped by directory.
/// Windows directory listings include timestamps, avoiding a file open
/// for every timestamp check.
#[derive(Default)]
struct Watched {
	/// Root-relative directories, with filenames normalized by `listed`.
	dirs: HashMap<PathBuf, HashMap<OsString, Vec<PathBuf>>>,
	times: HashMap<PathBuf, Option<SystemTime>>,
	/// Duration of the last poll.
	took: Duration
}

impl Watched {
	/// Watch files read by a parse starting at `read`.
	fn new(root: &Path, files: &[PathBuf], read: SystemTime) -> Self {
		let mut watched = Self::default();
		for file in files {
			watched.insert(file);
		}
		// Record initial timestamps using the same lookup as later polls.
		watched.changed(root);
		for time in watched.times.values_mut() {
			*time = unless_saved_since(*time, read);
		}
		watched
	}

	/// Register an untracked file read by a parse starting at `read`.
	fn add(&mut self, root: &Path, file: &Path, read: SystemTime) {
		let Some((dir, name)) = self.insert(file) else {
			return;
		};
		let names = &self.dirs[&dir];
		let time = listed_times(&root.join(&dir), names)
			.get(&name)
			.copied()
			.unwrap_or_else(|| modified_time(root, file));
		self.times.insert(file.to_owned(), unless_saved_since(time, read));
	}

	/// Register `file` without a timestamp. Return its directory and normalized
	/// name, or `None` if already tracked.
	fn insert(&mut self, file: &Path) -> Option<(PathBuf, OsString)> {
		if self.times.contains_key(file) {
			return None;
		}
		let dir = file.parent().map(Path::to_owned).unwrap_or_default();
		let name = file.file_name().map(listed).unwrap_or_default();
		self.dirs
			.entry(dir.clone())
			.or_default()
			.entry(name.clone())
			.or_default()
			.push(file.to_owned());
		self.times.insert(file.to_owned(), None);
		Some((dir, name))
	}

	/// Files changed since the last poll.
	fn changed(&mut self, root: &Path) -> HashSet<PathBuf> {
		let started = Instant::now();
		let mut changed = HashSet::new();
		for (dir, names) in &self.dirs {
			let listed = listed_times(&root.join(dir), names);
			for (name, files) in names {
				for file in files {
					// Fall back to metadata for missing or unmatched entries.
					let time = listed.get(name).copied().unwrap_or_else(|| modified_time(root, file));
					if let Some(was) = self.times.get_mut(file).filter(|was| **was != time) {
						*was = time;
						changed.insert(file.clone());
					}
				}
			}
		}
		self.took = started.elapsed();
		changed
	}
}

/// Clear timestamps near or after the parse start so the next poll rechecks
/// files that may have changed while being read. The clock allowance can
/// also cause a harmless reread of a file saved just before parsing.
fn unless_saved_since(time: Option<SystemTime>, read: SystemTime) -> Option<SystemTime> {
	time.filter(|time| *time + STAMP_LAG < read)
}

/// Read watched timestamps from a Windows directory listing. Return an empty
/// map elsewhere, where listings would require a metadata lookup per file.
fn listed_times<'a>(dir: &Path, names: &'a HashMap<OsString, Vec<PathBuf>>) -> HashMap<&'a OsString, Option<SystemTime>> {
	let Some(entries) = cfg!(windows).then(|| std::fs::read_dir(dir).ok()).flatten() else {
		return HashMap::new();
	};
	entries
		.flatten()
		// Use target metadata for symlinks, not the link timestamp.
		.filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
		.filter_map(|entry| {
			let (name, _) = names.get_key_value(&listed(&entry.file_name()))?;
			Some((name, entry.metadata().and_then(|m| m.modified()).ok()))
		})
		.collect()
}

/// Normalize filenames for case-insensitive matching on Windows.
fn listed(name: &OsStr) -> OsString {
	if cfg!(windows) {
		name.to_string_lossy().to_lowercase().into()
	} else {
		name.to_owned()
	}
}

/// Preprocessor and BYOND defines excluded from source macro exports.
const NOT_THE_CODES: [&str; 4] = ["DM_VERSION", "DM_BUILD", "FILE_DIR", "SPACEMAN_DMM"];

/// Export macros still defined after preprocessing, sorted by name.
/// Use the snapshot's defines and BYOND version; exclude built-in macros.
pub fn macros(dme: &Path, options: &ParseOptions) -> Result<Vec<Macro>, String> {
	let ctx = dreammaker::Context::default();
	let mut pp = dreammaker::Preprocessor::new(&ctx, dme.to_owned()).map_err(|e| format!("couldn't read {}: {}", dme.display(), e))?;
	let mut prelude = String::from("#undef SPACEMAN_DMM\n");
	if let Some((version, build)) = options.byond {
		prelude += &format!(
			"#undef DM_VERSION\n#define DM_VERSION {}\n#undef DM_BUILD\n#define DM_BUILD {}\n",
			version, build
		);
	}
	for (name, value) in &options.defines {
		prelude += &format!("#define {} {}\n", name, value.as_deref().unwrap_or(""));
	}
	pp.push_file(PathBuf::from("<auxtools defines>"), std::io::Cursor::new(prelude.into_bytes()))
		.map_err(|e| e.to_string())?;
	pp.by_ref().for_each(drop);
	let history = pp.finalize();

	// An active definition ends beyond every file. If a name was redefined,
	// export only its final definition.
	let mut defined: HashMap<String, (u16, Macro)> = HashMap::new();
	for (range, (name, define)) in history.iter() {
		let at_end = range.end.file == dreammaker::FileId::UNKNOWN && range.end.line == !0;
		if !at_end || define.location.file == dreammaker::FileId::BUILTINS || NOT_THE_CODES.contains(&name.as_str()) {
			continue;
		}
		if defined.get(name.as_str()).is_some_and(|(order, _)| *order > range.end.column) {
			continue;
		}
		let mut params: Vec<String> = define.params.iter().map(|param| param.to_string()).collect();
		if define.variadic {
			if let Some(last) = params.last_mut() {
				last.push_str("...");
			}
		}
		let text = auxtools_cli::macros::text_of(&define.subst);
		defined.insert(
			name.to_string(),
			(range.end.column, Macro {
				name: name.to_string(),
				params,
				text
			})
		);
	}
	let mut macros: Vec<Macro> = defined.into_values().map(|(_, define)| define).collect();
	macros.sort_by(|a, b| a.name.cmp(&b.name));
	Ok(macros)
}

/// `NAME` or `NAME=VALUE`, as DreamMaker takes them after `-D`
pub fn parse_define(define: &str) -> (String, Option<String>) {
	match define.split_once('=') {
		Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
		None => (define.to_owned(), None)
	}
}

#[cfg(test)]
mod tests {
	use std::time::{Duration, SystemTime};

	use super::*;

	#[test]
	fn changed_files_are_found_by_their_directories() {
		let root = std::env::temp_dir().join(format!("auxtools-watched-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds);
		let write = |path: &str, time: SystemTime| {
			let path = root.join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(&path, "").unwrap();
			std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
		};
		for file in ["world.dme", "code/a.dm", "code/b.dm", "code/sub/c.dm"] {
			write(file, at(0));
		}
		let files: Vec<PathBuf> = ["world.dme", "code/a.dm", "code/b.dm", "code/sub/c.dm", "code/new.dm"]
			.iter()
			.map(PathBuf::from)
			.collect();
		let mut watched = Watched::new(&root, &files, at(1));
		let changed = |watched: &mut Watched| {
			let mut changed: Vec<String> = watched.changed(&root).iter().map(|f| f.to_string_lossy().into_owned()).collect();
			changed.sort();
			changed
		};
		assert_eq!(changed(&mut watched), Vec::<String>::new());

		// Modified, created and deleted files.
		write("code/b.dm", at(5));
		write("code/sub/c.dm", at(5));
		write("code/new.dm", at(5));
		assert_eq!(changed(&mut watched), ["code/b.dm", "code/new.dm", "code/sub/c.dm"]);
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		std::fs::remove_file(root.join("code/a.dm")).unwrap();
		assert_eq!(changed(&mut watched), ["code/a.dm"]);

		// A file discovered by a later parse.
		write("code/later.dm", at(0));
		watched.add(&root, Path::new("code/later.dm"), at(1));
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		write("code/later.dm", at(9));
		assert_eq!(changed(&mut watched), ["code/later.dm"]);

		// Track differently cased paths to the same Windows file.
		if cfg!(windows) {
			watched.add(&root, Path::new("code/SUB/C.dm"), at(1));
			write("code/sub/c.dm", at(12));
			assert_eq!(changed(&mut watched), ["code/SUB/C.dm", "code/sub/c.dm"]);
		}

		// Recheck files saved during the parse, including newly discovered
		// files.
		write("code/b.dm", at(20));
		write("code/during.dm", at(20));
		let mut watched = Watched::new(&root, &files[..3], at(19));
		watched.add(&root, Path::new("code/during.dm"), at(19));
		assert_eq!(changed(&mut watched), ["code/b.dm", "code/during.dm"]);
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		let _ = std::fs::remove_dir_all(&root);
	}
}
