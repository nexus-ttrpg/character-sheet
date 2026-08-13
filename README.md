# Nexus Sheet — A Cross-VTT Character Synchronization Platform for PF2e / SF2e

"Git for TTRPG characters": a canonical character model and a diff/merge engine
sitting between platform adapters. Adapters are disposable; the canonical model
and the merge engine are the asset.

**Status:** Phase 2 in progress. Phase 0 (foundations), Phase 1 (adapter
boundary — GO decision in [ADR 0001](./docs/adr/0001-phase1-go-no-go-rust-ts-split.md)),
Phase 2.1 (Axum API), Phase 2.2 (Roll20 adapter), and Phase 2.4 (SF2e system
plugin + Foundry SF2e coverage) are complete. 238 tests passing (132 Rust +
106 TS). See [`implementation-plan.md`](./implementation-plan.md) for the phased
build plan and [`cross-vtt-sync-architecture.md`](./cross-vtt-sync-architecture.md)
for the full design (~1090 lines).

## What's built

| Component | Status | Tests | Description |
|-----------|--------|-------|-------------|
| `crates/schema-core` | ✅ Complete | 30 | Canonical Rust types, BLAKE3 hashing, `schemars` JsonSchema, shared chassis (validate/derived) |
| `crates/schema-pf2e` | ✅ Complete | 13 | PF2e SystemPlugin extending schema-core |
| `crates/schema-sf2e` | ✅ Complete | 8 | SF2e SystemPlugin — drones, Resolve Points, credits/UPB (§3.4) |
| `crates/sync-engine` | ✅ Complete | 15 | Pure diff/merge/DAG, proptest laws, rollback-as-revert |
| `crates/store` | ✅ Complete | 8 | SQLx Postgres + SQLite, AES-256-GCM credential encryption |
| `crates/adapter-host` | ✅ Complete | 33 | `deno_core` V8 sandbox, boundary validation, capability enforcement |
| `crates/api` | ✅ Complete | 10 | Axum REST API, utoipa OpenAPI, webhooks (Phase 2.1) |
| `cli/` | ✅ Complete | 6 | `nexus import\|push\|diff\|history` — single static binary |
| `packages/adapter-sdk` | ✅ Complete | 19 | `CharacterAdapter` interface, contract test harness |
| `packages/foundry-mapping` | ✅ Complete | — | PF2e/SF2e Actor ↔ canonical mapping logic (drones, vehicles, credits/UPB) |
| `adapters/foundry` | ✅ Complete | 39 | Reference adapter (identity through spellcasting; SF2e drones/vehicles) |
| `adapters/roll20` | ✅ Complete | 48 | Second adapter — flat attrs, repeating sections, asleep sandbox (§6) |
| `packages/schema-types` | ✅ Generated | — | TS types from `canonical.schema.json` |
| `packages/schema-zod` | ✅ Generated | — | Zod schemas from `canonical.schema.json` |

## Contributing an adapter

A contributor adding an adapter works entirely inside `adapters/` and `packages/`,
runs `pnpm test`, and never opens `crates/`. **A contributor writing a new
adapter never installs Rust.** Their loop is `npm install`, `npm test`, open a PR.

The one boundary that matters: `crates/sync-engine` depends on `schema-core` and
nothing else — enforced by Cargo, so violating it is a compile error.

## Repository layout

See §15 of the architecture doc. In short:

```
crates/        Rust core: schema-core (source of truth), schema-pf2e, schema-sf2e,
               sync-engine (pure, no I/O), store, adapter-host, api
packages/      TS shared: schema-types + schema-zod (GENERATED), adapter-sdk,
               foundry-mapping
adapters/      TS, contributed: foundry (reference), roll20 (second adapter)
cli/           `nexus` — Rust, clap, single static binary
xtask/         cargo xtask codegen | openapi | fixtures
examples/      fixtures/ (shared JSON ground-truth), example-adapter/ (Phase 3)
docs/          ADRs
```

## Build & test

### Rust

```
cargo test                              # all crates (132 tests)
cargo test -p sync-engine               # merge engine + proptest laws
cargo test -p api                       # API unit + integration tests
cargo test -p adapter-host              # boundary + unit tests (33)
cargo clippy --workspace --tests        # lint
cargo fmt --check                       # format check
cargo xtask codegen && git diff --exit-code  # codegen staleness check (§3.6)
cargo deny check                        # supply chain (licenses, advisories, bans)
```

### TypeScript

```
pnpm install                            # install all workspace deps
pnpm test                               # all packages + adapters (106 tests)
pnpm test --filter @nexus-sheet/foundry # Foundry adapter (39 tests)
pnpm test --filter @nexus-sheet/roll20  # Roll20 adapter (48 tests)
pnpm test --filter @nexus-sheet/adapter-sdk  # SDK + contract harness (19 tests)
```

### CLI

```
cargo run -p nexus-cli -- import --foundry <world-json>  # import → snapshot → HEAD
cargo run -p nexus-cli -- push --foundry                 # export HEAD → Foundry payload
cargo run -p nexus-cli -- diff                            # field-level diff
cargo run -p nexus-cli -- history                         # snapshot DAG
```

## Tooling

- Rust toolchain pinned via `rust-toolchain.toml` (1.97.1).
- Node pinned via `.nvmrc` (22) and `package.json` `engines`; pnpm via corepack.
- `cargo xtask codegen && git diff --exit-code` — the codegen staleness check
  (§3.6); the whole cross-language schema guarantee rests on this one CI job.
