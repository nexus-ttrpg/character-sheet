// HTTP endpoint handlers for the relay module (§7.1, §13.3).
//
// These are the relay-side counterparts to the server's pull/push/drift
// endpoints. The server (or CLI) calls these instead of running the foundry
// adapter in the adapter-host — the relay does the conversion in-browser using
// the same foundry-mapping code (§7.1's key win: "it's an import statement").
//
// Each handler is a pure function that takes the Foundry API (abstracted) and
// a relay request, and returns a relay response. The auth check is done by a
// wrapper, not inside each handler, so handlers focus on logic.
//
// §7.2: export writes max HP only — current HP is session state, never written.
// §4:   drift hooks never auto-sync — they just maintain telemetry.
// §7.7: homebrew is never an error — unknown items are warnings.

import { exportActor, importActor, type FoundryActor, type FoundrySystemId } from "@nexus-ttrpg/foundry-mapping";
import type { CharacterBuild } from "@nexus-ttrpg/schema-types";
import type { FoundryGame, RelayRequest, RelayResponse, RelayHttpServer } from "./types.js";
import { validateAuthHeader } from "./settings.js";
import type { DriftTracker } from "./drift.js";

// ── Auth wrapper ─────────────────────────────────────────────────────────────

/**
 * Wrap a handler with API key authentication.
 *
 * Returns 401 if the Authorization header is missing or doesn't match the
 * relay's API key. The key is validated in constant time (settings.ts).
 */
export function withAuth(
  apiKey: string,
  handler: (req: RelayRequest) => Promise<RelayResponse>,
): (req: RelayRequest) => Promise<RelayResponse> {
  return async (req: RelayRequest): Promise<RelayResponse> => {
    if (!validateAuthHeader(req.authHeader, apiKey)) {
      return { status: 401, body: { error: "unauthorized" } };
    }
    return handler(req);
  };
}

// ── Endpoint handlers ────────────────────────────────────────────────────────

/**
 * `GET /nexus/actors` — list all character actors in the world.
 *
 * Returns actor metadata (id, name, uuid) — not the full canonical build.
 * The server uses this to discover which actors are available for sync.
 */
export async function listActors(game: FoundryGame): Promise<RelayResponse> {
  const actors = game.actors.contents.filter((a) => a.type === "character");
  return {
    status: 200,
    body: {
      actors: actors.map((a) => ({ id: a.id, name: a.name, uuid: a.uuid })),
      system: game.system.id,
    },
  };
}

/**
 * `GET /nexus/actors/:id` — get a single actor as a canonical build (import).
 *
 * This is the relay-side equivalent of the adapter's `import()`: it reads the
 * live Actor document, converts it to a FoundryActor via `toObject()`, then
 * runs `importActor()` from foundry-mapping (§7.1 — "imports foundry-mapping
 * directly to convert Actors to canonical in-browser").
 */
export async function getActor(game: FoundryGame, req: RelayRequest): Promise<RelayResponse> {
  const actorId = req.params.id;
  if (!actorId) {
    return { status: 400, body: { error: "missing 'id' path parameter" } };
  }
  const actor = game.actors.get(actorId);
  if (!actor) {
    return { status: 404, body: { error: `actor not found: ${actorId}` } };
  }
  if (actor.type !== "character") {
    return { status: 400, body: { error: `actor is not a character: ${actor.type}` } };
  }

  const foundryActor = actor.toObject() as FoundryActor;
  const build = importActor(foundryActor);

  return {
    status: 200,
    body: {
      actor_id: actor.id,
      actor_uuid: actor.uuid,
      build,
    },
  };
}

/**
 * `POST /nexus/actors/:id/apply` — apply a canonical build to an actor (export).
 *
 * This is the relay-side equivalent of the adapter's `export()`: it calls
 * `exportActor()` from foundry-mapping to produce a FoundryActor, then writes
 * it to the live Actor document via `actor.update()`.
 *
 * §7.2: max HP only — the export from foundry-mapping only includes max HP,
 * never current/temp HP. §4: session state is never written.
 */
export async function applyActor(game: FoundryGame, req: RelayRequest): Promise<RelayResponse> {
  const actorId = req.params.id;
  if (!actorId) {
    return { status: 400, body: { error: "missing 'id' path parameter" } };
  }
  const actor = game.actors.get(actorId);
  if (!actor) {
    return { status: 404, body: { error: `actor not found: ${actorId}` } };
  }

  const body = req.body as { build?: CharacterBuild } | undefined;
  if (!body?.build) {
    return { status: 400, body: { error: "missing 'build' in request body" } };
  }

  const systemId = game.system.id as FoundrySystemId;
  const foundryActor = exportActor(body.build, systemId);

  // Write the exported actor to the live document. The update call carries
  // system data + items; Foundry's DataModel validates it (§7.6).
  await actor.update({
    name: foundryActor.name,
    system: foundryActor.system,
    items: foundryActor.items,
  });

  return {
    status: 200,
    body: {
      actor_id: actor.id,
      applied: true,
    },
  };
}

/**
 * `GET /nexus/drift` — report which actors have changed since their last sync.
 *
 * Returns the drift tracker's report: dirty actors (changed, not yet synced)
 * and clean actors (synced or unchanged). The server uses this to decide which
 * actors to pull without polling each one individually (§5.3, §7.1).
 */
export async function getDrift(tracker: DriftTracker): Promise<RelayResponse> {
  return {
    status: 200,
    body: tracker.report(),
  };
}

// ── Route registration ───────────────────────────────────────────────────────

/**
 * Register all relay HTTP routes on the given server.
 *
 * Called from relay.ts during module init. Each route is wrapped with
 * `withAuth()` to enforce the API key check.
 */
export function registerRoutes(
  server: RelayHttpServer,
  game: FoundryGame,
  tracker: DriftTracker,
  apiKey: string,
): void {
  server.registerRoute("GET", "/nexus/actors", withAuth(apiKey, () => listActors(game)));

  server.registerRoute("GET", "/nexus/actors/:id", withAuth(apiKey, (req) => getActor(game, req)));

  server.registerRoute("POST", "/nexus/actors/:id/apply", withAuth(apiKey, (req) => applyActor(game, req)));

  server.registerRoute("GET", "/nexus/drift", withAuth(apiKey, () => getDrift(tracker)));
}
