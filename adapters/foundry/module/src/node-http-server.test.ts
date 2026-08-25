// Integration tests for NodeHttpServer — real HTTP requests against the relay
// module's endpoint handlers (§7.1, §13.3, §16).
//
// Unlike relay.test.ts (which uses MockHttpServer — in-memory, no network),
// these tests start a real Node HTTP server on an ephemeral port and make
// actual HTTP requests using fetch(). This verifies:
//   - Route pattern matching (`:id` param extraction)
//   - JSON body parsing for POST routes
//   - Authorization header propagation
//   - HTTP status codes and response bodies
//   - 404 for unknown routes
//   - The full relay HTTP contract works over real HTTP, not just in-memory

import { describe, it, expect, beforeAll, afterAll, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import { exportActor, type FoundryActor } from "@nexus-sheet/foundry-mapping";

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

// ── Test fixtures ────────────────────────────────────────────────────────────

function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}

const fighterBuild = loadFixture("pf2e-level1-fighter.json");
const fighterActor = exportActor(fighterBuild, "pf2e");

// ── Mock Foundry API ─────────────────────────────────────────────────────────

class MockSettings implements FoundrySettings {
  private store = new Map<string, unknown>();
  registered = new Map<string, SettingOptions>();

  register(_moduleId: string, key: string, options: SettingOptions): void {
    this.registered.set(key, options);
    this.store.set(key, options.default ?? "");
  }

  get(_moduleId: string, key: string): unknown {
    return this.store.get(key);
  }

  async set(_moduleId: string, key: string, value: unknown): Promise<void> {
    this.store.set(key, value);
  }
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

function makeGame(actors: LiveActor[], settings: FoundrySettings, systemId = "pf2e"): FoundryGame {
  const collection: ActorCollection = {
    get: (id: string) => actors.find((a) => a.id === id),
    contents: actors,
  };
  return {
    actors: collection,
    settings,
    system: { id: systemId },
  };
}

// ── HTTP client helpers ──────────────────────────────────────────────────────

const API_KEY = "nsk_relay_test_integration_key";

async function httpGet(port: number, path: string, auth: boolean = true): Promise<{ status: number; body: unknown }> {
  const headers: Record<string, string> = {};
  if (auth) headers["authorization"] = `Bearer ${API_KEY}`;
  const resp = await fetch(`http://127.0.0.1:${port}${path}`, { headers });
  const body = await resp.json();
  return { status: resp.status, body };
}

async function httpPost(port: number, path: string, body: unknown, auth: boolean = true): Promise<{ status: number; body: unknown }> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (auth) headers["authorization"] = `Bearer ${API_KEY}`;
  const resp = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  const json = await resp.json();
  return { status: resp.status, body: json };
}

// ── Test suite ───────────────────────────────────────────────────────────────

describe("NodeHttpServer — HTTP integration", () => {
  let server: NodeHttpServer;
  let port: number;
  let game: FoundryGame;
  let tracker: DriftTracker;
  let actor: LiveActor;

  beforeAll(async () => {
    server = new NodeHttpServer();
    actor = makeLiveActor(fighterActor, "a1");
    game = makeGame([actor], new MockSettings());
    tracker = new DriftTracker();

    registerRoutes(server, game, tracker, API_KEY);
    port = await server.listen(0);
  });

  afterAll(async () => {
    await server.close();
  });

  // ── Auth ───────────────────────────────────────────────────────────────────

  describe("authentication", () => {
    it("rejects requests without Authorization header (401)", async () => {
      const { status, body } = await httpGet(port, "/nexus/actors", false);
      expect(status).toBe(401);
      expect((body as { error: string }).error).toBe("unauthorized");
    });

    it("rejects requests with wrong API key (401)", async () => {
      const resp = await fetch(`http://127.0.0.1:${port}/nexus/actors`, {
        headers: { authorization: "Bearer wrong_key" },
      });
      expect(resp.status).toBe(401);
    });

    it("accepts requests with correct Bearer token", async () => {
      const { status } = await httpGet(port, "/nexus/actors", true);
      expect(status).toBe(200);
    });
  });

  // ── GET /nexus/actors ──────────────────────────────────────────────────────

  describe("GET /nexus/actors", () => {
    it("lists character actors with id, name, uuid", async () => {
      const { status, body } = await httpGet(port, "/nexus/actors");
      expect(status).toBe(200);
      const b = body as { actors: Array<{ id: string; name: string; uuid: string }>; system: string };
      expect(b.actors).toHaveLength(1);
      expect(b.actors[0]!.id).toBe("a1");
      expect(b.actors[0]!.name).toBe(fighterActor.name);
      expect(b.actors[0]!.uuid).toBe("Actor.a1");
      expect(b.system).toBe("pf2e");
    });
  });

  // ── GET /nexus/actors/:id ──────────────────────────────────────────────────

  describe("GET /nexus/actors/:id", () => {
    it("returns canonical build for a character actor", async () => {
      const { status, body } = await httpGet(port, "/nexus/actors/a1");
      expect(status).toBe(200);
      const b = body as { actor_id: string; actor_uuid: string; build: { identity: { name: string } } };
      expect(b.actor_id).toBe("a1");
      expect(b.actor_uuid).toBe("Actor.a1");
      expect(b.build.identity.name).toBe(fighterBuild.identity.name);
    });

    it("returns 404 for a nonexistent actor", async () => {
      const { status } = await httpGet(port, "/nexus/actors/nonexistent");
      expect(status).toBe(404);
    });

    it("extracts URL-encoded params correctly", async () => {
      // The actor id has spaces; the URL must encode them as %20.
      // NodeHttpServer decodes the param, so the handler receives "actor with spaces".
      const specialActor = makeLiveActor(fighterActor, "actor with spaces");
      const specialGame = makeGame([specialActor], new MockSettings());
      const specialServer = new NodeHttpServer();
      const specialTracker = new DriftTracker();
      registerRoutes(specialServer, specialGame, specialTracker, API_KEY);
      const specialPort = await specialServer.listen(0);

      try {
        const { status } = await httpGet(specialPort, "/nexus/actors/actor%20with%20spaces");
        expect(status).toBe(200);
      } finally {
        await specialServer.close();
      }
    });
  });

  // ── POST /nexus/actors/:id/apply ───────────────────────────────────────────

  describe("POST /nexus/actors/:id/apply", () => {
    it("applies a canonical build to an actor", async () => {
      const { status, body } = await httpPost(port, "/nexus/actors/a1/apply", {
        build: fighterBuild,
      });
      expect(status).toBe(200);
      expect((body as { applied: boolean }).applied).toBe(true);
    });

    it("returns 400 when body is missing build", async () => {
      const { status } = await httpPost(port, "/nexus/actors/a1/apply", {});
      expect(status).toBe(400);
    });

    it("returns 404 for a nonexistent actor", async () => {
      const { status } = await httpPost(port, "/nexus/actors/nope/apply", {
        build: fighterBuild,
      });
      expect(status).toBe(404);
    });

    it("returns 400 when body is not valid JSON", async () => {
      // Send raw invalid JSON.
      const resp = await fetch(`http://127.0.0.1:${port}/nexus/actors/a1/apply`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${API_KEY}`,
        },
        body: "not json at all",
      });
      // The handler receives body=undefined, which triggers the 400 "missing build".
      expect(resp.status).toBe(400);
    });
  });

  // ── GET /nexus/drift ───────────────────────────────────────────────────────

  describe("GET /nexus/drift", () => {
    beforeEach(() => {
      tracker.clear();
    });

    it("returns empty drift report when no changes", async () => {
      const { status, body } = await httpGet(port, "/nexus/drift");
      expect(status).toBe(200);
      expect((body as { total: number }).total).toBe(0);
    });

    it("returns dirty actors after markChanged", async () => {
      tracker.markChanged("a1", "Actor.a1", "Fighter", "hash123");
      const { status, body } = await httpGet(port, "/nexus/drift");
      expect(status).toBe(200);
      const b = body as { total: number; dirty: Array<{ actorUuid: string }> };
      expect(b.total).toBe(1);
      expect(b.dirty[0]!.actorUuid).toBe("Actor.a1");
    });
  });

  // ── 404 for unknown routes ─────────────────────────────────────────────────

  describe("unknown routes", () => {
    it("returns 404 for an unregistered path", async () => {
      const { status } = await httpGet(port, "/nexus/unknown");
      expect(status).toBe(404);
    });

    it("returns 404 for wrong method on a registered path", async () => {
      const resp = await fetch(`http://127.0.0.1:${port}/nexus/actors`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${API_KEY}`,
        },
        body: "{}",
      });
      expect(resp.status).toBe(404);
    });
  });
});

// ── Standalone server lifecycle tests ────────────────────────────────────────

describe("NodeHttpServer — lifecycle", () => {
  it("listen returns the actual port when using 0", async () => {
    const server = new NodeHttpServer();
    const port = await server.listen(0);
    expect(port).toBeGreaterThan(0);
    expect(port).toBeLessThan(65536);
    await server.close();
  });

  it("listen on a specific port returns that port", async () => {
    const server = new NodeHttpServer();
    const port = await server.listen(13456);
    expect(port).toBe(13456);
    await server.close();
  });

  it("close stops accepting connections", async () => {
    const server = new NodeHttpServer();
    const port = await server.listen(0);
    await server.close();

    // A fetch to a closed server should fail (connection refused).
    await expect(fetch(`http://127.0.0.1:${port}/nexus/actors`)).rejects.toThrow();
  });

  it("listeningPort is 0 before listen", () => {
    const server = new NodeHttpServer();
    expect(server.listeningPort).toBe(0);
  });

  it("listeningPort reflects the actual port after listen", async () => {
    const server = new NodeHttpServer();
    const port = await server.listen(0);
    expect(server.listeningPort).toBe(port);
    await server.close();
  });

  it("close is a no-op when server was never started", async () => {
    const server = new NodeHttpServer();
    await expect(server.close()).resolves.toBeUndefined();
  });
});
