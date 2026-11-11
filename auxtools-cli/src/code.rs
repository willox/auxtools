//! Parse source and watch files for hot reload.
//! The source worker uses `dmasm::reload` to compile changes; the adapter
//! applies them and reports the result. SpacemanDMM contexts are thread-bound,
//! so all parsing stays on this worker thread.

use std::{
	cell::{Cell, RefCell},
	collections::{HashMap, HashSet},
	path::{Path, PathBuf},
	sync::{mpsc, Arc},
	time::{Duration, Instant, SystemTime}
};

use auxtools_cli::server_types::{
	HotReloadInitProc, HotReloadLines, HotReloadNewProc, HotReloadProc, HotReloadRequest, HotReloadVarDefault, Macro, ProcRef
};
use dmasm::reload::{diff, Changes, ParseOptions, ProcKey, Snapshot};

use crate::{adapter::Input, dap::Output};

/// How often to look for changed files
const INTERVAL: Duration = Duration::from_millis(500);

pub struct Options {
	pub dme: PathBuf,
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
	/// Changes to apply. The adapter replies with `Command::Reloaded`.
	/// `notes` are included in the journal's warnings.
	Reload {
		request: Box<HotReloadRequest>,
		summary: String,
		notes: Vec<String>
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
	Macros(Vec<Macro>)
}

/// Commands from the adapter to the source worker.
pub enum Command {
	/// The world connected, on this BYOND version
	Connected { byond: (u32, u32) },
	/// Whether the last reload was applied
	Reloaded(bool)
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
				macros: RefCell::new(None)
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
	/// The macros the world was last given
	macros: RefCell<Option<Vec<Macro>>>
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
	modified: HashMap<PathBuf, Option<SystemTime>>
}

impl Code {
	fn log(&self, message: impl Into<String>) {
		self.out.console(format!("Hot reload: {}", message.into()));
	}

	fn send(&self, event: Event) -> bool {
		self.inputs.send(Input::Code(event)).is_ok()
	}

	fn parse(&self) -> Result<Snapshot, String> {
		Snapshot::parse(&self.options.dme, &self.parse_options)
	}

	fn run(mut self) {
		// Parse after connecting: DM_VERSION and conditional compilation depend
		// on the running world's BYOND build.
		let byond = loop {
			match self.commands.recv() {
				Ok(Command::Connected { byond }) => break byond,
				Ok(Command::Reloaded(_)) => {}
				Err(_) => return
			}
		};
		self.parse_options.byond = Some(byond);

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

		let files = files(&built, &self.options.dme);
		let mut watching = Watching {
			modified: modified_times(&self.root, &files),
			dmb_modified: self.dmb_modified(),
			edited_since_built: HashSet::new(),
			applied: None,
			built
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
		std::fs::metadata(self.options.dme.with_extension("dmb")).and_then(|m| m.modified()).ok()
	}

	fn watch(&self, watching: &mut Watching) {
		loop {
			match self.commands.recv_timeout(INTERVAL) {
				Ok(Command::Connected { .. }) => self.reconnected.set(true),
				Ok(Command::Reloaded(_)) => {}
				Err(mpsc::RecvTimeoutError::Disconnected) => return,
				Err(mpsc::RecvTimeoutError::Timeout) => {
					if !self.look(watching) {
						return;
					}
				}
			}
			while self.reconnected.take() {
				if !self.catch_up(watching) || !self.send(Event::Ready) {
					return;
				}
			}
		}
	}

	/// Reload disk changes. Returns false if the adapter disconnected.
	fn look(&self, watching: &mut Watching) -> bool {
		let mut changed = changed_files(&self.root, &mut watching.modified);
		if changed.is_empty() {
			return true;
		}

		// Editors save in steps, and a checkout touches many files
		loop {
			std::thread::sleep(Duration::from_millis(200));
			let more = changed_files(&self.root, &mut watching.modified);
			if more.is_empty() {
				break;
			}
			changed.extend(more);
		}

		let mut shown: Vec<String> = changed.iter().map(|f| f.display().to_string()).collect();
		shown.sort();
		self.log(format!("changed: {}", shown.join(", ")));

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
					return match self.reload(&reload.changes, current) {
						None => false,
						Some(true) => {
							let current = match watching.applied.as_mut() {
								Some(applied) => applied,
								None => &mut watching.built
							};
							current.keep(reload);
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
		let changes = diff(watching.applied.as_ref().unwrap_or(&watching.built), &new, Some(&changed));
		watching.edited_since_built.extend(changed);

		// A file the new parse includes for the first time
		for file in files(&new, &self.options.dme) {
			watching.modified.entry(file.clone()).or_insert_with(|| modified_time(&self.root, &file));
		}

		match self.reload(&changes, &new) {
			None => false,
			Some(true) => {
				self.send(Event::Changed);
				watching.applied = Some(new);
				watching.built.clear_patches();
				self.give_macros()
			}
			Some(false) => true
		}
	}

	/// Synchronize source after reconnection. A reboot loses previous reloads;
	/// a rebuilt `.dmb` establishes a new baseline.
	fn catch_up(&self, watching: &mut Watching) -> bool {
		let dmb_modified = self.dmb_modified();
		if dmb_modified != watching.dmb_modified {
			self.log("using the rebuilt .dmb as the source baseline");
			watching.dmb_modified = dmb_modified;
			match self.parse() {
				Ok(built) => watching.built = built,
				Err(e) => self.log(format!("couldn't read the code: {}", e))
			}
			watching.applied = None;
			watching.edited_since_built.clear();
			return self.send(Event::Changed) && self.give_macros();
		}

		// Whatever was reloaded went with the old world
		watching.applied = None;
		watching.built.clear_patches();
		if watching.edited_since_built.is_empty() {
			return true;
		}

		self.log("world rebooted; restoring source changes since the build");
		let current = match self.parse() {
			Ok(current) => current,
			Err(e) => {
				self.log(format!("couldn't read the code: {}", e));
				return self.not_reloaded(e);
			}
		};
		let changes = diff(&watching.built, &current, Some(&watching.edited_since_built));
		match self.reload(&changes, &current) {
			None => false,
			Some(applied) => {
				if applied {
					watching.applied = Some(current);
					watching.built.clear_patches();
					return self.give_macros();
				}
				true
			}
		}
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

	/// Ask the adapter to apply `changes` against `snapshot`.
	/// Returns `None` if the adapter disconnected.
	fn reload(&self, changes: &Changes, snapshot: &Snapshot) -> Option<bool> {
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
		for why in &changes.warnings {
			self.out.console(format!("    note: {}", why));
		}

		let request = reload_request(changes);
		if request.is_empty() {
			self.log("no runtime changes");
			return self
				.send(Event::NothingChanged {
					warnings: changes.warnings.clone()
				})
				.then_some(true);
		}

		if !self.send(Event::Reload {
			request: Box::new(request),
			summary: summary(changes),
			notes: changes.warnings.clone()
		}) {
			return None;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => return Some(applied),
				// Reboot interrupted delivery; `catch_up` will retry.
				Ok(Command::Connected { .. }) => self.reconnected.set(true),
				Err(_) => return None
			}
		}
	}
}

/// Attach source locations to diagnostics as `file:line: message`.
fn placed(errors: &[String], snapshot: &Snapshot) -> Vec<String> {
	let ctx = snapshot.context();
	let parse_errors: Vec<(String, String)> = ctx
		.errors()
		.iter()
		.filter(|e| e.severity() == dreammaker::Severity::Error)
		.map(|e| {
			let file = ctx.file_path(e.location().file).to_string_lossy().replace('\\', "/");
			(
				format!("parse error: {}: {}", file, e.description()),
				format!("{}:{}: {}", file, e.location().line, e.description())
			)
		})
		.collect();
	errors
		.iter()
		.map(|error| {
			if let Some((_, placed)) = parse_errors.iter().find(|(text, _)| text == error) {
				return placed.clone();
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

/// Summarize a reload in one line.
fn summary(changes: &Changes) -> String {
	let list = |names: Vec<String>, what: &str| match names.len() {
		0 => None,
		1..=5 => Some(names.join(", ")),
		n => Some(format!("{} {}", n, what))
	};
	let parts: Vec<String> = [
		list(changes.new_types.iter().map(|t| t.path.clone()).collect(), "types").map(|s| format!("added {}", s)),
		list(changes.consts.iter().chain(&changes.retyped).cloned().collect(), "vars").map(|s| format!("recompiled users of {}", s)),
		list(changes.reloads.iter().map(|r| r.proc.to_string()).collect(), "procs").map(|s| format!("reloaded {}", s)),
		list(changes.settings.iter().map(|c| c.proc.to_string()).collect(), "procs").map(|s| format!("updated settings for {}", s)),
		list(changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect(), "procs").map(|s| format!("added {}", s)),
		list(changes.removed_procs.iter().map(|p| p.proc.to_string()).collect(), "procs").map(|s| format!("unlinked {}", s)),
		list(
			changes.new_vars.iter().map(|v| format!("{}/var/{}", v.type_path, v.var)).collect(),
			"vars"
		)
		.map(|s| format!("added {}", s)),
		list(
			changes.redeclared.iter().map(|v| format!("{}/var/{}", v.type_path, v.var)).collect(),
			"vars"
		)
		.map(|s| format!("updated declarations for {}", s)),
		list(
			changes
				.var_defaults
				.iter()
				.chain(&changes.builtin_defaults)
				.map(|d| format!("{}/var/{}", d.types[0], d.var))
				.collect(),
			"vars"
		)
		.map(|s| format!("set the default of {}", s)),
		list(changes.init_procs.iter().map(|i| i.type_path.clone()).collect(), "types").map(|s| format!("rebuilt the init procs of {}", s)),
		(!changes.line_moves.is_empty()).then(|| format!("remapped lines in {} additional procs", changes.line_moves.len()))
	]
	.into_iter()
	.flatten()
	.collect();
	parts.join("; ")
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
			.collect()
	}
}

/// The files a parse read, the .dme too
fn files(snapshot: &Snapshot, dme: &Path) -> Vec<PathBuf> {
	let mut files = snapshot.files();
	files.push(dme.to_owned());
	files
}

fn modified_time(root: &Path, file: &Path) -> Option<SystemTime> {
	std::fs::metadata(root.join(file)).and_then(|m| m.modified()).ok()
}

fn modified_times(root: &Path, files: &[PathBuf]) -> HashMap<PathBuf, Option<SystemTime>> {
	files.iter().map(|file| (file.clone(), modified_time(root, file))).collect()
}

/// Files changed since we last looked
fn changed_files(root: &Path, modified: &mut HashMap<PathBuf, Option<SystemTime>>) -> HashSet<PathBuf> {
	let files: Vec<PathBuf> = modified.keys().cloned().collect();
	let now = modified_times(root, &files);
	let changed = now
		.iter()
		.filter(|(file, time)| modified.get(*file) != Some(time))
		.map(|(file, _)| file.clone())
		.collect();
	*modified = now;
	changed
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
