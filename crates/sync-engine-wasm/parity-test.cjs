// Parity test: confirms the WASM module produces byte-identical output to
// native sync-engine on all fixture pairs and merge/apply scenarios.
//
// Prerequisites:
//   cargo test -p sync-engine-wasm --test generate_parity   # generates parity-expected.json
//   wasm-pack build crates/sync-engine-wasm --target nodejs --release
//
// Run with: node crates/sync-engine-wasm/parity-test.cjs
const {
  diff,
  merge,
  apply_merge,
  content_hash,
} = require("./pkg/sync_engine_wasm.js");
const fs = require("fs");
const path = require("path");

const expected = JSON.parse(
  fs.readFileSync(path.join(__dirname, "parity-expected.json"), "utf8"),
);

let failures = 0;
let checks = 0;

function check(name, cond) {
  checks++;
  if (cond) {
    console.log("  ok   -", name);
  } else {
    console.log("  FAIL -", name);
    failures++;
  }
}

// Deep equality that handles key order differences in objects.
function deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null) return a === b;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }
  if (typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (ka.length !== kb.length || !ka.every((k, i) => k === kb[i])) return false;
    return ka.every((k) => deepEqual(a[k], b[k]));
  }
  return false;
}

// ── Diff pairs (16) ──────────────────────────────────────────────────────
console.log("Diff pairs:");
for (const tc of expected.diffs) {
  const wasmOut = JSON.parse(diff(JSON.stringify(tc.a), JSON.stringify(tc.b)));
  check(
    `diff ${tc.name}`,
    !wasmOut.error && deepEqual(wasmOut, tc.expected),
  );
}

// ── Merge scenarios (3) ──────────────────────────────────────────────────
console.log("Merge scenarios:");
for (const tc of expected.merges) {
  const wasmOut = JSON.parse(
    merge(
      JSON.stringify(tc.ancestor),
      JSON.stringify(tc.ours),
      JSON.stringify(tc.theirs),
    ),
  );
  check(
    `merge ${tc.name}`,
    !wasmOut.error && deepEqual(wasmOut, tc.expected),
  );
}

// ── Apply_merge scenarios (4) ────────────────────────────────────────────
console.log("Apply_merge scenarios:");
for (const tc of expected.applyMerges) {
  const wasmOut = JSON.parse(
    apply_merge(
      JSON.stringify(tc.ours),
      JSON.stringify(tc.preview),
      JSON.stringify(tc.resolutions),
    ),
  );
  check(
    `apply_merge ${tc.name}`,
    !wasmOut.error && deepEqual(wasmOut, tc.expected),
  );
}

// ── Content hashes (4) ───────────────────────────────────────────────────
console.log("Content hashes:");
for (const tc of expected.contentHashes) {
  const wasmOut = JSON.parse(content_hash(JSON.stringify(tc.build)));
  check(
    `content_hash ${tc.name}`,
    !wasmOut.error && deepEqual(wasmOut, tc.expected),
  );
}

// ── Error handling ───────────────────────────────────────────────────────
console.log("Error handling:");
const badDiff = JSON.parse(diff("not json", "{}"));
check("diff bad input returns error", typeof badDiff.error === "string");

const badMerge = JSON.parse(merge("not json", "{}", "{}"));
check("merge bad input returns error", typeof badMerge.error === "string");

const badHash = JSON.parse(content_hash("not json"));
check("content_hash bad input returns error", typeof badHash.error === "string");

const badApply = JSON.parse(
  apply_merge("not json", '{"autoMerged":[],"conflicts":[]}', "[]"),
);
check("apply_merge bad input returns error", typeof badApply.error === "string");

// ── Summary ──────────────────────────────────────────────────────────────
console.log(
  `\n${checks - failures}/${checks} checks passed` +
    (failures === 0 ? "." : `, ${failures} FAILED.`),
);
process.exit(failures === 0 ? 0 : 1);
