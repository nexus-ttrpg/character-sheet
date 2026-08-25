// Full-loop integration test: canonical build → relay apply (export) →
// Foundry actor → relay get (import) → canonical build, over real HTTP (§7.1,
// §13.3, §16).
//
// This is the end-to-end data-fidelity test for the relay path. It proves that
// a canonical build produced by the Rust API can be pushed to the relay over
// HTTP, written to a (mock) Foundry actor, read back, and converted back to a
// canonical build — all without loss of the fields that matter.
//
// The test uses the same JSON fixtures the Rust e2e tests use (examples/
// fixtures/), so the canonical builds here are exactly what the API would
// produce. The relay's foundry-mapping import/export is the same code the
// server-side adapter uses (§7.1 — "it's an import statement").
//
// Flow:
//   1. Load a fixture (canonical build) — this is what the API would have.
//   2. Start NodeHttpServer with a mock game containing one actor.
//   3. POST /nexus/actors/:id/apply — push the build to the relay.
//   4. GET /nexus/actors/:id — pull the build back from the relay.
//   5. Compare the pulled build's key fields against the original.
//   6. Also test the reverse: GET first (actor exists in Foundry), then apply
//      a modified build, then GET again to verify the change took effect.

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import { exportActor, importActor, type FoundryActor, type FoundrySystemId } from "@nexus-sheet/foundry-mapping";

import { NodeHttpServer } from "./node-http-server.js";
import { registerRoutes } from "./endpoints.js";
import { DriftTracker } from "./drift.js";
import type {
  FoundryGame,
  FoundrySettings,
  LiveActor,
  ActorCollection,
  SettingOptions,
} from "./types.js";

// ── Fixtures ─────────────────────────────────────────────────────────────────

function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}

const fighterBuild = loadFixture("pf2e-level1-fighter.json");
const mechanicBuild = loadFixture("sf2e-level5-mechanic.json");

// ── Mock Foundry API ─────────────────────────────────────────────────────────

class MockSettings implements FoundrySettings {
  private store = new Map<string, unknown>();
  registered = new Map<string, SettingOptions>();
  register(_m: string, key: string, opts: SettingOptions): void {
    this.registered.set(key, opts);
    this.store.set(key, opts.default ?? "");
  }
  get(_m: string, key: string): unknown { return this.store.get(key); }
  async set(_m: string, key: string, val: unknown): Promise<void> { this.store.set(key, val); }
}

function makeLiveActor(foundryActor: FoundryActor, id: string): LiveActor {
  let data = foundryActor;
  return {
    id,
    name: foundryActor.name,
    type: foundryActor.type,
    uuid: `Actor.${id}`,
    toObject: () => data,
    async update(updateData: Record<string, unknown>): Promise<LiveActor> {
      data = { ...data, ...updateData } as FoundryActor;
      return this;
    },
  };
}

function makeGame(actors: LiveActor[], systemId: FoundrySystemId): FoundryGame {
  const collection: ActorCollection = {
    get: (id: string) => actors.find((a) => a.id === id),
    contents: actors,
  };
  return { actors: collection, settings: new MockSettings(), system: { id: systemId } };
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────

const API_KEY = "nsk_relay_fullloop_key";

async function relayGet(port: number, path: string): Promise<{ status: number; body: unknown }> {
  const resp = await fetch(`http://127.0.0.1:${port}${path}`, {
    headers: { authorization: `Bearer ${API_KEY}` },
  });
  return { status: resp.status, body: await resp.json() };
}

async function relayPost(port: number, path: string, body: unknown): Promise<{ status: number; body: unknown }> {
  const resp = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${API_KEY}`,
    },
    body: JSON.stringify(body),
  });
  return { status: resp.status, body: await resp.json() };
}

// ── Test suite ───────────────────────────────────────────────────────────────

describe("full-loop — canonical build → relay → canonical build (§7.1)", () => {
  // We test both PF2e and SF2e to verify the relay handles both systems.

  describe("PF2e fighter round-trip", () => {
    let server: NodeHttpServer;
    let port: number;
    let actor: LiveActor;

    beforeAll(async () => {
      const foundryActor = exportActor(fighterBuild, "pf2e");
      actor = makeLiveActor(foundryActor, "fighter1");
      const game = makeGame([actor], "pf2e");
      server = new NodeHttpServer();
      registerRoutes(server, game, new DriftTracker(), API_KEY);
      port = await server.listen(0);
    });

    afterAll(async () => {
      await server.close();
    });

    it("GET /nexus/actors/:id returns a build matching importActor()", async () => {
      const { status, body } = await relayGet(port, "/nexus/actors/fighter1");
      expect(status).toBe(200);

      const returnedBuild = (body as { build: CharacterBuild }).build;
      const expectedBuild = importActor(exportActor(fighterBuild, "pf2e"));

      expect(returnedBuild).toEqual(expectedBuild);
    });

    it("POST apply + GET round-trips the build identity", async () => {
      // Apply the original build.
      const { status: applyStatus } = await relayPost(port, "/nexus/actors/fighter1/apply", {
        build: fighterBuild,
      });
      expect(applyStatus).toBe(200);

      // Read it back.
      const { status: getStatus, body } = await relayGet(port, "/nexus/actors/fighter1");
      expect(getStatus).toBe(200);

      const pulledBuild = (body as { build: CharacterBuild }).build;

      // Key fields should survive the round-trip.
      expect(pulledBuild.identity.name).toBe(fighterBuild.identity.name);
      expect(pulledBuild.identity.level).toBe(fighterBuild.identity.level);
      expect(pulledBuild.ancestry?.slug).toBe(fighterBuild.ancestry?.slug);
      expect(pulledBuild.heritage?.slug).toBe(fighterBuild.heritage?.slug);
      expect(pulledBuild.class?.slug).toBe(fighterBuild.class?.slug);
    });

    it("POST apply with modified name, then GET returns the modified name", async () => {
      const modified = structuredClone(fighterBuild);
      modified.identity.name = "Sir Fight-a-lot";

      const { status } = await relayPost(port, "/nexus/actors/fighter1/apply", {
        build: modified,
      });
      expect(status).toBe(200);

      const { body } = await relayGet(port, "/nexus/actors/fighter1");
      const pulled = (body as { build: CharacterBuild }).build;
      expect(pulled.identity.name).toBe("Sir Fight-a-lot");
    });
  });

  describe("SF2e mechanic round-trip", () => {
    let server: NodeHttpServer;
    let port: number;

    beforeAll(async () => {
      const foundryActor = exportActor(mechanicBuild, "sf2e");
      const actor = makeLiveActor(foundryActor, "mech1");
      const game = makeGame([actor], "sf2e");
      server = new NodeHttpServer();
      registerRoutes(server, game, new DriftTracker(), API_KEY);
      port = await server.listen(0);
    });

    afterAll(async () => {
      await server.close();
    });

    it("GET /nexus/actors/:id returns a build matching importActor()", async () => {
      const { status, body } = await relayGet(port, "/nexus/actors/mech1");
      expect(status).toBe(200);

      const returnedBuild = (body as { build: CharacterBuild }).build;
      const expectedBuild = importActor(exportActor(mechanicBuild, "sf2e"));

      expect(returnedBuild).toEqual(expectedBuild);
    });

    it("POST apply + GET round-trips SF2e-specific fields (drones, credits)", async () => {
      const { status: applyStatus } = await relayPost(port, "/nexus/actors/mech1/apply", {
        build: mechanicBuild,
      });
      expect(applyStatus).toBe(200);

      const { status: getStatus, body } = await relayGet(port, "/nexus/actors/mech1");
      expect(getStatus).toBe(200);

      const pulled = (body as { build: CharacterBuild }).build;

      // SF2e fields should survive the round-trip.
      expect(pulled.drones?.length).toBe(mechanicBuild.drones?.length);
      expect(pulled.currencies.credits).toBe(mechanicBuild.currencies.credits);
      expect(pulled.vehicles?.length).toBe(mechanicBuild.vehicles?.length);
    });

    it("POST apply with modified credits, then GET returns modified credits", async () => {
      const modified = structuredClone(mechanicBuild);
      modified.currencies.credits = 99999;

      const { status } = await relayPost(port, "/nexus/actors/mech1/apply", {
        build: modified,
      });
      expect(status).toBe(200);

      const { body } = await relayGet(port, "/nexus/actors/mech1");
      const pulled = (body as { build: CharacterBuild }).build;
      expect(pulled.currencies.credits).toBe(99999);
    });
  });

  describe("drift tracking during full-loop", () => {
    it("GET /nexus/drift reflects actor state after apply", async () => {
      const foundryActor = exportActor(fighterBuild, "pf2e");
      const actor = makeLiveActor(foundryActor, "d1");
      const game = makeGame([actor], "pf2e");
      const tracker = new DriftTracker();
      const server = new NodeHttpServer();
      registerRoutes(server, game, tracker, API_KEY);
      const port = await server.listen(0);

      try {
        // Initially no drift.
        const { body: before } = await relayGet(port, "/nexus/drift");
        expect((before as { total: number }).total).toBe(0);

        // Mark the actor as changed (simulating a Foundry update event).
        tracker.markChanged("d1", "Actor.d1", "Fighter", "hash1");

        // Drift report should now show the dirty actor.
        const { body: after } = await relayGet(port, "/nexus/drift");
        const report = after as { total: number; dirty: Array<{ actorUuid: string }> };
        expect(report.total).toBe(1);
        expect(report.dirty[0]!.actorUuid).toBe("Actor.d1");

        // Apply a build (simulating a sync) — this doesn't auto-mark synced,
        // but we can manually mark it and verify.
        tracker.markSynced("d1");
        const { body: synced } = await relayGet(port, "/nexus/drift");
        expect((synced as { total: number; dirty: unknown[] }).dirty).toHaveLength(0);
      } finally {
        await server.close();
      }
    });
  });
});
