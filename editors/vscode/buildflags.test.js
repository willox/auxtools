// Run with `node buildflags.test.js`.

const assert = require("assert");
const { defines } = require("./buildflags");

assert.deepStrictEqual(defines(""), []);
assert.deepStrictEqual(defines("-DLOWMEMORYMODE"), ["LOWMEMORYMODE"]);
assert.deepStrictEqual(defines("-DA -DB=2  -DC"), ["A", "B=2", "C"]);
assert.deepStrictEqual(defines('-DMAP="tram station" -DTESTING'), ['MAP="tram station"', "TESTING"]);
assert.deepStrictEqual(defines("-D -X -DOK"), ["OK"]);

console.log("ok: buildflags");
