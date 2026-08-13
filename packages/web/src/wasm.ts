// Loads the browser-target WASM engine (sync-engine-wasm, `--target web`) and
// wraps its JSON-string functions in typed helpers that parse the results.
//
// The WASM pkg is served as a *separate* ES module at `/wasm/sync_engine_wasm.js`
// (copied to dist/wasm by build.mjs), NOT bundled into app.js. We load it with a
// runtime dynamic `import()` of a computed URL — esbuild cannot resolve a
// non-literal specifier, so it leaves the import for the browser to fetch. This
// keeps `tsc`/esbuild working even before `pnpm build:wasm` has run; the engine
// simply reports unavailable until the .wasm is present on the server.

import type {
  WasmApplyResult,
  WasmDiffResult,
  WasmHashResult,
  WasmMergeResult,
  JsonValue,
  MergePreview,
} from "./types";

interface WasmRawModule {
  default(): Promise<unknown>;
  diff(a: string, b: string): string;
  merge(ancestor: string, ours: string, theirs: string): string;
  apply_merge(ours: string, preview: string, resolutions: string): string;
  content_hash(build: string): string;
}

export interface Resolution {
  path: string;
  resolution: "ours" | "theirs" | JsonValue;
}

export interface WasmEngine {
  diffBuilds(a: JsonValue, b: JsonValue): WasmDiffResult;
  mergeBuilds(
    ancestor: JsonValue,
    ours: JsonValue,
    theirs: JsonValue,
  ): WasmMergeResult;
  applyMerge(
    ours: JsonValue,
    preview: MergePreview,
    resolutions: Resolution[],
  ): WasmApplyResult;
  contentHash(build: JsonValue): WasmHashResult;
}

let enginePromise: Promise<WasmEngine | null> | null = null;

export function loadWasm(): Promise<WasmEngine | null> {
  if (enginePromise) return enginePromise;
  enginePromise = (async () => {
    try {
      // Runtime URL — esbuild cannot statically resolve this, so it stays a
      // browser-side dynamic import. `document.baseURI` is the page origin,
      // so this resolves to `<origin>/wasm/sync_engine_wasm.js` in both dev
      // (esbuild serve) and prod (ServeDir on the API).
      const url = new URL("wasm/sync_engine_wasm.js", document.baseURI).href;
      const mod = (await import(/* @vite-ignore */ url)) as unknown as WasmRawModule;
      await mod.default(); // instantiate the wasm (auto-locates the .wasm)
      return wrap(mod);
    } catch (err) {
      console.warn("WASM engine failed to load:", err);
      return null;
    }
  })();
  return enginePromise;
}

function wrap(mod: WasmRawModule): WasmEngine {
  return {
    diffBuilds(a, b) {
      return JSON.parse(mod.diff(JSON.stringify(a), JSON.stringify(b))) as WasmDiffResult;
    },
    mergeBuilds(ancestor, ours, theirs) {
      return JSON.parse(
        mod.merge(JSON.stringify(ancestor), JSON.stringify(ours), JSON.stringify(theirs)),
      ) as WasmMergeResult;
    },
    applyMerge(ours, preview, resolutions) {
      const res = resolutions.map((r) => ({ path: r.path, resolution: r.resolution }));
      return JSON.parse(
        mod.apply_merge(
          JSON.stringify(ours),
          JSON.stringify(preview),
          JSON.stringify(res),
        ),
      ) as WasmApplyResult;
    },
    contentHash(build) {
      return JSON.parse(mod.content_hash(JSON.stringify(build))) as WasmHashResult;
    },
  };
}
