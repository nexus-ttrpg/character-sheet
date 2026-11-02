# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this workspace is

**Nexus Character Sheet** — a cross-VTT (Virtual Tabletop) character synchronization platform for PF2e / SF2e. "Git for TTRPG characters": a canonical character model and a diff/merge engine sitting between platform adapters. Adapters are disposable; the canonical model and the merge engine are the asset.

**Status:** Phase 3 complete (MVP gate met). Phases 0 (foundations), 1 (adapter boundary, GO decision in ADR 0001), 2 (Axum API, Roll20, Foundry relay, SF2e), and 3 (conflict UX, WASM diff preview, migration framework, hardening, adapter-authoring guide) are complete. Post-MVP work in progress: the Pathbuilder adapter (first third-party-builder, import-only — §8) and the `packages/web` conflict UI. 432 tests passing (210 Rust + 222 TS; CI runs 217 TS — the 6 `packages/web` WASM-parity tests need `pnpm build:wasm` first and skip otherwise, a placeholder running in their place, §3.1). See `implementation-plan.md` for the phased build plan and `docs/adr/0001-*.md` for the Phase 1 go/no-go decision.

The architecture proposal lives in `cross-vtt-sync-architecture.md` (~1090 lines) and remains the canonical design reference. ADRs in `docs/adr/` supersede specific choices as the project evolves (the proposal's closing line).

## Load-bearing invariants (do not break these when editing)

Every section answers the guiding principle from §0: *"Does this make it easier to add the next adapter, without touching the engine?"* Adapters are disposable; the canonical model and diff/merge engine are the asset. The split is pitched as "Git for TTRPG characters," not "a Roll20–Foundry sync tool."

- **Language boundary (§0.1):** Rust core (sync-engine, canonical schema, store, API, CLI) + TypeScript adapters (adapter SDK + all adapters). **A contributor writing an adapter never installs Rust** — their loop is `npm install`, `npm test`, open a PR. This is the single most important property of the design; §5, §10, §15 all defer to it.
- **`sync-engine` is pure and I/O-free (§1, §11.3):** owned data in, owned data out — no DB handle, no HTTP, no `async`. Enforced structurally: it's in a different language behind an op interface, so no adapter import can reach it. `merge()` is a pure function of three `CharacterBuild`s.
- **One-way codegen (§3.6):** Rust types (in `crates/schema-core`) are the only hand-written source → `schemars` → `canonical.schema.json` → generated `@nexus-ttrpg/schema-types` and `@nexus-ttrpg/schema-zod`. Generated output is **committed** so TS contributors need no Rust toolchain; CI regenerates and fails on diff. Never generate Rust from JSON Schema — generation runs toward the less-expressive target because Rust needs newtypes with invariants, `BTreeMap` ordering, and exhaustive enums that JSON Schema can't express.
- **Hub-and-spoke, not mesh (§1):** every platform talks only to the canonical model. O(n) adapters, not O(n²).
- **Session state never auto-syncs (§4):** HP, conditions, spell slots, initiative are scene-scoped, not character-scoped. Enforced *by types* (`SessionTelemetry` has no conversion into `CharacterBuild`), not by convention. Exposed as read-only telemetry only.
- **Conflicts are never auto-resolved (§11.3):** three-way merge returns conflicts for a human to resolve per field before the merge snapshot commits.
- **Rollback is `revert`, not `reset --hard` (§11.4):** history is an immutable snapshot DAG; rollback records a new snapshot, never destructively rewrites.
- **Adapters are untrusted (§1, §5.5):** everything crossing the boundary upward is validated against the canonical schema in Rust before reaching the engine. The TS type checker is a convenience for the adapter author, not a guarantee to the host.
- **Unknown/homebrew content is always a warning, never an error (§5.2):** an adapter that throws on unrecognized content is unusable in real Foundry worlds. Contract tests assert this.
- **Sync ops are idempotent and content-addressed (§12):** keyed by `content_hash` + parent snapshot; replay after a flaky reconnect is a no-op.

## Tech stack (§14)

Rust core: Axum, SQLx (PostgreSQL hosted + SQLite offline CLI, one codebase), serde/schemars/garde, utoipa (OpenAPI), `deno_core` (embeds V8 to run TS adapters in-process, no Node on server), BLAKE3 content hashing, `cargo-deny`/`cargo-audit`. TS side: TypeScript + pnpm + Vitest, adapter/web bundles via esbuild. Narrow `wasm-bindgen` use only for the browser-side diff preview (Phase 3). Two workspaces side by side (Cargo + pnpm); Turborepo deliberately not used.

## Repo layout (§15)

```
crates/
  schema-core/     SOURCE OF TRUTH — canonical Rust types, BLAKE3 content hashing, shared chassis (validate/derived)
  schema-pf2e/     PF2e SystemPlugin (extends schema-core; delegates to shared chassis)
  schema-sf2e/     SF2e SystemPlugin — drones, Resolve Points, credits/UPB (§3.4; delegates to shared chassis)
  sync-engine/     Pure diff/merge/DAG (no I/O, no adapter deps — Cargo-enforced)
  sync-engine-wasm/ cdylib wrapper over sync-engine — the ONLY wasm-bindgen use (§14); powers the web diff preview
  store/           SQLx Postgres + SQLite, AES-256-GCM credential encryption
  adapter-host/    deno_core V8 sandbox, boundary validation, capability enforcement
  api/             Axum REST API, utoipa OpenAPI, webhooks (Phase 2.1 — COMPLETE)
packages/
  schema-types/    GENERATED — do not hand-edit
  schema-zod/      GENERATED — do not hand-edit
  adapter-sdk/     CharacterAdapter interface, contract test harness
  foundry-mapping/ PF2e Actor ↔ canonical mapping logic
  web/             Conflict UI — History / Diff / Merge Lab views, client-side WASM diff (Phase 3.1)
adapters/
  foundry/         Reference adapter (39 adapter + 78 relay module tests, esbuild bundle)
  roll20/          Second adapter — flat attrs, repeating sections, asleep sandbox (48 tests)
  pathbuilder/     First third-party-builder adapter — import-only (§8), non-lossy ability-score components (20 tests)
cli/               `nexus` — Rust, clap, single static binary (import/push/diff/history)
xtask/             cargo xtask codegen | openapi | fixtures
examples/          fixtures/ (shared JSON ground-truth), example-adapter/ (Phase 3)
docs/              ADRs, adapter-authoring guide, self-hosting
```

The one boundary that matters: `sync-engine` depends on `schema-core` and nothing else — enforced by Cargo, so violating it is a compile error. A contributor adding an adapter works entirely inside `adapters/` and `packages/`, runs `pnpm test`, and never opens `crates/`.

## Commands (§16)

### Rust

```
cargo test                              # all crates (210 tests)
cargo test -p sync-engine               # merge engine + proptest laws
cargo test -p api                       # API unit + integration tests (43)
cargo test -p adapter-host              # 31 tests: 17 boundary + 14 unit
cargo test -p sync-engine-wasm          # WASM wrapper + native-parity checks (7)
cargo clippy --workspace --tests        # lint
cargo fmt --check                       # format check
cargo xtask codegen && git diff --exit-code  # codegen staleness check (§3.6)
cargo deny check                        # supply chain (licenses, advisories, bans)
```

### TypeScript

```
pnpm install                            # install all workspace deps
pnpm test                               # all packages + adapters (222 w/ WASM built, 217 without)
pnpm test --filter @nexus-ttrpg/foundry-adapter      # Foundry adapter + relay module (117 tests)
pnpm test --filter @nexus-ttrpg/roll20-adapter       # Roll20 adapter (48 tests)
pnpm test --filter @nexus-ttrpg/pathbuilder-adapter  # Pathbuilder adapter (20 tests)
pnpm test --filter @nexus-ttrpg/adapter-sdk          # SDK + contract harness (19 tests)
```

In an adapter directory: `pnpm test` — plain Vitest, no host, no DB, no network (adapters are stateless, §5.4). If a contributor ever needs `cargo` to iterate on an adapter, that's a bug in the SDK.

### CLI

```
cargo run -p nexus-cli -- import --foundry <world-json>  # import → snapshot → HEAD
cargo run -p nexus-cli -- push --foundry                 # export HEAD → Foundry payload
cargo run -p nexus-cli -- diff                            # field-level diff
cargo run -p nexus-cli -- history                         # snapshot DAG
```

## Before saying a change is ready to commit

**Run every CI check locally, exactly as `.github/workflows/` runs it, and never
tell the owner they can commit until all of them pass.** The full command list,
with prerequisites, is in `AGENTS.md` under "Before saying a change is ready to
commit". In short: `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace` with Docker running, the
sync-engine boundary check, `pnpm install --frozen-lockfile`, `cargo xtask
codegen`, `pnpm test`, the Foundry bundle plus `cargo test -p api --test
e2e_gate`, `cargo deny check` **and** `cargo audit`, and `pnpm audit`.

Report each result. If a check could not run, say so instead of calling the
change green. CI is a fresh Linux build, so a local pass is not proof: after a
push, check the Actions runs.

## CI & tooling notes

- **No Node 20.** GitHub deprecated the Node 20 action runtime (Sept 2025);
  actions targeting `node20` are force-run on Node 24 and will eventually fail.
  The app runtime is pinned to Node 22 (`.nvmrc` + `package.json` `engines`
  `>=22`) — never lower it to 20, and never hardcode `node-version: 20` in a
  workflow (use `node-version-file: '.nvmrc'`, as the existing workflows do).
  All workflow actions are on node24 runtimes: `actions/checkout@v5`,
  `actions/setup-node@v5`, `pnpm/action-setup@v6` (v6 was the fix for the old
  `@v4` node20 straggler). `pnpm/action-setup` takes no `version:` input — it
  reads `packageManager` from `package.json` (`pnpm@11.21.0`); passing `version:`
  alongside that field makes it error.
- **Rust toolchain in CI is pinned to `1.97`.** Every job uses
  `dtolnay/rust-toolchain@1.97` (not `@stable`) so it installs 1.97.1 directly
  instead of installing a baseline stable that `rust-toolchain.toml` then makes
  cargo re-sync away from. `cargo-deny-action` gets the same via its
  `rust-version: "1.97.1"` input.
- **`cargo audit` vs `cargo-deny`.** Both run in `.github/workflows/audit.yml`
  (§17) and they differ: cargo-audit scans the whole `Cargo.lock` regardless of
  enabled features (so resolved-but-uncompiled crates still flag), cargo-deny
  enforces `deny.toml`. Acknowledged-but-unfixable advisories are silenced in
  `.cargo/audit.toml` and `deny.toml` respectively — keep both in sync when
  silencing one. **cargo-audit only reads `.cargo/audit.toml`** (or
  `~/.cargo/audit.toml`); a root-level `audit.toml` is silently ignored, which
  is why the `cargo-audit` job failed until 2026-11-02. Example:
  `rsa`/RUSTSEC-2023-0071 is an uncompiled transitive of the unused
  `sqlx-mysql` driver with no upstream fix — silenced in `.cargo/audit.toml`
  (and not in `deny.toml` only because cargo-deny already passes via its own
  ignore list). Uncompiled crates such as `quinn-proto` are seen only by
  cargo-audit, so a green cargo-deny says nothing about cargo-audit.

## Implementation status

### Phase 0 — Foundations ✅ COMPLETE

- `schema-core`: canonical types, newtypes, `BTreeMap` determinism, `deny_unknown_fields`, BLAKE3 content hashing, `schemars` JsonSchema.
- `schema-pf2e`: SystemPlugin with PF2e-specific structs.
- `sync-engine`: `diff`, three-way `merge` → `MergePreview`, `Snapshot` DAG, rollback-as-revert, proptest suite (merge identity, `merge(a,x,x)` never conflicts, single-sided changes never lost, hash stability).
- `store`: SQLx Postgres + SQLite, one codebase. ULID cursor pagination, `secrecy::Secret` wrappers, AES-256-GCM credential encryption.
- `xtask codegen`: one-way pipeline (Rust → `canonical.schema.json` → `@nexus-ttrpg/schema-types` + `@nexus-ttrpg/schema-zod`), committed output, CI staleness check.
- `examples/fixtures/`: 4 PF2e + 1 SF2e ground-truth characters.

### Phase 1 — Adapter boundary ✅ COMPLETE (GO — ADR 0001)

- `adapter-host`: `deno_core` V8 sandbox, tiny op surface (`nexus_fetch` + `nexus_log`), boundary validation (`deny_unknown_fields`), capability enforcement (allowlisted fetch only), timeout/panic containment, `"runtime": "external"` escape hatch.
- `adapter-sdk`: `CharacterAdapter<TPlatform>` interface, `runContractTests(adapter, fixtures)` one-line harness, `canonicalPath()` typed helper.
- `adapters/foundry`: reference adapter — identity, ancestry, class, level, abilities, skills, feats, equipment, spellcasting. 39 adapter tests, esbuild bundle. `packages/foundry-mapping` shared mapping logic.
- `cli/`: `nexus import|push|diff|history` — SQLite local store, 6 integration tests.

**Phase 1 gate: GO.** The cross-language maintenance cost is measurable and bounded (~25 lines of V8 scope/promise code, `LocalSet` at call sites, esbuild bundle step). The sync-engine's purity, property-testability, and structural boundary enforcement earn their keep. See `docs/adr/0001-phase1-go-no-go-rust-ts-split.md`.

### Phase 2 — API + second adapter ✅ COMPLETE

- **2.1 Axum API ✅ COMPLETE** — `crates/api`: character CRUD with PATCH optimistic locking (`expectedContentHash` → 409), connections (incl. `DELETE`) + `pull`/`push`/`diff`/`drift`, history + rollback (revert semantics), HMAC-signed webhooks, utoipa OpenAPI at `/v1/openapi.json`. API key auth middleware (`crates/api/src/auth.rs`): `nsk_live_…` Bearer tokens, Argon2id-hashed at rest (PHC format), TTL-cached validation, three scopes (`read_only`/`sync`/`admin`) enforced per-method (401 on missing/invalid/malformed key, 403 on scope mismatch; `DELETE` requires Admin). Per-key token-bucket rate limiting (read 100/min, write 30/min, 429 + `Retry-After`), in-memory for Phase 2 (Redis deferred to the multi-instance hosted stretch goal, §20). `drift` returns `Unavailable { reason: SandboxNotRunning }` for Roll20 (§6.2). 43 tests (19 unit + 24 integration: `auth_flow` 16, `conflict_flow` 2 incl. full create → divergent edit → pull 409+mergePreview → resolve → verify DAG, `connections` 3, `e2e_gate`/`e2e_sf2e`/`postgres_integration` 1 each — Postgres via testcontainers, skips gracefully without Docker).
- **2.2 Roll20 adapter ✅ COMPLETE** — `adapters/roll20`: the structurally harder second adapter, proving the `CharacterAdapter` trait generalizes beyond Foundry. Declarative `ROLL20_PF2E_MAP` mapping table (§6.3) with `canonicalPath()` compile-time path checking and derived `fieldCoverage`. Logic-free collector script (`collector/nexus-collector.js`, ~150 lines, §6.5) — reads attrs verbatim, POSTs signed, writes verbatim, re-announces on sandbox start. Repeating rows matched by content fingerprint, not row id (§6.4). Lossy ability scores (value round-trips, components don't — §5.2). `validate()` checks `sheet_version` and throws `UnsupportedSheetVersion` (§6.4). 48 tests (contract harness + Roll20-specific: sheet_version validation, fingerprint matching, lossy ability scores, detectChanges, round-trip).
- **2.3 Foundry relay module ✅ COMPLETE** — `adapters/foundry/module/`: in-browser Foundry VTT module that shares `foundry-mapping` by plain import (§7.1 — "it's an import statement, not a WASM bridge"). Registers authenticated local HTTP endpoints (`GET /nexus/actors`, `GET /nexus/actors/:id`, `POST /nexus/actors/:id/apply`, `GET /nexus/drift`) using a per-world `nsk_relay_` API key (constant-time validated). Hooks `createActor`/`updateActor` for drift telemetry — never auto-syncs (§4), just marks actors dirty. `applyActor` writes max HP only (§7.2); current HP is session state. The pure logic (drift tracker, endpoint handlers, API key validation) is tested with mock Foundry API implementations — no real Foundry needed. 78 relay tests — `relay.test.ts` 50, `node-http-server.test.ts` 21, `full-loop.test.ts` 7 (117 foundry total: 39 adapter + 78 relay). Bundled by esbuild to a single self-contained ESM file (`module/nexus-character-sheet.js`) loaded via `module.json`.
- **2.4 SF2e system plugin + Foundry SF2e coverage ✅ COMPLETE** — `crates/schema-sf2e`: `Sf2ePlugin` implementing `SystemPlugin` (id `sf2e`, validate, derived_stats, content_index). The SF2e struct shapes (drones, `resolve_points` in session, credits/UPB, vehicles) already lived in `schema-core` as shared chassis (§3.4); this plugin adds the SF2e content index and delegates validation/derived-stats to the shared chassis. 14 tests (12 unit + 2 fixture). Foundry SF2e coverage in `packages/foundry-mapping`: drones (chassis + mods) and vehicles (role) now map to/from Foundry items (`"drone"`/`"vehicle"` item types), round-tripped through the contract harness against the SF2e mechanic fixture. Credits/UPB were already mapped; Resolve Points are session state (§4) and never written (like current HP). 10 new Foundry adapter tests (39 total). The shared PF2e/SF2e chassis logic (ability scores, HP, perception/saves/class DC, cross-field validation) was extracted into `schema-core::chassis` so both plugins share one implementation rather than drifting.

### Phase 3 — Conflict UX + hardening ✅ COMPLETE

- **3.1 Web UI + WASM diff preview ✅** — `crates/sync-engine-wasm`: cdylib wrapper over `sync-engine` (the only `wasm-bindgen` use — §14), exports `diff`/`merge`/`apply_merge`/`content_hash` as JSON-string functions. Parity confirmed by `parity-test.cjs` (16 diff pairs + 3 merges + 4 apply_merges + 4 content hashes against native output). `packages/web/`: minimal web UI with snapshot DAG timeline, per-field conflict resolution against a `MergePreview`, and diff preview computed client-side by the WASM engine (§19 Phase 3). Three views: History (DAG timeline), Diff (server↔WASM parity check made visible), Merge Lab (client-side 3-way merge + per-field ours/theirs/custom resolution + `apply_merge` → create character). Served by the API via `ServeDir` fallback outside auth (`NEXUS_WEB_DIR` env).
- **3.2 Migration framework ✅** — One intentional v0.0.1→v0.1.0 migration (`RenameCharacterName`: `identity.characterName` → `identity.name`), v0.0.1 fixtures in `examples/fixtures/v0.0.1/`, migration test matrix (`migration_matrix.rs`: every migration against every prior-version fixture, lossless), lazy migration on read wired into `store/src/lib.rs:294` (`decode_build_from_value` → `MigrationRegistry::migrate_to_current`), proven by `store/tests/migration_on_read.rs`.
- **3.3 Security/Docker/self-hosting ✅** — Argon2id API key hashing (§17), per-key rate limiting (read 100/min, write 30/min), TTL-cached key validation. `Dockerfile`: 3-stage multi-stage build (Node adapter bundles → Rust release → distroless non-root runtime). `docker-compose.yml`: postgres + api with secrets. `docs/self-hosting.md`: Docker+Postgres and offline SQLite paths. `cargo audit`/`cargo-deny`/`pnpm audit` in CI (`audit.yml`, `deny.toml`). Canary job in `.github/workflows/canary.yml`: three layers — contract tests, e2e gate test (full Foundry→canonical→Foundry divergent-merge cycle through the V8 sandbox), and conditional live-platform canary (`scripts/canary.sh`, gated on `CANARY_API_URL`/`CANARY_API_KEY` secrets).
- **3.4 Adapter-authoring guide + example-adapter ✅** — `docs/adapter-authoring-guide.md` (380 lines: quickstart, full `CharacterAdapter` interface, contract harness, V8 sandbox, invariants, PR checklist). `examples/example-adapter/`: clone-rename-implement-ship template passing the contract harness out of the box.

### Phase 3 gate — MVP done ✅

The §19 done criteria:
1. **Foundry→canonical→Foundry divergent merge ✅** — `crates/api/tests/e2e_gate.rs`: creates a character from the real PF2e fixture, pushes to Foundry via `adapter.export`, pulls back via `adapter.import` (sets `last_sync`), PATCHs `identity.name` (world A), modifies the Foundry actor's `name` (world B, same canonical field), pulls → 409 conflict, resolves (theirs), pushes merged build back to Foundry, verifies the actor carries the resolved name, verifies the snapshot DAG (≥3 snapshots, last is a merge).
2. **Adapter-authoring guide ✅** — `docs/adapter-authoring-guide.md` + `examples/example-adapter/` (3.4 above).

### Post-MVP — community-style adapters (in progress)

Third-party-builder adapters were explicitly out of MVP scope (§19, §20) — "left as the first community contributions." Started anyway as a real exercise of the adapter on-ramp:

- **Pathbuilder 2e adapter ✅ (with a caveat)** — `adapters/pathbuilder/`: the first third-party-*builder* adapter (§8). Import-only — Pathbuilder has no write-back API, so `capabilities.supportsExport` is `false` and the engine never offers a push; `export()` still renders re-importable JSON to satisfy the round-trip contract test. Non-lossy ability scores: because the canonical model stores components (boosts/flaws/itemBonus) and Pathbuilder recomputes the final number itself, `export → import` preserves components exactly — unlike the lossy Roll20 path. `manifest.json` scopes `fetch` to `https://pathbuilder2e.com` (§5.5). 20 tests (contract harness + non-lossy component round-trip). **Caveat:** the field names in `src/mapping.ts` are a representative shape modelled on the architecture doc, *not verified against a real Pathbuilder 2e export* — the contract tests prove import/export symmetry, not real-format fidelity. Needs a real export + remap before importing live data; the canary job (§3.3) is the drift-detection mechanism.

## MVP sequencing (§19)

Phase 0 foundations → Phase 1 adapter boundary (GO/NO-GO gate — **passed**) → Phase 2 Axum API + Roll20 + SF2e → Phase 3 conflict UX (sync-engine compiled to WASM for diff preview) + hardening + adapter-authoring guide.

Foundry is the reference adapter every later adapter is modeled on; Roll20 is second because its sandbox constraints (§6) make it structurally harder.
