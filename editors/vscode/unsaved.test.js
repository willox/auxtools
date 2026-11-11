// `node unsaved.test.js`: what the launcher is told of files with unsaved edits

const assert = require("assert");
const { UnsavedFiles } = require("./unsaved");

function document(fsPath, isDirty, scheme = "file") {
	return { uri: { scheme, fsPath }, isDirty, isClosed: false };
}

const files = new UnsavedFiles();
assert.deepStrictEqual(files.changed(document("/w/a.dm", false)), undefined);
assert.deepStrictEqual(files.changed(document("/w/a.dm", true)), { path: "/w/a.dm", unsaved: true });
// Further edits do not change the unsaved state.
assert.deepStrictEqual(files.changed(document("/w/a.dm", true)), undefined);
assert.deepStrictEqual(files.changed(document("/w/b.dm", true)), { path: "/w/b.dm", unsaved: true });
assert.deepStrictEqual(files.all(), [
	{ path: "/w/a.dm", unsaved: true },
	{ path: "/w/b.dm", unsaved: true },
]);
assert.deepStrictEqual(files.changed(document("/w/a.dm", false)), { path: "/w/a.dm", unsaved: false });
assert.deepStrictEqual(files.changed(document("/w/a.dm", false)), undefined);
// Closing without saving restores the on-disk breakpoint baseline.
const closed = document("/w/b.dm", true);
closed.isClosed = true;
assert.deepStrictEqual(files.changed(closed), { path: "/w/b.dm", unsaved: false });
assert.deepStrictEqual(files.all(), []);
// Exclude untitled and non-file documents.
assert.deepStrictEqual(files.changed(document("Untitled-1", true, "untitled")), undefined);
assert.deepStrictEqual(files.changed({ uri: { scheme: "git", fsPath: "/w/a.dm" }, isDirty: true }), undefined);

console.log("ok: unsaved file notifications");
