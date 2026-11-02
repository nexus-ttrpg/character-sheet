# AGENTS.md

Guidance for AI agents (Claude Code, etc.) working in this repository.

## What this is

**Nexus Character Sheet** — a cross-VTT character synchronization platform for PF2e / SF2e.
"Git for TTRPG characters": a canonical character model + diff/merge engine
sitting between disposable platform adapters. Rust core, TypeScript adapters.

**Status:** Phase 3 complete (MVP gate met). Phases 0, 1, 2, and 3 are
complete. 432 tests passing (210 Rust + 222 TS; CI runs 217 TS without the WASM build). Post-MVP: the Pathbuilder
adapter (first third-party-builder, import-only) and the `packages/web` conflict
UI are underway. See `CLAUDE.md` for full
invariants and status, `implementation-plan.md` for the phased plan,
`cross-vtt-sync-architecture.md` for the canonical design (§N references
throughout the code).

## Commands to run

### Before saying a change is ready to commit — run every CI check, exactly as CI runs it

**Never tell the owner they can commit until every command below has passed
locally.** They mirror `.github/workflows/` one for one. Weaker variants
(`cargo clippy --tests` without `-D warnings`, cargo-deny without cargo-audit,
skipping the e2e gate) have already let CI failures through.

Prerequisites: Node 22 + pnpm 11, `cargo-deny`, `cargo-audit`, and Docker
running (otherwise `postgres_integration` skips silently).

```
# rust.yml — test + lint + boundary
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
! cargo tree -p sync-engine --prefix none | grep -qE '^(store|api|adapter-host) v'   # boundary: must exit 0

# rust.yml — codegen staleness, and ts.yml
pnpm install --frozen-lockfile
cargo xtask codegen        # then commit any change it makes to the generated files
pnpm test

# canary.yml — e2e gate
pnpm --filter @nexus-ttrpg/foundry-adapter bundle
cargo test -p api --test e2e_gate

# audit.yml — all three; cargo-audit and cargo-deny find different things
cargo deny check
cargo audit                # reads .cargo/audit.toml; scans the whole Cargo.lock
pnpm audit --audit-level high   # informational (continue-on-error in CI)
```

Report the result of each check. If one could not run (missing tool, Docker
down), **say so explicitly** instead of calling the change green. A local pass
is not proof: CI is a fresh Linux build on `ubuntu-latest`, so after a push,
check the Actions runs before saying CI is fixed.

`cargo xtask codegen` is the cross-language schema guarantee — if it changes
generated files, you changed Rust schema types without regenerating. Commit the
regenerated output, don't skip it.

### Scoped test runs

```
cargo test -p sync-engine          # merge engine + proptest laws
cargo test -p api                  # API unit + integration tests
cargo test -p adapter-host         # 31 tests: 17 boundary + 14 unit
cargo test -p store                # store tests
pnpm test --filter @nexus-ttrpg/foundry-adapter      # Foundry adapter + relay module (117 tests)
pnpm test --filter @nexus-ttrpg/roll20-adapter       # Roll20 adapter (48 tests)
pnpm test --filter @nexus-ttrpg/pathbuilder-adapter  # Pathbuilder adapter (20 tests)
pnpm test --filter @nexus-ttrpg/adapter-sdk          # SDK + contract harness (19)
```

## Load-bearing invariants — do NOT break these

1. **`sync-engine` is pure and I/O-free.** Owned data in, owned data out — no DB
   handle, no HTTP, no `async`. Enforced structurally (it's in a different
   language behind an op interface). `merge()` is a pure function of three
   `CharacterBuild`s. If you find yourself adding `async` or a store handle to
   `crates/sync-engine`, stop.

2. **One-way codegen (Rust → JSON Schema → TS).** Rust types in `schema-core`
   are the only hand-written source. Never generate Rust from JSON Schema. The
   generated `packages/schema-types` and `packages/schema-zod` are committed so
   TS contributors need no Rust toolchain.

3. **Adapters never touch the engine.** A contributor adding an adapter works
   entirely inside `adapters/` and `packages/`, runs `pnpm test`, and never opens
   `crates/`. The Cargo boundary makes `sync-engine` depending on anything but
   `schema-core` a compile error.

4. **Conflicts are never auto-resolved.** Three-way merge returns conflicts for a
   human to resolve per field before the merge snapshot commits.

5. **Rollback is `revert`, not `reset --hard`.** History is an immutable snapshot
   DAG; rollback records a new snapshot, never destructively rewrites.

6. **Session state never auto-syncs.** HP, conditions, spell slots, initiative
   are scene-scoped, not character-scoped. `SessionTelemetry` has no conversion
   into `CharacterBuild` — enforced by types, not convention.

7. **Adapters are untrusted.** Everything crossing the boundary upward is
   validated against the canonical schema in Rust before reaching the engine.

8. **Unknown/homebrew content is always a warning, never an error.**

9. **Sync ops are idempotent and content-addressed.** Keyed by `content_hash` +
   parent snapshot; replay after a flaky reconnect is a no-op.

## Repo layout

```
crates/         Rust core (the asset)
  schema-core/    SOURCE OF TRUTH — canonical Rust types, BLAKE3 hashing
  schema-pf2e/    PF2e SystemPlugin
  schema-sf2e/    SF2e SystemPlugin (delegates to schema-core::chassis)
  sync-engine/    Pure diff/merge/DAG — depends on schema-core ONLY
  sync-engine-wasm/  cdylib wrapper over sync-engine — the ONLY wasm-bindgen use; powers the web diff preview
  store/          SQLx Postgres + SQLite, AES-256-GCM credentials
  adapter-host/   deno_core V8 sandbox, boundary validation
  api/            Axum REST API, utoipa OpenAPI, webhooks
packages/       TS shared (generated + SDK)
  schema-types/   GENERATED — do not hand-edit
  schema-zod/     GENERATED — do not hand-edit
  adapter-sdk/    CharacterAdapter interface, contract test harness
  foundry-mapping/  PF2e Actor ↔ canonical mapping logic
  web/            Conflict UI — History / Diff / Merge Lab, client-side WASM diff
adapters/       TS, contributed (disposable)
  foundry/        Reference adapter + relay module (117 tests)
                  src/       — server-side adapter (adapter-host)
                  module/    — in-browser relay (§7.1: shares foundry-mapping by plain import)
  roll20/         Second adapter — flat attrs, repeating sections, asleep sandbox (48 tests)
  pathbuilder/    First third-party-builder adapter — import-only (§8), non-lossy ability components (20 tests)
cli/            `nexus` — Rust, clap, single static binary
xtask/          cargo xtask codegen | openapi | fixtures
examples/       fixtures/ (shared JSON ground-truth), example-adapter/
docs/           ADRs, adapter-authoring guide, self-hosting
```

## Common pitfalls

- **`deno_core` V8 is `!Send`.** All host-calling code must run inside
  `tokio::task::LocalSet` on a current-thread runtime. The API bridges this with
  `spawn_blocking` + fresh current-thread runtime + `LocalSet` per adapter call
  (see `crates/api/src/adapter_worker.rs`).
- **`AdapterId` is a newtype.** Compare with `adapter.as_str() == "foundry"`, not
  direct `==`.
- **Axum 0.8 path syntax:** `{id}`, not `:id`.
- **`sync_engine::FieldDiff` and `Change` do NOT derive `Serialize`.** Manual
  conversion to JSON in `crates/api/src/patch.rs`.
- **`Ulid`, `ContentHash`, `CharacterBuild` do NOT impl `utoipa::ToSchema`.**
  DTOs use `String` / `serde_json::Value` and convert at the handler boundary.
- **Diff paths are `build.`-prefixed** (see `crates/sync-engine/src/diff.rs:44`).
  `apply_merge` in `patch.rs` wraps `ours` in `{"build": ...}` for this reason.
- **`secrecy::SecretString::new()` expects `Box<str>`.** Use
  `SecretString::from(string)` instead.
- **`SqliteStore::connect()` does not set `create_if_missing(true)`.** CLI uses
  `SqliteConnectOptions::from_str().create_if_missing(true)` instead.
- **Adapter bundles are single-file esbuild ESM output** (no npm imports) loaded
  by `deno_core` as ES modules. `host.rs` appends
  `globalThis.__nexusAdapter = adapter;` to the module source.
- **tsconfig exclude pattern:** use `"**/*.test.ts"`, not `"src/**/*.test.ts"`.
- **Patch path format:** dot-separated keys (`build.identity.name`), bracket
  slugs (`build.feats[toughness]`), bracket indices (`build.spellcasting.entries[0]`).

## Commit conventions

- Conventional Commits style (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`).
- Do not commit unless explicitly asked.
- Never commit secrets, API keys, or credentials.
