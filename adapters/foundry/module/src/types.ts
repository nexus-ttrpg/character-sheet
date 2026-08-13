// Minimal Foundry VTT API type declarations for the relay module (§7.1).
//
// These are the *only* Foundry API surfaces the relay module touches. They're
// intentionally minimal — the relay module is thin glue around foundry-mapping,
// not a full Foundry type package. A real deployment would install
// @league-of-foundry-developers/foundry-vtt-types for IDE autocomplete; here we
// declare just what we need so the code type-checks without that dependency.
//
// The interfaces are abstract enough to mock in tests (§16): the pure logic
// (drift tracking, endpoint handlers, API key validation) is tested with mock
// implementations of these interfaces.

// ── Foundry Actor (the live document, not the mapping type) ──────────────────
//
// `FoundryActor` from foundry-mapping is the *serializable* shape the mapping
// functions consume. A live Foundry Actor document is richer — it has methods
// like `toObject()`, `update()`, `uuid`. The relay module converts between the
// live document and the mapping type.

export interface LiveActor {
  readonly id: string;
  readonly name: string;
  readonly type: string;
  readonly uuid: string;
  /** Serialize to the plain-object shape foundry-mapping consumes. */
  toObject(): unknown;
  /** Update the actor's system data + items (§7.2 — max HP only, no session state). */
  update(data: Record<string, unknown>): Promise<LiveActor>;
}

// ── Foundry game (the global `game` object) ──────────────────────────────────

export interface FoundryGame {
  readonly actors: ActorCollection;
  readonly settings: FoundrySettings;
  /** The system id — "pf2e" or "sf2e" (§3.4). */
  readonly system: { id: string };
}

export interface ActorCollection {
  get(id: string): LiveActor | undefined;
  /** All character-type actors in the world. */
  readonly contents: LiveActor[];
}

// ── Foundry settings (module settings API) ───────────────────────────────────

export interface FoundrySettings {
  register(moduleId: string, key: string, options: SettingOptions): void;
  get(moduleId: string, key: string): unknown;
  set(moduleId: string, key: string, value: unknown): Promise<void>;
}

export interface SettingOptions {
  scope: "world" | "client";
  config: boolean;
  default?: unknown;
  type?: unknown;
}

// ── Foundry Hooks (event system) ─────────────────────────────────────────────

export interface FoundryHooks {
  on(event: string, handler: (...args: unknown[]) => void): void;
}

// ── Foundry HTTP server (route registration) ─────────────────────────────────
//
// Foundry VTT modules can register local HTTP routes. The exact API varies by
// Foundry version — this interface abstracts it so the endpoint logic is
// testable. A real deployment wires this to Foundry's Express integration.

export interface RelayHttpServer {
  registerRoute(
    method: "GET" | "POST",
    path: string,
    handler: (req: RelayRequest) => Promise<RelayResponse>,
  ): void;
}

export interface RelayRequest {
  /** Bearer token from the Authorization header. */
  authHeader?: string;
  /** URL path parameters (e.g. { id: "abc" } for /actors/:id). */
  params: Record<string, string>;
  /** Parsed JSON body (POST routes only). */
  body?: unknown;
}

export interface RelayResponse {
  status: number;
  body: unknown;
}
