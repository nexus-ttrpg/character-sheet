// Phase 3.1.1 spike: proves the wasm-pack-built `sync-engine-wasm` cdylib is
// callable from JS and `diff` round-trips correctly. Standalone — run with
// `node crates/sync-engine-wasm/spike-test.cjs` after
// `wasm-pack build crates/sync-engine-wasm --target nodejs`.
const { diff } = require("./pkg/sync_engine_wasm.js");
const fs = require("fs");
const path = require("path");

const minimal = fs.readFileSync(
  path.join(__dirname, "fixtures/minimal-build.json"),
  "utf8",
);

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log("  ok   -", name);
  } else {
    console.log("  FAIL -", name);
    failures++;
  }
}

// 1. Identical builds → empty diffs, no error.
const same = JSON.parse(diff(minimal, minimal));
check("identical builds: no error field", same.error === undefined);
check(
  "identical builds: empty diffs",
  Array.isArray(same.diffs) && same.diffs.length === 0,
);

// 2. Single name change → one `modified` at `build.identity.name`.
const a = JSON.parse(minimal);
const b = JSON.parse(minimal);
b.identity.name = "Renamed";
const changed = JSON.parse(diff(JSON.stringify(a), JSON.stringify(b)));
check("name change: no error field", changed.error === undefined);
check(
  "name change: exactly one diff",
  Array.isArray(changed.diffs) && changed.diffs.length === 1,
);
check(
  "name change: path is build.identity.name",
  changed.diffs[0].path === "build.identity.name",
);
check("name change: change is modified", changed.diffs[0].change === "modified");
check(
  "name change: before = Test Character",
  changed.diffs[0].before === "Test Character",
);
check("name change: after = Renamed", changed.diffs[0].after === "Renamed");

// 3. Malformed input → error object, no panic.
const bad = JSON.parse(diff("not json", "{}"));
check("bad input: returns error string", typeof bad.error === "string");

if (failures === 0) {
  console.log("\nAll JS spike checks passed.");
  process.exit(0);
} else {
  console.error(`\n${failures} check(s) FAILED.`);
  process.exit(1);
}
