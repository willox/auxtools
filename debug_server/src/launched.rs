//! Initialize the debug server in worlds started by the launcher.
//!
//! The launcher loads the library through LD_PRELOAD on Linux or
//! `auxtools_debug_attach` on Windows. `AUXTOOLS_DEBUG=wait` delays execution
//! until a client finishes configuration; `run` starts immediately.
//!
//! Helper procs are added at load time. A keepalive loop ensures requests
//! are processed even when no other DM code is running.

use std::{
	collections::{BTreeMap, HashMap},
	sync::{
		atomic::{AtomicBool, Ordering},
		OnceLock
	}
};

use auxtools::{byond_string, Proc, Value, WorldLoad};
use dmasm::reload::{ProcKey, Snapshot};

use crate::{
	connection, hot_reload,
	server::Server,
	server_types::{HotReloadNewProc, HotReloadProc, HotReloadRequest, ProcRef, HELPER_FILE},
	waker
};

/// Set by the launcher: `wait` or `run`
pub const DEBUG_VAR: &str = "AUXTOOLS_DEBUG";

/// Debugger helper procs added at world load. Shutdown runs after the
/// world's `world/Del()`. Reinitialize only after a reboot loads a new world.
const HELPER_PROCS: &str = r#"
/proc/auxtools_expr_stub()
	CRASH("auxtools_expr_stub is where the debugger evaluates expressions")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_keepalive()
	set waitfor = FALSE
	// Queue for spawned evaluations. The keepalive loop calls a proc only
	// when work is pending; a call during reboot could initialize the
	// debugger in the exiting world.
	var/list/inbox = auxtools_spawn_inbox()
	while(TRUE)
		if(inbox && inbox.len)
			inbox.len = 0
			var/list/next = auxtools_spawn_next()
			while(next)
				auxtools_spawned(next[1], next[2])
				next = auxtools_spawn_next()
		sleep(1)

/proc/auxtools_spawn_inbox()
	return null

/proc/auxtools_spawn_next()
	return null

/proc/auxtools_spawn_stub()
	CRASH("auxtools_spawn_stub is where the debugger runs what it's asked to spawn")

/proc/auxtools_spawned(number, list/given)
	set waitfor = FALSE
	auxtools_spawn_done(number, auxtools_spawn_stub(arglist(given)))

/proc/auxtools_spawn_done(number, result)
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_kept()
	// Keep values between evaluations: `auxtools_kept()["m"] = m`.
	var/static/list/kept = list()
	return kept

/proc/auxtools_launched_world_del(failed)
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_launched_world_reboot()
	CRASH("the debugger's library isn't loaded")

/world/Reboot()
	auxtools_launched_world_reboot()
	return ..()

/proc/auxtools_launched_world_new()
	CRASH("the debugger's library isn't loaded")

// Runs once the world has read its data, before any client logs in
/world/New()
	auxtools_launched_world_new()
	return ..()

/world/Del()
	try
		..()
	catch(var/exception/e)
		auxtools_launched_world_del(TRUE)
		throw e
	auxtools_launched_world_del()
"#;

/// Helper overrides excluded from source override numbering
/// (see `Proc::set_apart`). Keep them above reloaded world definitions.
const HELPER_OVERRIDES: [&str; 3] = ["/world/Del", "/world/Reboot", "/world/New"];

/// Set by `world/Reboot` so `world/Del` can distinguish reboot from shutdown.
static REBOOTING: AtomicBool = AtomicBool::new(false);

/// Whether the debugger owns the keepalive loop. Preserve a world's own
/// `auxtools_keepalive` if present.
static KEEPALIVE_OURS: AtomicBool = AtomicBool::new(false);

/// Whether spawned evaluation is available: the loop is ours and its
/// helper procs are hooked.
static SPAWNS: AtomicBool = AtomicBool::new(false);

/// Reason spawned evaluation is unavailable, if any.
pub fn can_spawn() -> Result<(), String> {
	if SPAWNS.load(Ordering::Relaxed) {
		return Ok(());
	}
	if !KEEPALIVE_OURS.load(Ordering::Relaxed) && Proc::find("/proc/auxtools_keepalive").is_some() {
		return Err("this world can't run spawned code: it has an /proc/auxtools_keepalive of its own, which doesn't run it".to_owned());
	}
	Err("this world can't run spawned code: it doesn't have the debugger's helper procs for it".to_owned())
}

/// Identify the debugger helpers that form the outer frames of spawned code.
pub fn runs_spawns(proc: &Proc) -> bool {
	KEEPALIVE_OURS.load(Ordering::Relaxed) && matches!(proc.path.as_str(), "/auxtools_keepalive" | "/auxtools_spawned")
}

static WORLD_LOAD: WorldLoad = WorldLoad { prepare, ready, failed };

/// Whether a launcher started this world
pub fn launched() -> bool {
	std::env::var_os(DEBUG_VAR).is_some()
}

// DreamDaemon's dependencies are loaded by now, libbyond.so among them
#[cfg(unix)]
#[auxtools::ctor::ctor]
fn on_load() {
	if !launched() {
		return;
	}

	// Programs the world runs with shell() aren't BYOND
	std::env::remove_var("LD_PRELOAD");
	attach();
}

/// Windows injection entry point, called on the main thread after
/// byondcore.dll is loaded.
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn auxtools_debug_attach() {
	if launched() {
		attach();
	}
}

/// Original command line and working directory. Capture before
/// DreamDaemon modifies the `.dmb` path in place.
static COMMAND_LINE: OnceLock<(Vec<String>, String)> = OnceLock::new();

/// What the world was started with, which says which .dmb it runs
pub fn command_line() -> (Vec<String>, String) {
	COMMAND_LINE.get().cloned().unwrap_or_else(read_command_line)
}

fn read_command_line() -> (Vec<String>, String) {
	let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
	(std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect(), cwd)
}

fn attach() {
	let _ = COMMAND_LINE.set(read_command_line());
	if let Err(e) = auxtools::init_on_world_load(&WORLD_LOAD) {
		failed(&e);
	}
}

/// Adds the procs in [`HELPER_PROCS`] the world doesn't have
fn prepare() -> Result<(), String> {
	crate::world_resources::loading();
	let request = helper_procs()?;
	KEEPALIVE_OURS.store(
		request
			.new_procs
			.iter()
			.any(|proc| proc.type_path.is_empty() && proc.name == "auxtools_keepalive"),
		Ordering::Relaxed
	);
	if request.is_empty() {
		return Ok(());
	}

	let mut warnings = vec![];
	let prepared = hot_reload::prepare(&request, &mut warnings, &mut vec![]).map_err(|errors| errors.join("; "))?;
	hot_reload::apply(prepared, &mut HashMap::<_, BTreeMap<(), ()>>::new(), &mut HashMap::new(), &mut warnings);
	for warning in warnings {
		eprintln!("auxtools: {}", warning);
	}
	Ok(())
}

fn helper_procs() -> Result<HotReloadRequest, String> {
	let ctx = dreammaker::Context::default();
	let pp = dreammaker::Preprocessor::from_buffer(&ctx, HELPER_FILE.into(), HELPER_PROCS);
	let mut parser = dreammaker::Parser::new(&ctx, pp);
	parser.enable_procs();
	let tree = parser.parse_object_tree();
	let mut snapshot = Snapshot::from_tree(ctx, tree);
	snapshot.set_byond(crate::world_byond());

	let mut request = HotReloadRequest::default();
	let keys: Vec<ProcKey> = snapshot.procs().cloned().collect();
	for key in keys {
		// `/name` for a global proc
		let (type_path, name) = key.path.rsplit_once('/').ok_or_else(|| format!("{} has no name", key.path))?;
		let (type_path, name) = (type_path.to_owned(), name.to_owned());

		// Preserve legacy world hooks and run ours after existing `world/Del`.
		let declared = type_path.is_empty();
		if declared && Proc::find(&key.path).is_some() {
			continue;
		}

		let code = snapshot.compile_reload(&key).map_err(|e| format!("{}: {}", key.path, e))?;
		let override_id = if declared { 0 } else { overrides(&key.path) };
		request.new_procs.push(HotReloadNewProc {
			type_path,
			name,
			declared,
			verb: false,
			code: HotReloadProc {
				proc: ProcRef {
					path: key.path.clone(),
					override_id
				},
				nodes: code.nodes,
				locals: code.locals,
				params: code.params,
				settings: code.settings,
				statics: code.statics
			}
		});
	}
	Ok(request)
}

/// How many definitions a proc has
fn overrides(path: &str) -> u32 {
	(0..).take_while(|&id| Proc::find_override(path, id).is_some()).count() as u32
}

fn ready() {
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_del") {
		let _ = proc.hook(world_del);
	}
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_reboot") {
		let _ = proc.hook(world_reboot);
	}
	// Without the hook nothing would say the world has read its resources
	if !Proc::find("/proc/auxtools_launched_world_new").is_some_and(|proc| proc.hook(world_new).is_ok()) {
		crate::world_resources::loaded();
	}
	REBOOTING.store(false, Ordering::Relaxed);
	let hooked =
		|path: &str, hook: fn(&Value, &Value, Vec<Value>) -> auxtools::DMResult| Proc::find(path).is_some_and(|proc| proc.hook(hook).is_ok());
	let spawns = KEEPALIVE_OURS.load(Ordering::Relaxed)
		& hooked("/proc/auxtools_spawn_inbox", spawn_inbox)
		& hooked("/proc/auxtools_spawn_next", spawn_next)
		& hooked("/proc/auxtools_spawn_done", spawn_done)
		&& ["/proc/auxtools_spawn_stub", "/proc/auxtools_spawned"]
			.into_iter()
			.all(|path| Proc::find(path).is_some());
	SPAWNS.store(spawns, Ordering::Relaxed);
	for path in HELPER_OVERRIDES {
		// Ours is the last
		if let Some(proc) = overrides(path).checked_sub(1).and_then(|last| Proc::find_override(path, last)) {
			proc.set_apart();
			hot_reload::pin_world_proc(proc.id.0);
		}
	}

	// Where to wake it if it sleeps with no players (see `waker`)
	if let Ok(port) = Value::WORLD.get_number(byond_string!("port")) {
		waker::set_port(port as u16);
	}
	// Record the load before accepting requests so reconnecting clients
	// observe the reboot immediately.
	crate::journal::world_loaded();
	let mut server = match Server::listen() {
		Ok(server) => server,
		Err(e) => {
			eprintln!("auxtools: couldn't listen for debuggers at {}: {}", connection::world_name(), e);
			return;
		}
	};
	if std::env::var(DEBUG_VAR).is_ok_and(|value| value == "wait") {
		server.process_until_configured();
	}
	crate::install_server(server);

	// It sleeps straight away, which returns here
	if let Some(proc) = Proc::find("/proc/auxtools_keepalive") {
		let _ = proc.call(&[]);
	}
}

/// The world has read its data: resources get their own ids from here on
fn world_new(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	crate::world_resources::loaded();
	if let Some(server) = unsafe { &mut *crate::DEBUG_SERVER.get() } {
		server.world_data_loaded();
	}
	Ok(Value::NULL)
}

/// Handle world/Del completion; `failed` indicates a runtime error.
fn world_del(_: &Value, _: &Value, args: Vec<Value>) -> auxtools::DMResult {
	let failed = args.first().is_some_and(|failed| failed.as_number().is_ok_and(|failed| failed != 0.0));
	if REBOOTING.swap(false, Ordering::Relaxed) {
		// Reboot proceeds even if Del throws. The post-load helper
		// distinguishes this world from the next one.
		let ours = Proc::find_override("/world/Del", Proc::APART);
		auxtools::init_on_next_world_load(ours.as_ref());
	} else if !failed {
		if let Some(server) = unsafe { &mut *crate::DEBUG_SERVER.get() } {
			server.world_shutting_down();
		}
		auxtools::shut_down_with_world();
	}
	// A failed Del leaves the world running and must preserve its debugger.
	Ok(Value::NULL)
}

fn world_reboot(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	REBOOTING.store(true, Ordering::Relaxed);
	Ok(Value::NULL)
}

/// The list the world's loop looks at for spawned code to run
fn spawn_inbox(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	match unsafe { &mut *crate::DEBUG_SERVER.get() } {
		Some(server) => Ok(server.spawn_inbox()),
		None => Ok(Value::NULL)
	}
}

/// Dequeue spawned code, installing it in `/proc/auxtools_spawn_stub`.
/// Return its number and arguments, or null if the queue is empty.
fn spawn_next(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	match unsafe { &mut *crate::DEBUG_SERVER.get() } {
		Some(server) => server.next_spawn(),
		None => Ok(Value::NULL)
	}
}

/// Complete a spawned evaluation with its result list, or null on failure.
fn spawn_done(_: &Value, _: &Value, args: Vec<Value>) -> auxtools::DMResult {
	let number = args.first().and_then(|number| number.as_number().ok());
	if let (Some(server), Some(number)) = (unsafe { &mut *crate::DEBUG_SERVER.get() }, number) {
		server.spawn_done(number as u64, args.get(1).unwrap_or(&Value::NULL));
	}
	Ok(Value::NULL)
}

/// Report initialization failure to connecting clients.
fn failed(error: &str) {
	eprintln!("auxtools: couldn't start the debugger: {}", error);
	Server::refuse(error.to_owned());
}
