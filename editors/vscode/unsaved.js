// Track unsaved files for the launcher's auxtoolsUnsaved request.
// DAP breakpoint lines refer to editor text, which may differ from disk
// and running code. Defer those breakpoints until save and reload.

// Only file-backed documents can be reported to a session.
function pathOf(document) {
	return document.uri && document.uri.scheme === "file" ? document.uri.fsPath : undefined;
}

class UnsavedFiles {
	constructor() {
		this.paths = new Set();
	}

	// Return a changed document state as { path, unsaved }, or undefined.
	changed(document) {
		const path = pathOf(document);
		if (path === undefined) {
			return undefined;
		}
		const unsaved = document.isDirty && !document.isClosed;
		if (unsaved === this.paths.has(path)) {
			return undefined;
		}
		if (unsaved) {
			this.paths.add(path);
		} else {
			this.paths.delete(path);
		}
		return { path, unsaved };
	}

	// Return all unsaved files when a session starts.
	all() {
		return [...this.paths].map((path) => ({ path, unsaved: true }));
	}
}

module.exports = { UnsavedFiles };
