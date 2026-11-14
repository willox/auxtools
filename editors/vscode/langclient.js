// Use the DM language client's selected environment unless launch settings
// specify another world (github.com/SpaceManiac/vscode-dm-langclient).

"use strict";

const path = require("path");

// DM language client extension ID.
const EXTENSION = "platymuus.dm-langclient";

// Whether the DM language client is installed.
function installed(vscode) {
	return Boolean(vscode.extensions.getExtension(EXTENSION));
}

// Resolve the language client's .dme and adjacent .dmb under `folder`.
// Return undefined when the extension or its environment is unavailable.
async function environment(vscode, folder) {
	const extension = vscode.extensions.getExtension(EXTENSION);
	folder = folder || (vscode.workspace.workspaceFolders || [])[0];
	if (!extension || !folder) {
		return undefined;
	}
	try {
		await extension.activate();
		const dme = await vscode.commands.executeCommand("dreammaker.getFilenameDme");
		if (typeof dme !== "string" || !dme.endsWith(".dme")) {
			return undefined;
		}
		const full = path.join(folder.uri.fsPath, dme);
		return { dme: full, dmb: full.replace(/\.dme$/, ".dmb") };
	} catch {
		return undefined;
	}
}

module.exports = { installed, environment };
