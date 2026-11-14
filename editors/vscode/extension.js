// Register the auxtools launcher as a VS Code debug adapter, supply
// launch/attach configurations, and report editor state.
//
// VS Code provides the vscode module; no additional dependencies are needed.

"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");
const { expressionAt } = require("./expression");
const { UnsavedFiles } = require("./unsaved");
const buildFlags = require("./buildflags");
const langclient = require("./langclient");

const DEBUG_TYPE = "auxtools";

function settings() {
	return vscode.workspace.getConfiguration("auxtools");
}

// Whether this notification category is enabled.
function notifies(kind) {
	const chosen = settings().get("notifications") || {};
	return chosen[kind] !== false;
}

// Resolve the launcher from settings or bundled platform binaries.
function launcherPath(context) {
	const configured = settings().get("launcherPath");
	if (configured) {
		return configured;
	}
	const exe = process.platform === "win32" ? "auxtools.exe" : "auxtools";
	return path.join(context.extensionPath, "bin", process.platform, exe);
}

// A packaged extension's files can lose their executable bit on Linux
function ensureExecutable(file) {
	if (process.platform === "win32") {
		return;
	}
	try {
		fs.accessSync(file, fs.constants.X_OK);
	} catch {
		try {
			fs.chmodSync(file, 0o755);
		} catch {
			// Let startup report any error if chmod fails.
		}
	}
}

// Add the launcher directory to terminal PATH. Existing terminals need
// to be relaunched to pick up the change.
function exposeLauncher(context) {
	const collection = context.environmentVariableCollection;
	collection.clear();
	if (!settings().get("addToTerminalPath")) {
		return;
	}
	const launcher = launcherPath(context);
	if (!fs.existsSync(launcher)) {
		return;
	}
	ensureExecutable(launcher);
	collection.description = "Adds the auxtools launcher directory to PATH.";
	collection.prepend("PATH", path.dirname(launcher) + path.delimiter);
}

// Find project files, preferring the workspace-folder name and shallow paths.
async function findFiles(extension, folder) {
	const glob = `**/*${extension}`;
	const pattern = folder ? new vscode.RelativePattern(folder, glob) : glob;
	const found = await vscode.workspace.findFiles(pattern, "**/node_modules/**", 50);
	const score = (uri) => {
		const owner = vscode.workspace.getWorkspaceFolder(uri);
		const named = owner && path.basename(uri.fsPath, extension) === owner.name ? 0 : 1;
		return [named, uri.fsPath.split(/[\\/]/).length];
	};
	return found.sort((a, b) => {
		const [x, y] = [score(a), score(b)];
		return x[0] - y[0] || x[1] - y[1] || a.fsPath.localeCompare(b.fsPath);
	});
}

// Choose the language client's world, falling back to workspace .dmb files.
async function pickWorldFile(folder) {
	const current = await langclient.environment(vscode, folder);
	if (current && fs.existsSync(current.dmb)) {
		return vscode.Uri.file(current.dmb);
	}
	const found = await findFiles(".dmb", folder);
	if (found.length === 0) {
		vscode.window.showErrorMessage("No .dmb found in the workspace. Build the world first (Tasks: Run Build Task).");
		return undefined;
	}
	if (found.length === 1) {
		return found[0];
	}
	const picked = await vscode.window.showQuickPick(
		found.map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })),
		{ placeHolder: "Select a world" }
	);
	return picked && picked.uri;
}

// The running worlds that can be attached to, from `auxtools list --json`
function listWorlds(context) {
	const launcher = launcherPath(context);
	ensureExecutable(launcher);
	return new Promise((resolve, reject) => {
		childProcess.execFile(launcher, ["list", "--json"], { timeout: 30000 }, (error, stdout) => {
			if (error) {
				reject(error);
				return;
			}
			try {
				resolve(JSON.parse(stdout));
			} catch (e) {
				reject(e);
			}
		});
	});
}

// One running world's process id, asking when there's a choice
async function pickWorld(context) {
	let worlds;
	try {
		worlds = await listWorlds(context);
	} catch (e) {
		vscode.window.showErrorMessage(`Could not list running worlds: ${e.message}`);
		return undefined;
	}
	if (worlds.length === 0) {
		vscode.window.showErrorMessage("No debugger-enabled worlds found. Start a world with the auxtools launcher.");
		return undefined;
	}
	const items = worlds.map((world) => ({
		label: world.dmb ? path.basename(world.dmb) : world.program,
		description: `process ${world.pid}`,
		detail: world.dmb || world.args.join(" "),
		pid: String(world.pid),
	}));
	if (items.length === 1) {
		return items[0].pid;
	}
	const picked = await vscode.window.showQuickPick(items, { placeHolder: "Select a world" });
	return picked && picked.pid;
}

// A launch configuration for a .dmb. Its .dme is the one next to it.
function launchConfiguration(dmb, name) {
	return {
		type: DEBUG_TYPE,
		request: "launch",
		name: name || `Debug ${path.basename(dmb)}`,
		dmb,
		server: "dreamdaemon",
	};
}

class ConfigurationProvider {
	constructor(context) {
		this.context = context;
	}

	async provideDebugConfigurations(folder) {
		// Use the language client's commands and build task. Its CurrentDME
		// and CurrentDMB variables belong to its own debugger.
		if (langclient.installed(vscode)) {
			return [
				{
					...launchConfiguration("${workspaceFolder}/${command:dreammaker.getFilenameDmb}", "Debug in DreamDaemon"),
					dme: "${workspaceFolder}/${command:dreammaker.getFilenameDme}",
					preLaunchTask: "dm: build - ${command:dreammaker.getFilenameDme}",
				},
			];
		}
		const found = await findFiles(".dmb", folder);
		const dmb = found.length > 0 ? found[0].fsPath : "${workspaceFolder}/${workspaceFolderBasename}.dmb";
		return [launchConfiguration(dmb, "Debug in DreamDaemon")];
	}

	// Resolve missing configuration values, including launches without launch.json.
	async resolveDebugConfiguration(folder, config) {
		if (!config.type && !config.request && !config.name) {
			const dmb = await pickWorldFile(folder);
			if (!dmb) {
				return undefined;
			}
			config = launchConfiguration(dmb.fsPath);
		}
		// Use the language client's environment when no world is specified.
		if (config.request === "launch" && !config.dmb && !config.dme) {
			const current = await langclient.environment(vscode, folder);
			if (current) {
				config.dmb = current.dmb;
				config.dme = current.dme;
			}
		}
		if (config.request === "attach" && !config.processId) {
			config.processId = await pickWorld(this.context);
			if (!config.processId) {
				return undefined;
			}
		}
		if (!config.byondPath && settings().get("byondPath")) {
			config.byondPath = settings().get("byondPath");
		}
		if (!config.debugLibrary && settings().get("debugLibrary")) {
			config.debugLibrary = settings().get("debugLibrary");
		}
		// Hot reload needs the same defines used for the build.
		if (config.defines === undefined) {
			const selected = await buildFlags.selected(vscode);
			if (selected) {
				config.defines = selected;
			}
		}
		return config;
	}

	// Explicit SS13 Build Flags substitutions return several -D arguments
	// in one string; split them before passing them to the parser.
	resolveDebugConfigurationWithSubstitutedVariables(folder, config) {
		if (Array.isArray(config.defines)) {
			config.defines = config.defines.flatMap((define) => (/^\s*-D/.test(define) ? buildFlags.defines(define) : define.trim() ? [define] : []));
		}
		return config;
	}
}

class AdapterFactory {
	constructor(context) {
		this.context = context;
	}

	createDebugAdapterDescriptor() {
		const launcher = launcherPath(this.context);
		if (!fs.existsSync(launcher)) {
			vscode.window.showErrorMessage(`Auxtools launcher not found: ${launcher}.`);
			return undefined;
		}
		ensureExecutable(launcher);
		return new vscode.DebugAdapterExecutable(launcher, ["dap"]);
	}
}

// Attaches to a running world, picked from a list
async function attach() {
	await vscode.debug.startDebugging(undefined, {
		type: DEBUG_TYPE,
		request: "attach",
		name: "Attach to a world",
	});
}

// Debug the selected world in DreamDaemon.
async function debugInDreamDaemon() {
	const editor = vscode.window.activeTextEditor;
	const folder = editor ? vscode.workspace.getWorkspaceFolder(editor.document.uri) : undefined;
	const dmb = await pickWorldFile(folder);
	if (!dmb) {
		return;
	}
	await vscode.debug.startDebugging(vscode.workspace.getWorkspaceFolder(dmb), launchConfiguration(dmb.fsPath));
}

// Report unsaved files so the launcher can distinguish editor line numbers
// from the running source.
function trackUnsavedFiles(context) {
	const unsaved = new UnsavedFiles();
	const sessions = new Set();
	const tell = (session, message) => {
		// Older launchers reject the extension request; ignore that response.
		session.customRequest("auxtoolsUnsaved", message).then(undefined, () => {});
	};
	const update = (document) => {
		const message = unsaved.changed(document);
		if (message) {
			sessions.forEach((session) => tell(session, message));
		}
	};
	context.subscriptions.push(
		vscode.workspace.onDidChangeTextDocument((event) => update(event.document)),
		vscode.workspace.onDidSaveTextDocument(update),
		vscode.workspace.onDidCloseTextDocument(update),
		vscode.debug.onDidStartDebugSession((session) => {
			if (session.type !== DEBUG_TYPE) {
				return;
			}
			vscode.workspace.textDocuments.forEach(update);
			sessions.add(session);
			unsaved.all().forEach((message) => tell(session, message));
		}),
		vscode.debug.onDidTerminateDebugSession((session) => sessions.delete(session))
	);
	vscode.workspace.textDocuments.forEach(update);
}

// Reconnect players to load an interface update.
// DreamSeeker-hosted worlds must reboot instead.
async function reconnectClients(session) {
	session = session || vscode.debug.activeDebugSession;
	if (!session || session.type !== DEBUG_TYPE) {
		vscode.window.showErrorMessage("No active auxtools debug session.");
		return;
	}
	try {
		const done = await session.customRequest("auxtoolsReconnect");
		if (done && done.rebooted) {
			vscode.window.showInformationMessage("Rebooting the DreamSeeker-hosted world.");
		} else if (done) {
			vscode.window.showInformationMessage(`Asked ${done.reconnected} ${done.reconnected === 1 ? "player" : "players"} to reconnect.`);
		}
	} catch (e) {
		vscode.window.showErrorMessage(`Could not reconnect the players: ${e.message}`);
	}
}

// Reboot the active session's world without relaunching its process.
async function reboot() {
	const session = vscode.debug.activeDebugSession;
	if (!session || session.type !== DEBUG_TYPE) {
		vscode.window.showErrorMessage("No active auxtools debug session.");
		return;
	}
	try {
		await session.customRequest("auxtoolsReboot");
	} catch (e) {
		vscode.window.showErrorMessage(`Could not reboot the world: ${e.message}`);
	}
}

// Offer the address when DreamSeeker is unavailable.
async function offerAddress(address, started) {
	const message = started ? `The world started at ${address}.` : `The world is at ${address}.`;
	if ((await vscode.window.showInformationMessage(message, "Copy Address")) === "Copy Address") {
		await vscode.env.clipboard.writeText(address);
	}
}

// Join with DreamSeeker on Windows; otherwise offer the address.
async function join(session) {
	session = session || vscode.debug.activeDebugSession;
	if (!session || session.type !== DEBUG_TYPE) {
		vscode.window.showErrorMessage("No active auxtools debug session.");
		return;
	}
	try {
		const joined = await session.customRequest("auxtoolsJoin", { closeWithWorld: settings().get("closeDreamSeeker") });
		if (!joined.opened) {
			await offerAddress(joined.address, false);
		}
	} catch (e) {
		vscode.window.showErrorMessage(`Could not join the world: ${e.message}`);
	}
}

// Join newly launched worlds, or offer the join action.
async function worldStarted(event) {
	const { address, started, canJoin, hosted } = event.body;
	if (!started || !address || hosted || event.session.configuration.request !== "launch") {
		return;
	}
	if (canJoin && settings().get("autoJoin")) {
		await join(event.session);
		return;
	}
	if (!notifies("join")) {
		return;
	}
	if (!canJoin) {
		await offerAddress(address, true);
	} else if ((await vscode.window.showInformationMessage(`The world started at ${address}.`, "Join with DreamSeeker")) === "Join with DreamSeeker") {
		await join(event.session);
	}
}

// Status bar labels and tooltips by reload state.
const STATES = {
	reading: { text: "$(sync~spin) Reading source", tooltip: "Reading source for hot reload." },
	active: { text: "$(check) Hot reload", tooltip: "Save files to reload supported changes." },
	reloading: { text: "$(sync~spin) Reloading", tooltip: "Applying saved changes." },
	reloaded: { text: "$(check) Hot reload", tooltip: "Last reload: " },
	refused: {
		text: "$(warning) Rebuild needed",
		tooltip: "Rebuild required. See the Problems panel, then rebuild and restart.",
		background: "statusBarItem.warningBackground",
	},
	failed: { text: "$(error) Hot reload failed", tooltip: "The last hot reload failed: ", background: "statusBarItem.errorBackground" },
	off: { text: "$(circle-slash) Hot reload off", tooltip: "Hot reload is off: " },
};

// Track reload state per session. Show the active session in the status
// bar and refused changes in the Problems panel.
class ReloadStatus {
	constructor(context) {
		this.sessions = new Map();
		this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
		this.item.command = "auxtools.menu";
		this.problems = vscode.languages.createDiagnosticCollection("auxtools");
		context.subscriptions.push(
			this.item,
			this.problems,
			vscode.debug.onDidStartDebugSession((session) => {
				if (session.type === DEBUG_TYPE) {
					this.sessions.set(session.id, {});
					this.show();
				}
			}),
			vscode.debug.onDidTerminateDebugSession((session) => {
				if (this.sessions.delete(session.id)) {
					this.problems.clear();
					this.show();
				}
			}),
			vscode.debug.onDidChangeActiveDebugSession(() => this.show())
		);
	}

	// Update the session's reload state and its outstanding refusals.
	changed(event) {
		const { state, summary, problems } = event.body;
		const known = this.sessions.get(event.session.id) || {};
		this.sessions.set(event.session.id, { state, summary, said: known.said, shown: known.shown });
		this.refused(event.session, problems || []);
		this.show();
	}

	// Show outstanding refusals in Problems, and notify once per distinct set.
	refused(session, problems) {
		const known = this.sessions.get(session.id);
		if (!problems.length) {
			// Leave other sessions' entries alone.
			if (known.shown) {
				this.problems.clear();
			}
			known.shown = false;
			known.said = undefined;
			return;
		}
		this.problems.clear();
		known.shown = true;
		const byFile = new Map();
		for (const problem of problems) {
			if (!problem.file) {
				continue;
			}
			const line = Math.max((problem.line || 1) - 1, 0);
			const diagnostic = new vscode.Diagnostic(
				new vscode.Range(line, 0, line, 1000),
				`${problem.message}. Rebuild required.`,
				vscode.DiagnosticSeverity.Warning
			);
			diagnostic.source = "hot reload";
			const uri = vscode.Uri.file(problem.file);
			const entry = byFile.get(uri.toString()) || [uri, []];
			entry[1].push(diagnostic);
			byFile.set(uri.toString(), entry);
		}
		for (const [uri, diagnostics] of byFile.values()) {
			this.problems.set(uri, diagnostics);
		}

		const said = problems.map((problem) => problem.message).join("\n");
		if (!notifies("rebuild") || known.said === said) {
			return;
		}
		known.said = said;
		const more = problems.length > 1 ? ` (and ${problems.length - 1} more)` : "";
		vscode.window
			.showWarningMessage(`Rebuild required: ${problems[0].message}${more}. Restart after rebuilding.`, "Restart", "Show Problems")
			.then((action) => {
				if (action === "Restart") {
					vscode.commands.executeCommand("workbench.action.debug.restart");
				} else if (action === "Show Problems") {
					vscode.commands.executeCommand("workbench.actions.view.problems");
				}
			});
	}

	show() {
		const session = vscode.debug.activeDebugSession;
		const known = session && this.sessions.get(session.id);
		if (!known) {
			this.item.hide();
			return;
		}
		const shown = STATES[known.state] || { text: "$(debug-alt) auxtools", tooltip: "" };
		this.item.text = shown.text;
		this.item.tooltip = `${shown.tooltip}${shown.tooltip.endsWith(": ") ? known.summary || "" : ""}\nClick for world actions.`.trim();
		this.item.backgroundColor = shown.background ? new vscode.ThemeColor(shown.background) : undefined;
		this.item.show();
	}

	state(session) {
		return (session && this.sessions.get(session.id)) || {};
	}
}

// Warn about edits that may be missing from the build.
async function outdated(event) {
	const { files } = event.body;
	if (!files || !files.length || !notifies("outdated")) {
		return;
	}
	const named = files.slice(0, 3).join(", ") + (files.length > 3 ? ` and ${files.length - 3} more` : "");
	const message = `Files changed since the build: ${named}. Rebuild and restart to include any missing edits.`;
	if ((await vscode.window.showWarningMessage(message, "Restart")) === "Restart") {
		await vscode.commands.executeCommand("workbench.action.debug.restart");
	}
}

// World actions for the active session.
async function worldMenu(status) {
	const session = vscode.debug.activeDebugSession;
	if (!session || session.type !== DEBUG_TYPE) {
		return;
	}
	const items = [
		{ label: "$(person-add) Join with DreamSeeker", run: () => join(session) },
		{ label: "$(plug) Reconnect players", run: () => reconnectClients(session) },
		{ label: "$(debug-restart) Restart", detail: "Run preLaunchTask, then restart the world", run: () => vscode.commands.executeCommand("workbench.action.debug.restart") },
		{ label: "$(refresh) Reboot world", run: () => reboot() },
		{ label: "$(debug-stop) Stop", run: () => vscode.commands.executeCommand("workbench.action.debug.stop") },
	];
	if (status.state(session).state === "refused") {
		items.unshift({ label: "$(warning) Show Problems", run: () => vscode.commands.executeCommand("workbench.actions.view.problems") });
	}
	const picked = await vscode.window.showQuickPick(items, { placeHolder: session.name });
	if (picked) {
		await picked.run();
	}
}

// Offer reconnect after a skin reload.
async function skinReloaded(event) {
	const { file, clients, hosted } = event.body;
	if (!clients || !notifies("reconnect")) {
		return;
	}
	const message = hosted
		? `Skin ${file} updated. Reboot the world to update DreamSeeker's interface.`
		: `Skin ${file} reloaded. ${clients} connected ${clients === 1 ? "player needs" : "players need"} to reconnect to load the new interface.`;
	const action = hosted ? "Reboot" : "Reconnect";
	if ((await vscode.window.showInformationMessage(message, action)) === action) {
		await reconnectClients(event.session);
	}
}

// Offer reconnect after a client script reload.
async function scriptReloaded(event) {
	const { clients, hosted } = event.body;
	if (!clients || !notifies("reconnect")) {
		return;
	}
	const message = hosted
		? "Client script updated. Reboot the world to update DreamSeeker's script."
		: `Client script reloaded. ${clients} connected ${clients === 1 ? "player needs" : "players need"} to reconnect to load the new script.`;
	const action = hosted ? "Reboot" : "Reconnect";
	if ((await vscode.window.showInformationMessage(message, action)) === action) {
		await reconnectClients(event.session);
	}
}

function activate(context) {
	const provider = new ConfigurationProvider(context);
	const status = new ReloadStatus(context);
	trackUnsavedFiles(context);
	exposeLauncher(context);
	context.subscriptions.push(
		vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, new AdapterFactory(context)),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider, vscode.DebugConfigurationProviderTriggerKind.Dynamic),
		vscode.commands.registerCommand("auxtools.debugDreamDaemon", debugInDreamDaemon),
		vscode.commands.registerCommand("auxtools.attach", attach),
		vscode.commands.registerCommand("auxtools.reconnectClients", () => reconnectClients()),
		vscode.commands.registerCommand("auxtools.join", () => join()),
		vscode.commands.registerCommand("auxtools.reboot", reboot),
		vscode.commands.registerCommand("auxtools.menu", () => worldMenu(status)),
		vscode.workspace.onDidChangeConfiguration((event) => {
			if (event.affectsConfiguration("auxtools.launcherPath") || event.affectsConfiguration("auxtools.addToTerminalPath")) {
				exposeLauncher(context);
			}
		}),
		vscode.debug.onDidReceiveDebugSessionCustomEvent((event) => {
			if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsSkin") {
				skinReloaded(event);
			} else if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsScript") {
				scriptReloaded(event);
			} else if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsWorld") {
				worldStarted(event);
			} else if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsReload") {
				status.changed(event);
			} else if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsOutdated") {
				outdated(event);
			}
		}),
		// For `${command:auxtools.pickWorld}` in attach configurations
		vscode.commands.registerCommand("auxtools.pickWorld", () => pickWorld(context)),
		// Provide a read-only expression for hover evaluation.
		vscode.languages.registerEvaluatableExpressionProvider(
			{ language: "dm" },
			{
				provideEvaluatableExpression(document, position) {
					const found = expressionAt(document.lineAt(position.line).text, position.character);
					if (!found) {
						return undefined;
					}
					return new vscode.EvaluatableExpression(new vscode.Range(position.line, found.start, position.line, found.end));
				},
			}
		)
	);
}

function deactivate() {}

module.exports = { activate, deactivate };
