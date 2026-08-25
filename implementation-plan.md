# Nexus Sheet — Implementation Plan

**Status:** Phase 2 in progress. Phase 0 (foundations) ✅, Phase 1 (adapter boundary, GO — ADR 0001) ✅, Phase 2.1 (Axum API) ✅, Phase 2.2 (Roll20 adapter) ✅, Phase 2.3 (Foundry relay module) ✅, Phase 2.4 (SF2e system plugin + Foundry SF2e coverage) ✅. 288 tests passing (132 Rust + 156 TS).
**Relationship to the architecture doc:** This plan decomposes the §19 roadmap into sequenced workstreams with concrete deliverables, dependencies, and verification gates. It does not change any architecture decision — every choice here is justified by a §N reference back to the proposal. Where this plan adds detail the proposal leaves implicit, it says so.

The four load-bearing invariants from the proposal are non-negotiable and every task below defers to them: (1) the language boundary — Rust core, TS adapters, and a contributor writing an adapter never installs Rust (§0.1); (2) `sync-engine` is pure and I/O-free (§1, §11.3); (3) one-way codegen from Rust types toward the less-expressive TS target, with committed output and a CI staleness check (§3.6); (4) conflicts are never auto-resolved and rollback is `revert`, not `reset --hard` (§11.3, §11.4).

---

## How to read this plan

- **Phase → Workstream → Task.** Phases match §19. Each workstream has a goal, concrete deliverables (files/artifacts), what it depends on, how it is verified, and the risk it carries.
- **Gates.** Each phase ends with a gate: an explicit, runnable check that must pass before the next phase begins. Phase 1's gate is a go/no-go decision point, not just a test pass (§0.2, §19).
- **Critical path.** The dependency chain that determines the shortest possible schedule is called out in §A. Slipping a critical-path task slips the whole plan; slipping a parallelizable task does not.
- **Verify blocks name commands.** These are the intended commands from §16. None are runnable yet — the workspace has no code. They are the *definition of done* for each task.

---

## Critical path and dependency summary

```
Phase 0 (foundations) ──────────────────────────────────────────────
  0.1 repo scaffold ─┬─► 0.2 schema-core ─┬─► 0.4 codegen + staleness ──► (generated TS types exist)
                     │                    │
                     │                    ├─► 0.5 sync-engine + proptest  (depends ONLY on schema-core: §15)
                     │                    │
                     │                    └─► 0.7 fixtures (shared JSON, both languages)
                     │
                     └─► 0.3 schema-pf2e (extends schema-core) ──► 0.6 store (SQLx, PG+SQLite)
                                                            │
Phase 1 (adapter boundary — RISK CONCENTRATION) ◄───────────┘
  0.4 generated types ─┐
  0.5 sync-engine ─────┼─► 1.1 adapter-host (deno_core) ─┐
  0.7 fixtures ────────┤                                 ├─► 1.2 adapter-sdk + contract harness ─┐
                       │                                 │                                       ├─► 1.3 Foundry adapter (reference)
                       └─► packages/foundry-mapping ◄────┘                                       │
                                                                                                  │
  1.1 + 1.3 ─► 1.4 CLI (nexus import/push/diff) ◄────────────────────────────────────────────────┘
                       │
                       └─► GATE 1: GO / NO-GO  ◄── the whole split is re-evaluated here (§0.2)

Phase 2 (API + second adapter) ◄── GATE 1 pass
  1.4 CLI ─► 2.1 Axum API (CRUD, connections, sync, webhooks, auth)
  2.1 ─► 2.2 Roll20 adapter (collector + handshake + chunked apply)
  packages/foundry-mapping ─► 2.3 Foundry relay module
  0.3 schema-core ─► 2.4 SF2e system plugin + Foundry SF2e coverage

Phase 3 (conflict UX + hardening) ◄── GATE 2 pass
  0.5 sync-engine (WASM build) ─► 3.1 web UI (history + conflict resolution + diff preview)
  0.6 store ─► 3.2 exercise migration framework (one intentional v1→v2)
  everything ─► 3.3 security pass, rate limiting, Docker, self-host docs
  1.2 adapter-sdk ─► 3.4 adapter-authoring guide + example-adapter template
```

The single most sequence-sensitive fact: **the codegen staleness check (0.4) must exist before anything depends on generated TS types** (§19 Phase 0, §3.6). `sync-engine` (0.5) is pure and depends only on `schema-core` — it can and should be built and property-tested in parallel with `schema-pf2e` and `store`, because the Cargo boundary makes it impossible to accidentally couple them (§15).

---

## Phase 0 — Foundations (weeks 1–4) ✅ COMPLETE

Goal: the correctness-critical core exists, is property-tested, and the cross-language schema guarantee is enforced by CI before anything builds on top of it.

### 0.1 Repository scaffold and CI skeleton

**Goal:** two workspaces side by side (Cargo + pnpm), xtask wired, CI running on every push.

**Deliverables (per §15):**
- `Cargo.toml` (Rust workspace), `pnpm-workspace.yaml` (TS workspace).
- Empty crate skeletons: `crates/{schema-core,schema-pf2e,schema-sf2e,sync-engine,store,adapter-host,api}`.
- Empty package skeletons: `packages/{schema-types,schema-zod,adapter-sdk,foundry-mapping}` (the two generated ones marked generated in `package.json` description + a file header).
- `xtask/` with a `cargo xtask codegen` stub (implemented in 0.4) and `openapi`/`fixtures` stubs.
- `.github/workflows/`: `rust.yml` (`cargo test`, `cargo xtask codegen && git diff --exit-code`), `ts.yml` (`pnpm test`), `audit.yml` (`cargo-deny`/`cargo-audit` + `pnpm audit`, per §14, §17).
- `crates/sync-engine` `Cargo.toml` with a dependency on `schema-core` and *nothing else* — this is the boundary §15 makes a compile error to violate. Add a CI check asserting `sync-engine`'s dep set.

**Depends on:** nothing.
**Verify:** `cargo build --workspace` and `pnpm install` both succeed; the `sync-engine` dep-isolation check passes.
**Risk:** low. Doing the dep-isolation check now means a later violation is a CI failure, not a review comment.

### 0.2 `schema-core` — the source of truth

**Goal:** the base canonical Rust types that everything else is generated from (§3.1, §3.6).

**Deliverables:**
- `crates/schema-core`: `Character`, `CharacterMeta` (§3.2), `CharacterBuild` (§3.3), `SessionState` (§4, with the structurally-enforced no-conversion-into-`CharacterBuild` property), `ExtensionBag` (§3.5), `SystemRefId` (§9.1).
- `#[serde(rename_all = "camelCase", deny_unknown_fields)]` on typed regions (§3.1); `deny_unknown_fields` deliberately absent on `ExtensionBag`.
- `BTreeMap` (not `HashMap`) for all map fields — the determinism requirement for content hashing (§3.3, §9.2).
- Newtypes: `Level` (1..=20 enforced on construction), `SkillKey`, `ContentHash`, `Ulid`, `Slug` (§3.3).
- `serde` + `schemars` (`JsonSchema`) + `garde` (semantic validation) wired per the three-layer split (§3.1).

**Depends on:** 0.1.
**Verify:** `cargo test -p schema-core`; `schemars` emits a `canonical.schema.json` artifact for the base types.
**Risk:** getting `BTreeMap`/newtypes/`deny_unknown_fields` right early is cheap; fixing it later forces a content-hash-affecting change that invalidates stored snapshots. Do it now.

### 0.3 `schema-pf2e` — PF2e SystemPlugin

**Goal:** the PF2e ruleset as a compiled-in `SystemPlugin` (§10.2), extending `schema-core`.

**Deliverables:**
- `crates/schema-pf2e`: implements `SystemPlugin` (`id`, `json_schema`, `validate`, `derived_stats`, `content_index` — §10.2).
- PF2e-specific structs referenced by `CharacterBuild` in 0.2 (ancestry/heritage/background/class/feat/spellcasting shapes).
- A `SystemPlugin` registry trait so `(system, platform) → adapter` resolves at runtime (§10.3).

**Depends on:** 0.2. Parallelizable with 0.4/0.5/0.6 once 0.2's base types stabilize.
**Verify:** `cargo test -p schema-pf2e`; the plugin's `validate` accepts the PF2e fixtures from 0.7 and rejects known-bad input.

### 0.4 Codegen pipeline + staleness check  ⚑ CRITICAL PATH

**Goal:** the one-way pipeline (Rust types → `canonical.schema.json` → `@nexus-sheet/schema-types` + `@nexus-sheet/schema-zod`) committed and CI-enforced (§3.6).

**Deliverables:**
- `xtask/src/codegen.rs`: runs `schemars` over registered types → writes `canonical.schema.json` → runs `json-schema-to-typescript` and `json-schema-to-zod` → writes `packages/schema-types/` and `packages/schema-zod/`.
- Generated file header comments + `package.json` descriptions marking them generated (§15, §21 risk row).
- CI job: `cargo xtask codegen && git diff --exit-code` (§16, §3.6). **This job must be green before 0.2's types are consumed by any TS package.**
- `canonicalPath()` template-literal type helper in `adapter-sdk` (lands fully in 1.2 but the type machinery depends on generated types existing).

**Depends on:** 0.2 (needs types to generate from).
**Verify:** the staleness check fails when a generated file is hand-edited or when Rust types change without regen; passes after `cargo xtask codegen`.
**Risk:** this is the single control backing the "one source of truth" guarantee (§21). If it lands late, TS contributors build against stale types and divergence shows up as silent adapter/core rejection (§3.6). It is sequenced first among the "depends on generated types" tasks for exactly this reason.

### 0.5 `sync-engine` — diff, merge, DAG, content hashing + proptest suite  ⚑ CRITICAL PATH

**Goal:** the pure, I/O-free engine that everything else trusts, with algebraic laws asserted from day one (§11, §16).

**Deliverables:**
- `crates/sync-engine`: `Snapshot`, `SnapshotOrigin` enum (§11.1); structural `diff` with identity-matched collection pairing (§11.2); three-way `merge(ancestor, ours, theirs) -> MergePreview` (§11.3); rollback-as-`revert` over the snapshot DAG (§11.4); BLAKE3 `content_hash` of canonicalized `build` only (§9.2).
- `merge` signature takes `&CharacterBuild` — structurally cannot see `session` (§3.1, §4).
- **Proptest suite from day one** (§16): merge identity; `merge(a, x, x)` never conflicts; single-sided changes never lost; `hash(x) == hash(roundtrip(x))`; identical inputs never produce conflicts (§21 risk row).

**Depends on:** 0.2 only (the §15 hard boundary). Parallelizable with 0.3/0.4/0.6.
**Verify:** `cargo test -p sync-engine`; `cargo test -p sync-engine --features proptest` (or however the crate exposes it) with a configurable iteration count high enough to catch interleaving bugs.
**Risk:** the engine is the asset (§0). A wrong merge silently corrupts a character a user spent months building (§0.1) — the proptest suite is not optional polish, it is the correctness argument for the whole Rust bet (§0.2).

### 0.6 `store` — SQLx, PostgreSQL + SQLite

**Goal:** one codebase targeting both hosted (Postgres) and offline CLI (SQLite) stores (§12, §14).

**Deliverables:**
- `crates/store`: SQLx queries + migrations for the snapshot DAG, characters, connections, content-hash index.
- JSONB `build` documents in Postgres; SQLite equivalent (§14). ULID-ordered cursor pagination (§18).
- Provider swap via SQLx's feature-gated drivers; query code shared.
- `secrecy::Secret` wrappers around credential material at rest; AES-256-GCM encryption of connection credentials (§17).

**Depends on:** 0.2 (schema types), 0.5 (snapshot/content-hash shapes). Parallelizable with 0.4.
**Verify:** `cargo test -p store` against `testcontainers` Postgres and an in-memory/temp SQLite; round-trip a snapshot and assert `content_hash` is stable across read-back.

### 0.7 Fixtures — shared ground-truth

**Goal:** exactly one set of canonical characters, consumed by both languages as plain JSON (§16).

**Deliverables:**
- `examples/fixtures/`: PF2e characters across complexity tiers (fresh level 1, multiclassed level 10, companion-heavy) and at least one SF2e fixture (drone-heavy mechanic) to exercise §3.4 shapes.
- One fixture per existing schema version (initially v1; grows as migrations land in 3.2).
- A manifest mapping each fixture to the complexity tier and system it exercises.

**Depends on:** 0.2 (types to validate against), 0.3 (PF2e validation). Needed by 0.5's proptest and by 1.2's contract harness.
**Verify:** every fixture deserializes into `CharacterBuild` and passes `schema-pf2e`'s `validate`.

### Phase 0 gate ✅ PASSED

All of:
- `cargo test --workspace` green; `pnpm test` green (even if mostly stubs).
- `cargo xtask codegen && git diff --exit-code` green — the staleness control is live.
- `sync-engine` proptest laws pass at the agreed iteration count.
- `store` round-trips a snapshot with a stable hash on both Postgres and SQLite.
- Every fixture validates against its system plugin.
- The `sync-engine` dep-isolation CI check passes (it depends on `schema-core` and nothing else).

If the codegen staleness check is not green here, **stop** — nothing in Phase 1 should build on un-enforced generated types.

---

## Phase 1 — The adapter boundary (weeks 5–10) ✅ COMPLETE (GO — ADR 0001) ⚑ RISK CONCENTRATION

Goal: prove the cross-language machinery end to end with one real adapter. This is where the Rust/TS split either earns its keep or reveals that all-TypeScript was the better call (§0.2, §19).

### 1.1 `adapter-host` — deno_core runtime, ops, capability enforcement

**Goal:** run TS adapters in-process from the Rust server, sandboxed, with boundary validation (§5.5).

**Deliverables:**
- `crates/adapter-host`: embeds `deno_core` (V8 + a deliberately tiny op layer: `fetch`, `log`, nothing else — §21 risk row); loads adapters as ES modules; invokes the `CharacterAdapter` interface.
- Boundary validation: adapter output deserialized and validated against the canonical schema **in Rust before the engine sees it** (§5.5, §1).
- Capability grants from the adapter manifest (§5.5): no filesystem, no environment, no DB handle, network only via an allowlisted host op.
- The `"runtime": "external"` subprocess JSON-RPC escape hatch (§5.5) — same interface, for adapters that genuinely need full Node. (Implementation can be minimal now; it exists so the fallback is real, not theoretical — §21.)
- Timeouts and panic/hang containment so one bad adapter degrades one connection, not the server (§21).

**Depends on:** 0.2, 0.4 (generated types the host validates against), 0.5 (engine the host feeds).
**Verify:** host boundary tests (§16): reject adapter output violating the schema; deny a non-allowlisted `fetch`; contain a panicking/hanging adapter.
**Risk:** `deno_core` is the largest dependency the split adds, with a V8-tied upgrade cadence, on the critical path of every sync (§14.1, §21). Keep the op surface tiny so the integration is shallow and replaceable; the external-runtime fallback is the documented mitigation.

### 1.2 `adapter-sdk` + contract test harness — built alongside 1.3

**Goal:** the contribution surface, with a one-line contract test a new adapter runs against itself (§5, §16).

**Deliverables:**
- `packages/adapter-sdk`: the `CharacterAdapter<TPlatform>` interface (§5.1 — generic over platform format, the ergonomic win TS enables); `ImportOutcome`/`ExportOutcome`/`AdapterWarning`/`AdapterError` (§5.2); `AdapterCapabilities` with static `fieldCoverage` (§5.4); `ImportContext`/`ExportContext` carrying credentials from the connection record (§5.4).
- `@nexus-sheet/adapter-sdk/testing`: `runContractTests(adapter, fixtures)` (§16) asserting: `import(export(fixture))` round-trips within `fieldCoverage`; `validate()` rejects known-bad input; `detectChanges` is a no-op on unchanged input; **unknown content produces warnings, never throws** (§5.2 — the homebrew invariant).
- `canonicalPath()` typed against generated schema types so a mapping typo is a build-time type error (§6.3).

**Depends on:** 0.4 (generated types), 0.7 (fixtures). **Co-evolves with 1.3** — the harness is shaped by real use on the Foundry adapter, not guessed at (§19).
**Verify:** the contract harness runs with `pnpm test` and **no Rust and no host in the loop** (§16, §21 risk row). If a contributor ever needs `cargo` to iterate on an adapter, that is a bug in the SDK.
**Risk:** the "no Rust required" promise is the single most important property of the design (§0.1). The harness running standalone in pure TS is what makes it true; verify it explicitly.

### 1.3 Foundry adapter — the reference implementation (PF2e only)

**Goal:** one real adapter, modeled on by every later adapter (§19, §7).

**Deliverables:**
- `packages/foundry-mapping`: the actual PF2e Actor↔canonical mapping logic, written once (§7.1).
- `adapters/foundry`: the adapter package consuming `foundry-mapping`, implementing `CharacterAdapter` for the Foundry platform format.
- Coverage: identity, ancestry, class, level, abilities, skills, feats, equipment, spellcasting (§19 Phase 1 scope). Max HP only on export — current HP is session state and is never written (§4, §7.2).
- Compendium UUID handling: store in `SystemRefId::foundry_uuid`, export a compendium link when resolvable in the target world, else a fully inlined item tagged `extensions["foundry.pf2e"].inlined_fallback` (§7.3).
- Homebrew as inlined items with `extensions["foundry.pf2e"].homebrew = true`, lossless Foundry→Foundry (§7.7).

**Depends on:** 1.1 (host to run it — though most iteration is pure TS via 1.2), 1.2 (SDK + harness), 0.7 (fixtures).
**Verify:** `pnpm test` in `adapters/foundry` passes the contract harness against the PF2e fixtures; `import(export(fixture))` round-trips within declared `fieldCoverage`.

### 1.4 CLI — `nexus import | push | diff`

**Goal:** the offline-first single static binary driving the Phase 1 flow (§12, §19).

**Deliverables:**
- `cli/` (Rust, `clap`): `nexus import --foundry`, `nexus push --foundry`, `nexus diff` (§19).
- SQLite local store (shares `store` query code, §12); queued ops flush on reconnect (§12).
- Musl static build target (`cargo build --target x86_64-unknown-linux-musl`, §12).

**Depends on:** 1.1 (host), 1.3 (Foundry adapter), 0.6 (store).
**Verify:** `nexus import --foundry <world>` produces a snapshot; `nexus diff` shows field-level changes; `nexus push --foundry` writes back and records a new snapshot.

### Phase 1 gate — GO / NO-GO  ⚑ ✅ PASSED (ADR 0001)

**Decision: GO.** Retain the Rust core / TypeScript adapter split. The
maintenance burden is measurable and bounded (~25 lines of V8 scope/promise
code, `LocalSet` at call sites, esbuild bundle step). The sync-engine's purity,
property-testability, and structural boundary enforcement earn their keep. See
[`docs/adr/0001-phase1-go-no-go-rust-ts-split.md`](./docs/adr/0001-phase1-go-no-go-rust-ts-split.md).

This is not just a test pass; it is the explicit decision point from §0.2/§19/§21. All of:

- The Foundry adapter imports a real PF2e Actor, exports it back, and round-trips within `fieldCoverage` — running in the real `adapter-host`, not mocked.
- Boundary tests prove the core rejects malformed adapter output and enforces capability grants.
- The contract harness runs standalone in pure TS (no Rust in an adapter author's loop).
- The codegen staleness check is still green after Phase 1 changes.
- `deno_core`'s maintenance burden is *measurable* at this point, not speculative.

**Decision:** if the boundary is consuming more attention than the engine's guarantees are saving, collapsing to all-TypeScript is still cheap here — the canonical model, the adapter interface, and the conceptual architecture are language-agnostic and survive the move (§0.2, §21 risk row 1). **Deferring this decision past Phase 1 is what makes it expensive.** Document the decision as the first ADR in `docs/adr/` (the proposal's closing line expects ADRs to supersede specific choices).

---

## Phase 2 — API + second adapter (weeks 11–16) — IN PROGRESS

Goal: the hosted API, a structurally harder second adapter (Roll20), the Foundry relay, and SF2e parity.

### 2.1 Axum API — CRUD, connections, sync, webhooks, auth ✅ COMPLETE

**Goal:** the REST surface from §13, OpenAPI generated from the same types via `utoipa` (§13, §14).

**Deliverables:**
- `crates/api`: character CRUD with `PATCH` optimistic locking via `expectedContentHash` → `409` on mismatch (§13.2, §18); connections + `pull`/`push`/`diff`/`drift` (§13.3); history + rollback (§13.4); HMAC-signed webhooks (§13.5); plugin registration (§13.6).
- Auth: OAuth 2.0 code+PKCE for the web app; scoped, revocable API keys (`nsk_live_…`) for CLI/relay/collector (§13.1). One tower middleware layer for the party-role model (`gm`/`player`, §13.1).
- `drift` returns `Unavailable { reason: SandboxNotRunning }` for Roll20 — honest unknown, not stale "no changes" (§6.2, §13.3).
- Rate limiting: per-key token bucket, generous on `drift`/`session`, strict on `push`/`pull` (§17, §18).

**Depends on:** 1.4 (CLI shares the API's store and sync flow), 0.6.
**Verify:** `testcontainers` integration test exercising the full conflict flow end to end: create → divergent platform edit → `pull` returns `409` with `mergePreview` → re-request with resolutions → verify the DAG (§16, §13.3).
**Status:** ✅ Complete. API key auth middleware (`crates/api/src/auth.rs`): `nsk_live_…` Bearer tokens, Argon2id-hashed at rest (PHC format), TTL-cached validation (~50ms Argon2 verify amortised), three scopes (`read_only`/`sync`/`admin`) enforced per-method (403 on scope mismatch, 401 on missing/invalid/malformed key). Per-key token-bucket rate limiting (read 100/min, write 30/min, 429 + `Retry-After`), in-memory for Phase 2 (Redis for multi-instance hosted). Layer order: auth → rate_limit on `api_routes` only; the static web-UI fallback sits outside auth. `crates/api/tests/auth_flow.rs` (16 router-level tests) asserts every rejection path: missing/invalid/malformed key → 401, scope violations → 403, revocation → 401, bootstrap (first key when the store is empty), and rate-limit exhaustion (read vs write bucket, per-key independence). `crates/api/tests/postgres_integration.rs` runs a full API round-trip (create → patch → pull → history) against a real Postgres via testcontainers, skipping gracefully when Docker is absent. The full conflict flow test (`crates/api/tests/conflict_flow.rs`) exercises create → PATCH → pull (auto-merge) → PATCH → pull (409 conflict) → resolve → verify 5-snapshot DAG, plus an idempotent replay test. OpenAPI spec served at `/v1/openapi.json`.

### 2.2 Roll20 adapter — collector, handshake, chunked apply ✅ COMPLETE

**Goal:** the structurally harder second adapter, proving the trait is sufficient against a very different platform (§6, §21 scope-creep row).

**Deliverables:**
- `adapters/roll20/collector/nexus-collector.js`: ~150 lines, logic-free, unbuilt/unminified/auditable (§6.1, §6.5). Reads Character/Attribute objects verbatim; on `!nexus-pull` POSTs a signed dump; on `!nexus-apply` writes attrs verbatim.
- `adapters/roll20/src`: declarative `ROLL20_PF2E_MAP` mapping table (§6.3); row matching by content fingerprint, not row id (§6.4); `sheet_version` check returning `UnsupportedSheetVersion` (§6.4).
- Signed-payload handshake: per-campaign install token + short-lived signed JWT via `!nexus-auth` (§6.5).
- Chunked `!nexus-apply` across sandbox ticks with server-tracked acknowledgements (§6.4).
- Sandbox-availability handling: collector re-announces on sandbox start (§6.2).

**Depends on:** 2.1 (the `drift`/`pull`/`push` endpoints it calls), 1.2 (SDK + harness), 0.7 (fixtures).
**Verify:** contract harness passes; the collector script is auditably short; `drift` on an asleep sandbox returns `Unavailable` (§6.2).
**Status:** ✅ Complete. 48 tests (contract harness against 3 PF2e fixtures + Roll20-specific tests covering sheet_version validation/throws, lossy ability score round-trip, repeating-row fingerprint matching, detectChanges, capabilities, and round-trip fidelity). The mapping table uses `canonicalPath()` for compile-time path checking and derives `fieldCoverage` from the table itself (§6.3). Ability scores are lossy: Roll20 stores the derived number only; the value round-trips but boosts/flaws are lost (§5.2, warned on both export and import). `capabilities.supportsSessionRead` is false (§4, §6.2). The `drift` endpoint already returns `Unavailable { reason: SandboxNotRunning }` for Roll20 connections (§6.2, wired in `crates/api/src/routes/connections.rs`).
**Risk:** Roll20's sandbox is unreachable and frequently asleep (§6.2, §21). Document the asymmetry prominently; never build a scraping workaround to fake parity (§21).

### 2.3 Foundry relay module — ✅ COMPLETE

**Goal:** the in-browser module sharing `foundry-mapping` by plain import (§7.1).

**Deliverables:**
- `adapters/foundry/module/`: `nexus-sheet-relay` + `module.json`; registers authenticated local HTTP endpoints keyed by a generated API key (§7, §7.1). Four endpoints: `GET /nexus/actors` (list), `GET /nexus/actors/:id` (import), `POST /nexus/actors/:id/apply` (export), `GET /nexus/drift` (drift report).
- Hooks `Actor` create/update for drift telemetry — still never auto-syncing (§4, §7.1). DriftTracker is pure logic, tested with mocks.
- Imports `foundry-mapping` directly to convert Actors in-browser, works fully offline (§7.1). The round-trip test asserts the relay uses the same mapping as the server-side adapter.
- Direct world access (LevelDB) is a **CLI concern** (§7.4), not the relay module — the relay is for running worlds only.

**Depends on:** `packages/foundry-mapping` (from 1.3), 2.1.
**Verify:** 50 relay tests pass (drift tracker, API key validation, endpoint handlers, auth enforcement, hook wiring, round-trip through foundry-mapping). Bundled by esbuild to a single ESM file.
**Note:** Direct LevelDB access (§7.4) remains a CLI-side task; the relay module itself is complete.

### 2.4 SF2e system plugin + Foundry SF2e coverage — ✅ COMPLETE

**Goal:** SF2e at parity for the fields SF2e uses (§3.4, §19).

**Deliverables:**
- `crates/schema-sf2e`: `SystemPlugin` extending `schema-core`; Resolve Points in `session` (§3.4, §4); drones with chassis/mod slots; credits/UPB as optional fields on one `Currencies` struct (§3.4); lightweight vehicles/starship roles.
- Foundry SF2e coverage in `foundry-mapping` (drones, Resolve Points, credits).

**Depends on:** 0.2 (base types), 1.3 (Foundry mapping extension point), 0.7 (SF2e fixture).
**Verify:** the SF2e fixture validates; Foundry SF2e Actor round-trips within `fieldCoverage`.
**Status:** ✅ Complete. The `Sf2ePlugin` (`crates/schema-sf2e`) — `id`/`json_schema`/`validate`/`derived_stats`/`content_index`, with an SF2e content index; the shared PF2e/SF2e validate + derived-stat logic lives in `schema_core::chassis` so both plugins share one implementation (8 tests). Foundry SF2e coverage in `packages/foundry-mapping`: drones (chassis + mods) and vehicles (role) map to/from Foundry items (`"drone"`/`"vehicle"` item types), added to `fieldCoverage` and the contract harness against the SF2e mechanic fixture (10 new Foundry adapter tests, 39 total). Credits/UPB were already mapped in `mapCurrencies`/`importCurrencies`. Resolve Points are session state (§3.4, §4) and are never written — like current HP, they live in `session`, not `build`.

### Phase 2 gate

- The §19 done-criteria flow works on the hosted API (not just the CLI): build in Foundry → push to canonical → pull into a second Foundry world or export to Roll20 → divergent edits → resolve a real conflict end to end.
- SF2e at parity for SF2e-used fields.
- Webhooks deliver signed payloads; rate limiting is in place.

---

## Phase 3 — Conflict UX + hardening (weeks 17–21)

Goal: a usable conflict-resolution UI, the migration framework exercised for real, security/scale hardening, and the adapter on-ramp that the whole architecture exists to enable.

### 3.1 Web UI — history + conflict resolution + WASM diff preview

**Goal:** the conflict UX, with `sync-engine` compiled to WASM driving the diff preview so the UI shows exactly what the server would compute (§19 Phase 3, §14 `wasm-bindgen` narrow use).

**Deliverables:**
- `sync-engine` → `wasm-bindgen` build target (the only WASM use in the project — §14, §0.1).
- Minimal web UI: snapshot DAG timeline; per-field conflict resolution against a `MergePreview`; diff preview computed client-side by the WASM engine.

**Depends on:** 0.5 (sync-engine, pure so it compiles to WASM unchanged — §1), 2.1 (API).
**Verify:** the WASM diff preview matches the server's `diff` output on the same inputs (a parity check — the narrow WASM use still needs one).

### 3.2 Exercise the migration framework — one intentional v1→v2

**Goal:** prove the lazy, chained migration path works for real before self-hosters hit a multi-version jump (§9.3, §21 risk row).

**Deliverables:**
- One intentional `v1→v2` `Migration` (§9.3) operating on `serde_json::Value`.
- Migration test matrix (§16): every migration runs against every prior-version fixture; output deserializes into the current schema and preserves known values.
- Lazy migration on read + optional compaction pass (§9.3).

**Depends on:** 0.6 (store), 0.7 (fixtures at multiple versions — add a v1 fixture if not present).
**Verify:** a v1 character read by the v2 binary migrates through automatically with no manual stepping.

### 3.3 Security pass, rate limiting, Docker, self-hosting docs

**Goal:** the §17 posture enforced and the §12 self-hosting story delivered.

**Deliverables:**
- Security review against §17: API key scoping/hashing, OAuth scope, `secrecy` wrappers, adapter capability enforcement (already in 1.1 — re-audit), the pasted-script problem (§6.5).
- Docker: multi-stage distroless image of the static binary (§14).
- `docker-compose.yml` for self-hosters; self-hosting docs covering the offline SQLite path that never contacts hosted infrastructure (§12).
- Canary job (§16, §21): scheduled import of a known test character from each live third-party platform, alerting on upstream format changes.

**Depends on:** 2.1 (API to harden), 0.6.
**Verify:** `cargo audit`/`cargo-deny`/`pnpm audit` clean; the Docker image runs the full stack on a small VPS; the canary job runs green against live platforms.

### 3.4 Adapter-authoring guide + `example-adapter` template

**Goal:** the on-ramp the entire architecture is arranged around (§19 Phase 3 — a deliverable, not docs cleanup).

**Deliverables:**
- `docs/adapter-authoring-guide.md`: the path from `npm install` to a merged adapter PR, with the "no Rust required" loop front and center (§0.1, §15).
- `examples/example-adapter/`: a clone-rename-implement-ship template that passes the contract harness out of the box.

**Depends on:** 1.2 (SDK + harness), 1.3 (Foundry as the reference the guide documents).
**Verify:** someone outside the core team can write a working adapter from the guide alone — this is part of the §19 done criteria, not a stretch goal.

### Phase 3 gate — MVP done ✅

The §19 done criteria, all met:
- **Foundry→canonical→Foundry divergent merge ✅** — `crates/api/tests/e2e_gate.rs` exercises the full cycle with the real PF2e fixture through the V8 adapter sandbox: create → push (adapter.export) → pull (adapter.import, sets last_sync) → PATCH identity.name (world A) → modify actor.name (world B) → pull → 409 conflict → resolve (theirs) → push merged → verify resolved name in exported actor + snapshot DAG.
- **Adapter-authoring guide ✅** — `docs/adapter-authoring-guide.md` + `examples/example-adapter/` (3.4). Someone outside the core team can write a working adapter from the guide alone.

Explicitly **not** in the MVP (§19, §20): third-party builder adapters (Pathbuilder/Wanderer's Guide/Demiplane — left as the first community contributions), TS system plugins, the hosted multi-tenant service.

---

## Cross-cutting concerns (woven through every phase)

- **Testing strategy (§16).** Each task's *Verify* block names the §16 layer it exercises. The proptest suite is part of 0.5, not a later hardening pass. Contract tests are part of 1.2, not added after adapters exist. The codegen staleness check is part of 0.4. Fixtures (0.7) are the single ground-truth set shared by both languages.
- **Supply chain (§14, §17).** `cargo-deny`/`cargo-audit` + `pnpm audit` in CI from 0.1. The TS side matters more than usual — adapter transitive deps run in-process (sandboxed, but part of the trust surface).
- **Observability.** Tower tracing middleware in the API (2.1); boundary validation errors are precise so cross-boundary debugging is survivable (§14.1).
- **Documentation.** ADRs in `docs/adr/` supersede specific architecture choices as the project evolves (the proposal's closing line). The Phase 1 go/no-go decision is the first ADR.

---

## Risk register (mapped to phases, per §21)

| Risk (§21) | Lands in | Mitigation in this plan |
|---|---|---|
| Two-language split costs more than it returns | Phase 1 gate | Explicit go/no-go with a documented ADR; collapse is still cheap at that point (§0.2). |
| `deno_core` heavy, V8-tied, on critical path | 1.1 | Tiny op surface (fetch, log only); `"runtime": "external"` fallback exists as a real escape hatch. |
| Generated schema packages get hand-edited | 0.4 | Header comments + `package.json` markers + the CI staleness job (the actual control). |
| Contributors need both toolchains for full tests | 1.2 | Contract harness runs standalone in pure TS; verified explicitly. |
| Roll20 sandbox unreachable/asleep | 2.2 | Document asymmetry; `drift` returns `Unavailable`; no scraping workaround. |
| Direct LevelDB access corrupts a world | 2.3 | Never raw LevelDB; shutdown-gated; route through Foundry's pack/unpack CLI; backup first. |
| Canonical schema becomes a bottleneck | 0.2/0.3 | `extensions` escape hatch (§3.5, §9.4) means adapters are never blocked. |
| Adapter authors couple engine to platform | 0.1 | Cargo dep-isolation CI check on `sync-engine` from day one. |
| Merge conflicts overwhelm with noise | 0.5 | Identity-aware diffing; `session` structurally excluded; proptest asserts identical inputs never conflict. |
| Third-party builders change export formats | 3.3 | Pinned-fixture contract tests + scheduled canary job. |
| Self-hosters skip upgrades, hit a painful jump | 3.2 | Chained, lazy migrations; migration test matrix. |
| Homebrew too strict or too loose | 1.2 | Error/warning split; contract test asserts unknown content warns, never throws. |
| Malicious/careless adapter reaches something | 1.1 | Granted capabilities (no fs/env/db, allowlisted fetch); boundary tests assert enforcement; timeouts contain panics/hangs. |
| Scope creep across too many platforms | Phase 2 | Sequence Foundry → Roll20 → community; two structurally different platforms before asking for more. |

---

*This plan is a decomposition of `cross-vtt-sync-architecture.md` §19, not a new architecture. Where it adds sequencing detail the proposal leaves implicit, the intent is to make the §19 phases executable; any deviation from the proposal's invariants is a bug in this plan, not a license to break them. Expect ADRs in `docs/adr/` to supersede specific choices here as implementation reveals them — the same versioning philosophy §9.3 applies to data, applied to the plan itself.*
