// Relay module tests (§16).
//
// The relay module's pure logic — drift tracking, API key validation, endpoint
// handlers — is tested with mock implementations of the Foundry API interfaces
// from types.ts. No real Foundry needed.
//
// These tests cover:
//   - settings.ts: API key generation format, auth validation, constant-time comparison
//   - drift.ts: DriftTracker mark/resolve/report, actorContentHash determinism
//   - endpoints.ts: listActors, getActor, applyActor, getDrift, withAuth 401, registerRoutes
//   - relay.ts: initPhase registers setting, readyPhase wires hooks + routes + key

import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import { exportActor, importActor, type FoundryActor } from "@nexus-sheet/foundry-mapping";

import {
  generateApiKey,
  registerApiKeySetting,
  getOrGenerateApiKey,
  validateAuthHeader,
  MODULE_ID,
  SETTING_KEY,
} from "./settings.js";
import { DriftTracker, actorContentHash, type DriftReport } from "./drift.js";
import {
  withAuth,
  listActors,
  getActor,
  applyActor,
  getDrift,
  registerRoutes,
} from "./endpoints.js";
import { initPhase, readyPhase } from "./relay.js";
import type {
  FoundryGame,
  FoundryHooks,
  FoundrySettings,
  RelayHttpServer,
  RelayRequest,
  RelayResponse,
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

// ── Mock Foundry API factories ───────────────────────────────────────────────

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

  /** Test helper: directly set a stored value (bypassing register). */
  directSet(key: string, value: unknown): void {
    this.store.set(key, value);
  }
}

class MockHooks implements FoundryHooks {
  handlers = new Map<string, Array<(...args: unknown[]) => void>>();

  on(event: string, handler: (...args: unknown[]) => void): void {
    if (!this.handlers.has(event)) {
      this.handlers.set(event, []);
    }
    this.handlers.get(event)!.push(handler);
  }

  /** Test helper: fire an event as Foundry would. */
  fire(event: string, ...args: unknown[]): void {
    const handlers = this.handlers.get(event) ?? [];
    for (const h of handlers) {
      h(...args);
    }
  }
}

class MockHttpServer implements RelayHttpServer {
  routes = new Map<string, (req: RelayRequest) => Promise<RelayResponse>>();

  registerRoute(method: "GET" | "POST", path: string, handler: (req: RelayRequest) => Promise<RelayResponse>): void {
    this.routes.set(`${method} ${path}`, handler);
  }

  /** Test helper: invoke a registered route. */
  async call(method: "GET" | "POST", path: string, req: RelayRequest): Promise<RelayResponse> {
    const handler = this.routes.get(`${method} ${path}`);
    if (!handler) return { status: 404, body: { error: "route not found" } };
    return handler(req);
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

// ── settings.ts ──────────────────────────────────────────────────────────────

describe("settings — API key management", () => {
  it("generateApiKey produces nsk_relay_ prefix with 32 hex chars", () => {
    const key = generateApiKey();
    expect(key).toMatch(/^nsk_relay_[0-9a-f]{32}$/);
  });

  it("generateApiKey produces unique keys", () => {
    const keys = new Set<string>();
    for (let i = 0; i < 100; i++) {
      keys.add(generateApiKey());
    }
    expect(keys.size).toBe(100);
  });

  it("registerApiKeySetting registers a world-scoped, non-config setting", () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const opts = settings.registered.get(SETTING_KEY);
    expect(opts).toBeDefined();
    expect(opts!.scope).toBe("world");
    expect(opts!.config).toBe(false);
  });

  it("getOrGenerateApiKey returns existing key if present", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    settings.directSet(SETTING_KEY, "nsk_relay_existing");
    const key = await getOrGenerateApiKey(settings);
    expect(key).toBe("nsk_relay_existing");
  });

  it("getOrGenerateApiKey generates and stores a new key if absent", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const key = await getOrGenerateApiKey(settings);
    expect(key).toMatch(/^nsk_relay_[0-9a-f]{32}$/);
    // The generated key is now stored.
    const stored = settings.get(MODULE_ID, SETTING_KEY);
    expect(stored).toBe(key);
  });

  it("validateAuthHeader accepts correct Bearer token", () => {
    const key = "nsk_relay_abcdef0123456789abcdef0123456789";
    expect(validateAuthHeader(`Bearer ${key}`, key)).toBe(true);
  });

  it("validateAuthHeader rejects missing header", () => {
    expect(validateAuthHeader(undefined, "nsk_relay_key")).toBe(false);
  });

  it("validateAuthHeader rejects wrong scheme", () => {
    const key = "nsk_relay_key";
    expect(validateAuthHeader(`Basic ${key}`, key)).toBe(false);
  });

  it("validateAuthHeader rejects wrong key", () => {
    const key = "nsk_relay_correct";
    expect(validateAuthHeader("Bearer nsk_relay_wrong", key)).toBe(false);
  });

  it("validateAuthHeader rejects key of different length (constant-time)", () => {
    const key = "nsk_relay_abcdef0123456789abcdef0123456789";
    expect(validateAuthHeader("Bearer nsk_relay_short", key)).toBe(false);
  });
});

// ── drift.ts ─────────────────────────────────────────────────────────────────

describe("drift — DriftTracker", () => {
  it("starts empty", () => {
    const tracker = new DriftTracker();
    const report = tracker.report();
    expect(report.dirty).toEqual([]);
    expect(report.clean).toEqual([]);
    expect(report.total).toBe(0);
  });

  it("markChanged adds a dirty entry", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("actor1", "Actor.actor1", "Aragorn", "hash1");
    const report = tracker.report();
    expect(report.dirty).toHaveLength(1);
    expect(report.dirty[0]!.actorUuid).toBe("Actor.actor1");
    expect(report.dirty[0]!.actorName).toBe("Aragorn");
    expect(report.dirty[0]!.synced).toBe(false);
    expect(report.total).toBe(1);
  });

  it("markSynced moves an actor from dirty to clean", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("actor1", "Actor.actor1", "Aragorn", "hash1");
    tracker.markSynced("actor1");
    const report = tracker.report();
    expect(report.dirty).toEqual([]);
    expect(report.clean).toEqual(["actor1"]);
    expect(report.total).toBe(1);
  });

  it("markChanged on same actor updates the entry", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("actor1", "Actor.actor1", "Aragorn", "hash1");
    tracker.markChanged("actor1", "Actor.actor1", "Aragorn II", "hash2");
    const report = tracker.report();
    expect(report.dirty).toHaveLength(1);
    expect(report.dirty[0]!.actorName).toBe("Aragorn II");
    expect(report.dirty[0]!.contentHash).toBe("hash2");
  });

  it("markSynced on unknown actor is a no-op", () => {
    const tracker = new DriftTracker();
    tracker.markSynced("nonexistent");
    expect(tracker.report().total).toBe(0);
  });

  it("isDirty returns true for changed, false for synced", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("actor1", "Actor.actor1", "A", "h1");
    expect(tracker.isDirty("actor1")).toBe(true);
    tracker.markSynced("actor1");
    expect(tracker.isDirty("actor1")).toBe(false);
  });

  it("isDirty returns false for unknown actor", () => {
    const tracker = new DriftTracker();
    expect(tracker.isDirty("unknown")).toBe(false);
  });

  it("get returns the entry for a known actor", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("actor1", "Actor.actor1", "Aragorn", "hash1");
    const entry = tracker.get("actor1");
    expect(entry).toBeDefined();
    expect(entry!.actorName).toBe("Aragorn");
  });

  it("get returns undefined for unknown actor", () => {
    const tracker = new DriftTracker();
    expect(tracker.get("unknown")).toBeUndefined();
  });

  it("clear removes all entries", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("a1", "Actor.a1", "A", "h1");
    tracker.markChanged("a2", "Actor.a2", "B", "h2");
    tracker.clear();
    expect(tracker.report().total).toBe(0);
  });

  it("report separates dirty and clean actors", () => {
    const tracker = new DriftTracker();
    tracker.markChanged("a1", "Actor.a1", "A", "h1");
    tracker.markChanged("a2", "Actor.a2", "B", "h2");
    tracker.markChanged("a3", "Actor.a3", "C", "h3");
    tracker.markSynced("a2");
    const report = tracker.report();
    expect(report.dirty).toHaveLength(2);
    expect(report.clean).toEqual(["a2"]);
    expect(report.total).toBe(3);
  });
});

describe("drift — actorContentHash", () => {
  it("is deterministic for the same input", () => {
    const json = JSON.stringify({ name: "Aragorn", level: 5 });
    expect(actorContentHash(json)).toBe(actorContentHash(json));
  });

  it("differs for different input", () => {
    const a = JSON.stringify({ name: "Aragorn", level: 5 });
    const b = JSON.stringify({ name: "Aragorn", level: 6 });
    expect(actorContentHash(a)).not.toBe(actorContentHash(b));
  });

  it("produces an 8-char hex string", () => {
    const hash = actorContentHash('{"test":true}');
    expect(hash).toMatch(/^[0-9a-f]{8}$/);
  });
});

// ── endpoints.ts ─────────────────────────────────────────────────────────────

describe("endpoints — withAuth", () => {
  it("passes through to handler when auth is valid", async () => {
    const key = "nsk_relay_testkey";
    const handler = vi.fn(async () => ({ status: 200, body: { ok: true } }));
    const wrapped = withAuth(key, handler);
    const res = await wrapped({ authHeader: `Bearer ${key}`, params: {} });
    expect(res.status).toBe(200);
    expect(handler).toHaveBeenCalledOnce();
  });

  it("returns 401 when auth header is missing", async () => {
    const handler = vi.fn(async () => ({ status: 200, body: {} }));
    const wrapped = withAuth("key", handler);
    const res = await wrapped({ params: {} });
    expect(res.status).toBe(401);
    expect(handler).not.toHaveBeenCalled();
  });

  it("returns 401 when key is wrong", async () => {
    const handler = vi.fn(async () => ({ status: 200, body: {} }));
    const wrapped = withAuth("correct", handler);
    const res = await wrapped({ authHeader: "Bearer wrong", params: {} });
    expect(res.status).toBe(401);
    expect(handler).not.toHaveBeenCalled();
  });
});

describe("endpoints — listActors", () => {
  it("returns all character actors with id/name/uuid", async () => {
    const actor1 = makeLiveActor(fighterActor, "a1");
    const npcActor: LiveActor = {
      id: "npc1", name: "Goblin", type: "npc", uuid: "Actor.npc1",
      toObject: () => ({}), update: async () => npcActor,
    };
    const game = makeGame([actor1, npcActor], new MockSettings());
    const res = await listActors(game);
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      actors: [{ id: "a1", name: fighterActor.name, uuid: "Actor.a1" }],
      system: "pf2e",
    });
  });

  it("returns empty list when no character actors exist", async () => {
    const game = makeGame([], new MockSettings());
    const res = await listActors(game);
    expect(res.status).toBe(200);
    expect((res.body as { actors: unknown[] }).actors).toEqual([]);
  });
});

describe("endpoints — getActor", () => {
  it("returns canonical build for a character actor", async () => {
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const res = await getActor(game, { params: { id: "a1" } });
    expect(res.status).toBe(200);
    const body = res.body as { actor_id: string; actor_uuid: string; build: unknown };
    expect(body.actor_id).toBe("a1");
    expect(body.actor_uuid).toBe("Actor.a1");
    // The build should have identity.name matching the fixture.
    expect((body.build as { identity: { name: string } }).identity.name).toBe(
      fighterBuild.identity.name,
    );
  });

  it("returns 404 for a nonexistent actor", async () => {
    const game = makeGame([], new MockSettings());
    const res = await getActor(game, { params: { id: "nope" } });
    expect(res.status).toBe(404);
  });

  it("returns 400 for a non-character actor", async () => {
    const npc: LiveActor = {
      id: "npc1", name: "Goblin", type: "npc", uuid: "Actor.npc1",
      toObject: () => ({}), update: async () => npc,
    };
    const game = makeGame([npc], new MockSettings());
    const res = await getActor(game, { params: { id: "npc1" } });
    expect(res.status).toBe(400);
  });
});

describe("endpoints — applyActor", () => {
  it("applies a canonical build to an actor via update()", async () => {
    const actor = makeLiveActor(fighterActor, "a1");
    const updateSpy = vi.spyOn(actor, "update");
    const game = makeGame([actor], new MockSettings());

    const res = await applyActor(game, {
      params: { id: "a1" },
      body: { build: fighterBuild },
    });

    expect(res.status).toBe(200);
    expect((res.body as { applied: boolean }).applied).toBe(true);
    expect(updateSpy).toHaveBeenCalledOnce();
    // The update call should carry the exported actor's system + items.
    const updateArg = updateSpy.mock.calls[0]![0];
    expect(updateArg.name).toBe(fighterBuild.identity.name);
    expect(updateArg.system).toBeDefined();
    expect(updateArg.items).toBeDefined();
  });

  it("returns 404 for a nonexistent actor", async () => {
    const game = makeGame([], new MockSettings());
    const res = await applyActor(game, {
      params: { id: "nope" },
      body: { build: fighterBuild },
    });
    expect(res.status).toBe(404);
  });

  it("returns 400 when body is missing build", async () => {
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const res = await applyActor(game, { params: { id: "a1" } });
    expect(res.status).toBe(400);
  });

  it("returns 400 when body is missing entirely", async () => {
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const res = await applyActor(game, { params: { id: "a1" }, body: undefined });
    expect(res.status).toBe(400);
  });
});

describe("endpoints — getDrift", () => {
  it("returns the drift tracker's report", async () => {
    const tracker = new DriftTracker();
    tracker.markChanged("a1", "Actor.a1", "A", "h1");
    const res = await getDrift(tracker);
    expect(res.status).toBe(200);
    expect((res.body as DriftReport).dirty).toHaveLength(1);
    expect((res.body as DriftReport).total).toBe(1);
  });

  it("returns empty report when no changes", async () => {
    const tracker = new DriftTracker();
    const res = await getDrift(tracker);
    expect(res.status).toBe(200);
    expect((res.body as DriftReport).total).toBe(0);
  });
});

describe("endpoints — registerRoutes", () => {
  it("registers 4 routes on the server", () => {
    const server = new MockHttpServer();
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const tracker = new DriftTracker();

    registerRoutes(server, game, tracker, "testkey");

    expect(server.routes.size).toBe(4);
    expect(server.routes.has("GET /nexus/actors")).toBe(true);
    expect(server.routes.has("GET /nexus/actors/:id")).toBe(true);
    expect(server.routes.has("POST /nexus/actors/:id/apply")).toBe(true);
    expect(server.routes.has("GET /nexus/drift")).toBe(true);
  });

  it("registered routes enforce auth", async () => {
    const server = new MockHttpServer();
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const tracker = new DriftTracker();

    registerRoutes(server, game, tracker, "secret");

    // No auth header → 401.
    const res = await server.call("GET", "/nexus/actors", { params: {} });
    expect(res.status).toBe(401);
  });

  it("registered GET /nexus/actors returns actor list with valid auth", async () => {
    const server = new MockHttpServer();
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());
    const tracker = new DriftTracker();

    registerRoutes(server, game, tracker, "secret");

    const res = await server.call("GET", "/nexus/actors", {
      authHeader: "Bearer secret",
      params: {},
    });
    expect(res.status).toBe(200);
    expect((res.body as { actors: unknown[] }).actors).toHaveLength(1);
  });

  it("registered GET /nexus/drift returns report with valid auth", async () => {
    const server = new MockHttpServer();
    const game = makeGame([], new MockSettings());
    const tracker = new DriftTracker();
    tracker.markChanged("a1", "Actor.a1", "A", "h1");

    registerRoutes(server, game, tracker, "secret");

    const res = await server.call("GET", "/nexus/drift", {
      authHeader: "Bearer secret",
      params: {},
    });
    expect(res.status).toBe(200);
    expect((res.body as DriftReport).total).toBe(1);
  });
});

// ── relay.ts ─────────────────────────────────────────────────────────────────

describe("relay — initPhase", () => {
  it("registers the API key setting", () => {
    const settings = new MockSettings();
    const game = makeGame([], settings);
    initPhase(game);
    expect(settings.registered.has(SETTING_KEY)).toBe(true);
  });
});

describe("relay — readyPhase", () => {
  it("generates an API key if none exists", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const game = makeGame([], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    await readyPhase(game, hooks, server);

    const key = settings.get(MODULE_ID, SETTING_KEY) as string;
    expect(key).toMatch(/^nsk_relay_[0-9a-f]{32}$/);
  });

  it("reuses existing API key if present", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    settings.directSet(SETTING_KEY, "nsk_relay_existing");
    const game = makeGame([], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    await readyPhase(game, hooks, server);

    expect(settings.get(MODULE_ID, SETTING_KEY)).toBe("nsk_relay_existing");
  });

  it("registers 4 HTTP routes", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const game = makeGame([], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    await readyPhase(game, hooks, server);

    expect(server.routes.size).toBe(4);
  });

  it("hooks createActor and updateActor for drift tracking", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    const tracker = await readyPhase(game, hooks, server);

    // Fire createActor — should mark the actor as dirty.
    hooks.fire("createActor", actor);
    expect(tracker.isDirty("a1")).toBe(true);

    // Fire updateActor — should update the drift entry.
    hooks.fire("updateActor", actor);
    expect(tracker.report().dirty).toHaveLength(1);
  });

  it("ignores non-character actors in drift hooks", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const npc: LiveActor = {
      id: "npc1", name: "Goblin", type: "npc", uuid: "Actor.npc1",
      toObject: () => ({}), update: async () => npc,
    };
    const game = makeGame([npc], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    const tracker = await readyPhase(game, hooks, server);

    hooks.fire("createActor", npc);
    expect(tracker.report().total).toBe(0);
  });

  it("returns a DriftTracker instance", async () => {
    const settings = new MockSettings();
    registerApiKeySetting(settings);
    const game = makeGame([], settings);
    const hooks = new MockHooks();
    const server = new MockHttpServer();

    const tracker = await readyPhase(game, hooks, server);

    expect(tracker).toBeInstanceOf(DriftTracker);
  });
});

// ── Round-trip: relay uses the same mapping as the adapter (§7.1) ─────────────

describe("relay — foundry-mapping round-trip (§7.1)", () => {
  it("getActor → applyActor round-trips through foundry-mapping", async () => {
    // This is the §7.1 guarantee: the relay imports foundry-mapping by plain
    // import, so the conversion is identical to the server-side adapter.
    const actor = makeLiveActor(fighterActor, "a1");
    const game = makeGame([actor], new MockSettings());

    // Import: Actor → canonical build.
    const importRes = await getActor(game, { params: { id: "a1" } });
    expect(importRes.status).toBe(200);
    const importedBuild = (importRes.body as { build: unknown }).build;

    // The imported build should reconstruct the same fields via importActor.
    const expectedBuild = importActor(fighterActor);
    expect(importedBuild).toEqual(expectedBuild);
  });
});
