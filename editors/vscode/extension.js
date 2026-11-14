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

const DEBUG_TYPE = "auxtools";

function settings() {
	return vscode.workspace.getConfiguration("auxtools");
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

// One built world (.dmb) to run, asking when there's a choice
async function pickWorldFile(folder) {
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
// Dream Seeker-hosted worlds must reboot instead.
async function reconnectClients(session) {
	session = session || vscode.debug.activeDebugSession;
	if (!session || session.type !== DEBUG_TYPE) {
		vscode.window.showErrorMessage("No active auxtools debug session.");
		return;
	}
	try {
		const done = await session.customRequest("auxtoolsReconnect");
		if (done && done.rebooted) {
			vscode.window.showInformationMessage("Rebooting the Dream Seeker-hosted world.");
		} else if (done) {
			vscode.window.showInformationMessage(`Asked ${done.reconnected} ${done.reconnected === 1 ? "player" : "players"} to reconnect.`);
		}
	} catch (e) {
		vscode.window.showErrorMessage(`Could not reconnect the players: ${e.message}`);
	}
}

// Offer reconnect after a skin reload.
async function skinReloaded(event) {
	const { file, clients, hosted } = event.body;
	if (!clients) {
		return;
	}
	const message = hosted
		? `Skin ${file} reloaded. Reboot to update Dream Seeker's interface.`
		: `Skin ${file} reloaded. ${clients} connected ${clients === 1 ? "player needs" : "players need"} to reconnect to load the new interface.`;
	const action = hosted ? "Reboot" : "Reconnect";
	if ((await vscode.window.showInformationMessage(message, action)) === action) {
		await reconnectClients(event.session);
	}
}

// Offer reconnect after a client script reload.
async function scriptReloaded(event) {
	const { clients, hosted } = event.body;
	if (!clients) {
		return;
	}
	const message = hosted
		? "Client script reloaded. Reboot to update Dream Seeker's script."
		: `Client script reloaded. ${clients} connected ${clients === 1 ? "player needs" : "players need"} to reconnect to load the new script.`;
	const action = hosted ? "Reboot" : "Reconnect";
	if ((await vscode.window.showInformationMessage(message, action)) === action) {
		await reconnectClients(event.session);
	}
}

function activate(context) {
	const provider = new ConfigurationProvider(context);
	trackUnsavedFiles(context);
	context.subscriptions.push(
		vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, new AdapterFactory(context)),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider, vscode.DebugConfigurationProviderTriggerKind.Dynamic),
		vscode.commands.registerCommand("auxtools.debugDreamDaemon", debugInDreamDaemon),
		vscode.commands.registerCommand("auxtools.attach", attach),
		vscode.commands.registerCommand("auxtools.reconnectClients", () => reconnectClients()),
		vscode.debug.onDidReceiveDebugSessionCustomEvent((event) => {
			if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsSkin") {
				skinReloaded(event);
			} else if (event.session.type === DEBUG_TYPE && event.event === "auxtoolsScript") {
				scriptReloaded(event);
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
