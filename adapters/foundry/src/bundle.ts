// Bundle entry point for the Foundry adapter (§7, §5.5).
//
// This file is the entry point for esbuild bundling. It imports the adapter
// and assigns it to a top-level `const adapter` so that the adapter-host's
// `load()` method can access it after appending
// `globalThis.__nexusAdapter = adapter;` to the module source (§5.5, host.rs).
//
// The bundled output is a single self-contained JS file with no npm imports
// — `deno_core` loads it as an ES module without a module resolver.
//
// The `export` prevents esbuild from tree-shaking the binding; deno_core
// evaluates the module body, making `adapter` available to the host's
// appended `globalThis.__nexusAdapter = adapter;` line.

import { foundryAdapter } from "./index.js";

export const adapter = foundryAdapter;
