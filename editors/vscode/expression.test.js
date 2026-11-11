// `node expression.test.js`: what a hover takes for an expression

const assert = require("assert");
const { expressionAt } = require("./expression");

// The expression found with the mouse on the `|` in `line`
function at(marked) {
	const column = marked.indexOf("|");
	const line = marked.replace("|", "");
	const found = expressionAt(line, column);
	return found && line.slice(found.start, found.end);
}

assert.strictEqual(at("\tvar/y = |x + 1"), "x");
assert.strictEqual(at("\tvar/y = x| + 1"), "x");
assert.strictEqual(at("\treturn he|alth * 2"), "health");
assert.strictEqual(at("\tsrc.na|me = called"), "src.name");
assert.strictEqual(at("\ts|rc.name = called"), "src");
assert.strictEqual(at("\tmob.loc.conte|nts.len"), "mob.loc.contents");
assert.strictEqual(at("\tmob.loc.contents.l|en"), "mob.loc.contents.len");
assert.strictEqual(at("\tif (target?.cli|ent)"), "target?.client");
assert.strictEqual(at("\tthing:hid|den = 1"), "thing:hidden");
assert.strictEqual(at("\tthing?:hid|den = 1"), "thing?:hidden");
assert.strictEqual(at("\ta . b . |c"), "a . b . c");
// What's reached through a call or an index starts after it
assert.strictEqual(at("\tget_mob().na|me"), undefined);
assert.strictEqual(at("\titems[1].na|me"), undefined);
// Calls aren't made
assert.strictEqual(at("\tsrc.bu|mp(2)"), undefined);
assert.strictEqual(at("\tle|af(x)"), undefined);
assert.strictEqual(at("\tleaf(|x)"), "x");
assert.strictEqual(at("/proc/le|af(x)"), undefined);
// Nor are texts, comments, keywords, numbers or paths values to read
assert.strictEqual(at('\tworld.log << "dap: wor|ld"'), undefined);
assert.strictEqual(at('\tworld.log << "a \\" b|" + c'), undefined);
assert.strictEqual(at('\tworld.log << "a" + |c'), "c");
assert.strictEqual(at("\tx = 1 // then |y"), undefined);
assert.strictEqual(at("\tret|urn x"), undefined);
assert.strictEqual(at("\tvar/|y = 1"), undefined);
assert.strictEqual(at("\tx = 1e|3"), undefined);
assert.strictEqual(at("\tnew /datum/th|ing(x)"), undefined);
assert.strictEqual(at("\tvar/datum/thing/|t = x"), undefined);
// A ternary's colon isn't a member's
assert.strictEqual(at("\treturn a ? b : |c"), "c");
assert.strictEqual(at("\treturn a ? b :|c"), "c");
// Nothing there
assert.strictEqual(at("\tx = | 1"), undefined);
assert.strictEqual(at("|"), undefined);
console.log("ok: 30 hover expressions");
