// Bundle entry point for the Foundry relay module (§7.1).
//
// Foundry loads this file as an ES module (declared in module.json's
// `esmodules`). It wires the relay module's two init phases to Foundry's
// `init` and `ready` hooks.
//
// The bundle includes foundry-mapping + schema-types (type-only, erased) +
// the relay glue code — a single self-contained JS file with no npm imports,
// ready to drop into a Foundry modules directory.
//
// Build: pnpm --filter @nexus-ttrpg/foundry-adapter bundle:relay
// Output: module/nexus-character-sheet.js

import { initPhase, readyPhase } from "./src/relay.js";
import type { FoundryHooks } from "./src/types.js";

// Foundry exposes `game`, `Hooks`, and a route registration API as globals.
// The relay module's types.ts declares minimal interfaces for these; here we
// cast the globals to those interfaces.
//
// In a real deployment, the HTTP server abstraction (`RelayHttpServer`) would
// be wired to Foundry's Express integration or a companion library. This
// bundle entry point is where that wiring happens.

declare const game: unknown;
declare const Hooks: FoundryHooks;

// Minimal HTTP server shim — a real deployment wires this to Foundry's route
// registration. The interface matches `RelayHttpServer` from types.ts.
const relayServer = {
  routes: [] as Array<{ method: string; path: string }>,
  registerRoute(method: "GET" | "POST", path: string, handler: unknown) {
    this.routes.push({ method, path });
    // In a real deployment: Foundry's Express integration or fvtt-router.
    // For now, this shim records the route for diagnostics.
  },
};

Hooks.on("init", () => {
  initPhase(game as never);
});

Hooks.on("ready", async () => {
  await readyPhase(game as never, Hooks, relayServer as never);
});
