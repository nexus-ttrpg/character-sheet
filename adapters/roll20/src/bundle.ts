// Bundle entry point for the Roll20 adapter (§6, §5.5).
//
// esbuild bundles this into a single self-contained ESM file with no npm
// imports; deno_core loads it as an ES module. The host appends
// `globalThis.__nexusAdapter = adapter;` to the module source after load (§5.5).

import { roll20Adapter } from "./index.js";

export const adapter = roll20Adapter;
