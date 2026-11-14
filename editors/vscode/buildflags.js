// Read SS13 Build Flags defines for hot reload
// (github.com/spacestation13/SS13-BuildFlags). They must match the build.

"use strict";

// SS13 Build Flags extension ID.
const EXTENSION = "ss13.ss13buildflags";

// Parse DreamMaker arguments (`-DNAME -DNAME=VALUE -DNAME="a b"`) into
// `NAME` or `NAME=VALUE`. Preserve quotes around DM string values.
function defines(text) {
	const found = [];
	let token = "";
	let quoted = false;
	for (const char of text) {
		if (char === '"') {
			quoted = !quoted;
		}
		if (!quoted && /\s/.test(char)) {
			if (token) {
				found.push(token);
			}
			token = "";
			continue;
		}
		token += char;
	}
	if (token) {
		found.push(token);
	}
	return found.filter((arg) => arg.startsWith("-D") && arg.length > 2).map((arg) => arg.slice(2));
}

// Read selected command-line defines, or return undefined when the
// extension is absent or injects defines into an included source file.
async function selected(vscode) {
	if (!vscode.extensions.getExtension(EXTENSION)) {
		return undefined;
	}
	if (vscode.workspace.getConfiguration("ss13BuildFlags").get("injectionMode", "cli-args") !== "cli-args") {
		return undefined;
	}
	try {
		const current = await vscode.commands.executeCommand("ss13BuildFlags.current");
		return typeof current === "string" ? defines(current) : undefined;
	} catch {
		return undefined;
	}
}

module.exports = { defines, selected };
