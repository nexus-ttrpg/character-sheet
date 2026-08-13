// Main relay module entry point (§7.1).
//
// This is the code Foundry loads when the module is activated. It:
//   1. Registers the API key setting (settings.ts).
//   2. Creates a DriftTracker and hooks Actor create/update (drift.ts).
//   3. Registers HTTP endpoints (endpoints.ts).
//
// The module runs in the GM's browser. It imports foundry-mapping by plain
// import — the clearest concrete win from the Rust/TS split (§7.1):
// "no WASM toolchain, no dual-target build, no parity test suite."
//
// §4:  drift hooks never auto-sync — they just maintain telemetry.
// §7.2: export writes max HP only — current HP is session state.
// §7.7: homebrew is never an error — unknown items are warnings.

import type { FoundryGame, FoundryHooks, RelayHttpServer, LiveActor } from "./types.js";
import { registerApiKeySetting, getOrGenerateApiKey } from "./settings.js";
import { DriftTracker, actorContentHash } from "./drift.js";
import { registerRoutes } from "./endpoints.js";

const MODULE_ID = "nexus-sheet-relay";

/**
 * Initialize the relay module.
 *
 * Called on Foundry's `init` hook (to register settings early) and `ready`
 * hook (to register routes + hooks, when the game data is loaded).
 *
 * This function is split into two phases because Foundry requires settings to
 * be registered before the world data loads, but routes/hooks need the loaded
 * game. The bundle entry point (relay-bundle.ts) wires both phases to the
 * appropriate Foundry hooks.
 */

/**
 * Phase 1: register settings (called on `init`).
 */
export function initPhase(game: FoundryGame): void {
  registerApiKeySetting(game.settings);
}

/**
 * Phase 2: register routes + hooks (called on `ready`).
 *
 * @returns the DriftTracker instance (for testing).
 */
export async function readyPhase(
  game: FoundryGame,
  hooks: FoundryHooks,
  server: RelayHttpServer,
): Promise<DriftTracker> {
  // Get or generate the relay API key.
  const apiKey = await getOrGenerateApiKey(game.settings);

  // Create the drift tracker.
  const tracker = new DriftTracker();

  // Hook Actor create/update for drift telemetry (§7.1 point 2).
  // Never auto-syncs (§4) — just marks actors as dirty.
  hooks.on("createActor", (actor: unknown) => {
    trackActorChange(tracker, actor as LiveActor);
  });
  hooks.on("updateActor", (actor: unknown) => {
    trackActorChange(tracker, actor as LiveActor);
  });

  // Register HTTP endpoints (§7.1 point 1).
  registerRoutes(server, game, tracker, apiKey);

  return tracker;
}

/**
 * Track an actor change in the drift tracker.
 *
 * Extracted as a standalone function so it can be tested without Foundry's
 * Hooks system. Computes a content hash of the actor's serialized form for
 * cheap drift detection.
 */
function trackActorChange(tracker: DriftTracker, actor: LiveActor): void {
  if (actor.type !== "character") return;

  const actorJson = JSON.stringify(actor.toObject());
  const hash = actorContentHash(actorJson);

  tracker.markChanged(actor.id, actor.uuid, actor.name, hash);
}

export { MODULE_ID };
