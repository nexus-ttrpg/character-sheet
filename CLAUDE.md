# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this workspace is

**Nexus Sheet** — a cross-VTT (Virtual Tabletop) character synchronization platform for PF2e / SF2e. "Git for TTRPG characters": a canonical character model and a diff/merge engine sitting between platform adapters. Adapters are disposable; the canonical model and the merge engine are the asset.

**Status:** Phase 2 in progress. Phase 0 (foundations), Phase 1 (adapter boundary, GO decision in ADR 0001), Phase 2.1 (Axum API), and Phase 2.4 (SF2e system plugin) are complete. 180 tests passing (132 Rust + 48 TS). See `implementation-plan.md` for the phased build plan and `docs/adr/0001-*.md` for the Phase 1 go/no-go decision.

The architecture proposal lives in `cross-vtt-sync-architecture.md` (~1090 lines) and remains the canonical design reference. ADRs in `docs/adr/` supersede specific choices as the project evolves (the proposal's closing line).

## Load-bearing invariants (do not break these when editing)

Every section answers the guiding principle from §0: *"Does this make it easier to add the next adapter, without touching the engine?"* Adapters are disposable; the canonical model and diff/merge engine are the asset. The split is pitched as "Git for TTRPG characters," not "a Roll20–Foundry sync tool."

- **Language boundary (§0.1):** Rust core (sync-engine, canonical schema, store, API, CLI) + TypeScript adapters (adapter SDK + all adapters). **A contributor writing an adapter never installs Rust** — their loop is `npm install`, `npm test`, open a PR. This is the single most important property of the design; §5, §10, §15 all defer to it.
- **`sync-engine` is pure and I/O-free (§1, §11.3):** owned data in, owned data out — no DB handle, no HTTP, no `async`. Enforced structurally: it's in a different language behind an op interface, so no adapter import can reach it. `merge()` is a pure function of three `CharacterBuild`s.
- **One-way codegen (§3.6):** Rust types (in `crates/schema-core`) are the only hand-written source → `schemars` → `canonical.schema.json` → generated `@nexus-sheet/schema-types` and `@nexus-sheet/schema-zod`. Generated output is **committed** so TS contributors need no Rust toolchain; CI regenerates and fails on diff. Never generate Rust from JSON Schema — generation runs toward the less-expressive target because Rust needs newtypes with invariants, `BTreeMap` ordering, and exhaustive enums that JSON Schema can't express.
- **Hub-and-spoke, not mesh (§1):** every platform talks only to the canonical model. O(n) adapters, not O(n²).
- **Session state never auto-syncs (§4):** HP, conditions, spell slots, initiative are scene-scoped, not character-scoped. Enforced *by types* (`SessionTelemetry` has no conversion into `CharacterBuild`), not by convention. Exposed as read-only telemetry only.
- **Conflicts are never auto-resolved (§11.3):** three-way merge returns conflicts for a human to resolve per field before the merge snapshot commits.
- **Rollback is `revert`, not `reset --hard` (§11.4):** history is an immutable snapshot DAG; rollback records a new snapshot, never destructively rewrites.
- **Adapters are untrusted (§1, §5.5):** everything crossing the boundary upward is validated against the canonical schema in Rust before reaching the engine. The TS type checker is a convenience for the adapter author, not a guarantee to the host.
- **Unknown/homebrew content is always a warning, never an error (§5.2):** an adapter that throws on unrecognized content is unusable in real Foundry worlds. Contract tests assert this.
- **Sync ops are idempotent and content-addressed (§12):** keyed by `content_hash` + parent snapshot; replay after a flaky reconnect is a no-op.

## Tech stack (§14)

Rust core: Axum, SQLx (PostgreSQL hosted + SQLite offline CLI, one codebase), serde/schemars/garde, utoipa (OpenAPI), `deno_core` (embeds V8 to run TS adapters in-process, no Node on server), BLAKE3 content hashing, `cargo-deny`/`cargo-audit`. TS side: TypeScript + pnpm + Vitest + tsup. Narrow `wasm-bindgen` use only for the browser-side diff preview (Phase 3). Two workspaces side by side (Cargo + pnpm); Turborepo deliberately not used.

## Repo layout (§15)

```
crates/
  schema-core/     SOURCE OF TRUTH — canonical Rust types, BLAKE3 content hashing, shared chassis (validate/derived)
  schema-pf2e/     PF2e SystemPlugin (extends schema-core; delegates to shared chassis)
  schema-sf2e/     SF2e SystemPlugin — drones, Resolve Points, credits/UPB (§3.4; delegates to shared chassis)
  sync-engine/     Pure diff/merge/DAG (no I/O, no adapter deps — Cargo-enforced)
  store/           SQLx Postgres + SQLite, AES-256-GCM credential encryption
  adapter-host/    deno_core V8 sandbox, boundary validation, capability enforcement
  api/             Axum REST API, utoipa OpenAPI, webhooks (Phase 2.1 — COMPLETE)
packages/
  schema-types/    GENERATED — do not hand-edit
  schema-zod/      GENERATED — do not hand-edit
  adapter-sdk/     CharacterAdapter interface, contract test harness
  foundry-mapping/ PF2e Actor ↔ canonical mapping logic
adapters/
  foundry/         Reference adapter (29 tests, esbuild bundle)
cli/               `nexus` — Rust, clap, single static binary (import/push/diff/history)
xtask/             cargo xtask codegen | openapi | fixtures
examples/          fixtures/ (shared JSON ground-truth), example-adapter/ (Phase 3)
docs/              ADRs
```

The one boundary that matters: `sync-engine` depends on `schema-core` and nothing else — enforced by Cargo, so violating it is a compile error. A contributor adding an adapter works entirely inside `adapters/` and `packages/`, runs `pnpm test`, and never opens `crates/`.

## Commands (§16)

### Rust

```
cargo test                              # all crates
cargo test -p sync-engine               # merge engine + proptest laws
cargo test -p api                       # API unit + integration tests
cargo test -p adapter-host              # boundary tests (19) + unit tests (14)
cargo clippy --workspace --tests        # lint
cargo fmt --check                       # format check
cargo xtask codegen && git diff --exit-code  # codegen staleness check (§3.6)
cargo deny check                        # supply chain (licenses, advisories, bans)
```

### TypeScript

```
pnpm install                            # install all workspace deps
pnpm test                               # all packages + adapters
pnpm test --filter @nexus-sheet/foundry # Foundry adapter (29 tests)
pnpm test --filter @nexus-sheet/adapter-sdk  # SDK + contract harness (19 tests)
```

In an adapter directory: `pnpm test` — plain Vitest, no host, no DB, no network (adapters are stateless, §5.4). If a contributor ever needs `cargo` to iterate on an adapter, that's a bug in the SDK.

### CLI

```
cargo run -p nexus-cli -- import --foundry <world-json>  # import → snapshot → HEAD
cargo run -p nexus-cli -- push --foundry                 # export HEAD → Foundry payload
cargo run -p nexus-cli -- diff                            # field-level diff
cargo run -p nexus-cli -- history                         # snapshot DAG
```

## CI & tooling notes

- **No Node 20.** GitHub deprecated the Node 20 action runtime (Sept 2025);
  actions targeting `node20` are force-run on Node 24 and will eventually fail.
  The app runtime is pinned to Node 22 (`.nvmrc` + `package.json` `engines`
  `>=22`) — never lower it to 20, and never hardcode `node-version: 20` in a
  workflow (use `node-version-file: '.nvmrc'`, as the existing workflows do).
  Known straggler: `pnpm/action-setup@v4` still targets node20 internally (the
  latest major available) — GitHub force-runs it on Node 24 with a deprecation
  warning; bump to a node24 build / v5 when one ships.
- **`cargo audit` vs `cargo-deny`.** Both run in `.github/workflows/audit.yml`
  (§17) and they differ: cargo-audit scans the whole `Cargo.lock` regardless of
  enabled features (so resolved-but-uncompiled crates still flag), cargo-deny
  enforces `deny.toml`. Acknowledged-but-unfixable advisories are silenced in
  `audit.toml` and `deny.toml` respectively — keep both in sync when silencing
  one. Example: `rsa`/RUSTSEC-2023-0071 is an uncompiled transitive of the unused
  `sqlx-mysql` driver with no upstream fix — silenced in `audit.toml` (and not in
  `deny.toml` only because cargo-deny already passes via its own ignore list).

## Implementation status

### Phase 0 — Foundations ✅ COMPLETE

- `schema-core`: canonical types, newtypes, `BTreeMap` determinism, `deny_unknown_fields`, BLAKE3 content hashing, `schemars` JsonSchema.
- `schema-pf2e`: SystemPlugin with PF2e-specific structs.
- `sync-engine`: `diff`, three-way `merge` → `MergePreview`, `Snapshot` DAG, rollback-as-revert, proptest suite (merge identity, `merge(a,x,x)` never conflicts, single-sided changes never lost, hash stability).
- `store`: SQLx Postgres + SQLite, one codebase. ULID cursor pagination, `secrecy::Secret` wrappers, AES-256-GCM credential encryption.
- `xtask codegen`: one-way pipeline (Rust → `canonical.schema.json` → `@nexus-sheet/schema-types` + `@nexus-sheet/schema-zod`), committed output, CI staleness check.
- `examples/fixtures/`: 4 PF2e + 1 SF2e ground-truth characters.

### Phase 1 — Adapter boundary ✅ COMPLETE (GO — ADR 0001)

- `adapter-host`: `deno_core` V8 sandbox, tiny op surface (`nexus_fetch` + `nexus_log`), boundary validation (`deny_unknown_fields`), capability enforcement (allowlisted fetch only), timeout/panic containment, `"runtime": "external"` escape hatch.
- `adapter-sdk`: `CharacterAdapter<TPlatform>` interface, `runContractTests(adapter, fixtures)` one-line harness, `canonicalPath()` typed helper.
- `adapters/foundry`: reference adapter — identity, ancestry, class, level, abilities, skills, feats, equipment, spellcasting. 29 tests, esbuild bundle. `packages/foundry-mapping` shared mapping logic.
- `cli/`: `nexus import|push|diff|history` — SQLite local store, 6 integration tests.

**Phase 1 gate: GO.** The cross-language maintenance cost is measurable and bounded (~25 lines of V8 scope/promise code, `LocalSet` at call sites, esbuild bundle step). The sync-engine's purity, property-testability, and structural boundary enforcement earn their keep. See `docs/adr/0001-phase1-go-no-go-rust-ts-split.md`.

### Phase 2 — API + second adapter (IN PROGRESS)

- **2.1 Axum API ✅ COMPLETE** — `crates/api`: character CRUD with PATCH optimistic locking (`expectedContentHash` → 409), connections + `pull`/`push`/`diff`/`drift`, history + rollback (revert semantics), HMAC-signed webhooks, utoipa OpenAPI at `/v1/openapi.json`. 10 tests (8 unit + 2 integration including full conflict flow: create → divergent edit → pull 409+mergePreview → resolve → verify DAG). `drift` returns `Unavailable { reason: SandboxNotRunning }` for Roll20 (§6.2).
  - **Not yet done:** API key auth middleware (using hardcoded `default_owner()` for now), rate limiting, testcontainers integration test against Postgres.
- **2.2 Roll20 adapter** — not started. Collector script, declarative mapping table, signed-payload handshake, chunked apply.
- **2.3 Foundry relay module** — not started. In-browser module sharing `foundry-mapping`.
- **2.4 SF2e system plugin ✅ COMPLETE** — `crates/schema-sf2e`: `Sf2ePlugin` implementing `SystemPlugin` (id `sf2e`, validate, derived_stats, content_index). The SF2e struct shapes (drones, `resolve_points` in session, credits/UPB, vehicles) already lived in `schema-core` as shared chassis (§3.4); this plugin adds the SF2e content index and delegates validation/derived-stats to the shared chassis. 8 tests (6 unit + 2 fixture: the SF2e mechanic fixture now validates + produces sane derived stats). The shared PF2e/SF2e chassis logic (ability scores, HP, perception/saves/class DC, cross-field validation) was extracted into `schema-core::chassis` so both plugins share one implementation rather than drifting.

### Phase 3 — Conflict UX + hardening (NOT STARTED)

- WASM diff preview, web UI, migration framework exercise, security pass, Docker, adapter-authoring guide.

## MVP sequencing (§19)

Phase 0 foundations → Phase 1 adapter boundary (GO/NO-GO gate — **passed**) → Phase 2 Axum API + Roll20 + SF2e → Phase 3 conflict UX (sync-engine compiled to WASM for diff preview) + hardening + adapter-authoring guide.

Foundry is the reference adapter every later adapter is modeled on; Roll20 is second because its sandbox constraints (§6) make it structurally harder.
