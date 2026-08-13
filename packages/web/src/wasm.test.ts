// WASM smoke test — verifies the browser-target WASM engine produces the same
// diff/merge/apply/hash results as the native Rust tests, using a real fixture.
// Skips gracefully if the WASM hasn't been built yet (run `pnpm build:wasm`).
//
// Uses `initSync` with a readFileSync'd .wasm buffer (no browser needed),
// exercising the same `sync-engine-wasm` crate that the browser loads via
// `__wbg_init()`.

import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const wasmJsPath = join(here, "wasm", "sync_engine_wasm.js");
const wasmBgPath = join(here, "wasm", "sync_engine_wasm_bg.wasm");
const fixturePath = join(here, "..", "..", "..", "examples", "fixtures", "pf2e-level1-fighter.json");

const wasmAvailable = existsSync(wasmJsPath) && existsSync(wasmBgPath);

// Minimal WASM module type — the actual module is dynamically imported.
interface WasmModule {
  initSync(module: WebAssembly.Module): void;
  diff(a: string, b: string): string;
  merge(ancestor: string, ours: string, theirs: string): string;
  apply_merge(ours: string, preview: string, resolutions: string): string;
  content_hash(build: string): string;
}

describe.skipIf(!wasmAvailable)("WASM engine (sync-engine-wasm)", () => {
  let mod: WasmModule;

  // Shared fixture data — loaded once, cloned by value for mutation.
  const fighter = JSON.parse(readFileSync(fixturePath, "utf-8")) as Record<string, unknown>;
  const ours = structuredClone(fighter);
  (ours.identity as { name: string }).name = "Brodric the Brave";
  const theirs = structuredClone(fighter);
  (theirs.identity as { name: string }).name = "Alistair the Bold";

  it("loads and inits", async () => {
    mod = (await import("./wasm/sync_engine_wasm.js")) as unknown as WasmModule;
    mod.initSync(readFileSync(wasmBgPath));
  });

  it("diff detects field changes", () => {
    const result = JSON.parse(mod.diff(JSON.stringify(fighter), JSON.stringify(ours)));
    expect(result.error).toBeUndefined();
    expect(result.diffs).toBeDefined();
    const paths = (result.diffs as { path: string }[]).map((d) => d.path);
    expect(paths).toContain("build.identity.name");
  });

  it("merge detects conflicts on divergent name edits", () => {
    const result = JSON.parse(
      mod.merge(
        JSON.stringify(fighter),
        JSON.stringify(ours),
        JSON.stringify(theirs),
      ),
    );
    expect(result.error).toBeUndefined();
    expect(result.mergePreview).toBeDefined();
    const conflicts = result.mergePreview.conflicts as { path: string }[];
    expect(conflicts.length).toBe(1);
    expect(conflicts[0]!.path).toBe("build.identity.name");
  });

  it("apply_merge with 'ours' resolution keeps our name", () => {
    const mergeResult = JSON.parse(
      mod.merge(
        JSON.stringify(fighter),
        JSON.stringify(ours),
        JSON.stringify(theirs),
      ),
    );
    const preview = mergeResult.mergePreview;
    const resolutions = [{ path: "build.identity.name", resolution: "ours" }];
    const applyResult = JSON.parse(
      mod.apply_merge(
        JSON.stringify(ours),
        JSON.stringify(preview),
        JSON.stringify(resolutions),
      ),
    );
    expect(applyResult.error).toBeUndefined();
    expect(applyResult.merged).toBeDefined();
    expect((applyResult.merged.identity as { name: string }).name).toBe("Brodric the Brave");
  });

  it("apply_merge with 'theirs' resolution takes their name", () => {
    const mergeResult = JSON.parse(
      mod.merge(
        JSON.stringify(fighter),
        JSON.stringify(ours),
        JSON.stringify(theirs),
      ),
    );
    const preview = mergeResult.mergePreview;
    const resolutions = [{ path: "build.identity.name", resolution: "theirs" }];
    const applyResult = JSON.parse(
      mod.apply_merge(
        JSON.stringify(ours),
        JSON.stringify(preview),
        JSON.stringify(resolutions),
      ),
    );
    expect(applyResult.error).toBeUndefined();
    expect((applyResult.merged.identity as { name: string }).name).toBe("Alistair the Bold");
  });

  it("content_hash is deterministic and differs across builds", () => {
    const h1 = JSON.parse(mod.content_hash(JSON.stringify(ours)));
    const h2 = JSON.parse(mod.content_hash(JSON.stringify(ours)));
    expect(h1.contentHash).toBeDefined();
    expect(h1.contentHash).toBe(h2.contentHash);
    const h3 = JSON.parse(mod.content_hash(JSON.stringify(theirs)));
    expect(h3.contentHash).not.toBe(h1.contentHash);
  });
});

describe.skipIf(wasmAvailable)("WASM engine (not built)", () => {
  it("skips — run `pnpm build:wasm` to enable", () => {
    // Placeholder so the test file isn't empty when WASM is absent.
  });
});
