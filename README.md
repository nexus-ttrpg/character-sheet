# Nexus Character Sheet — A Cross-VTT Character Synchronization Platform for PF2e / SF2e

"Git for TTRPG characters": a canonical character model and a diff/merge engine
sitting between platform adapters. Adapters are disposable; the canonical model
and the merge engine are the asset.

Part of [nexus-ttrpg](https://github.com/nexus-ttrpg). Formerly *Nexus Sheet*.

**Status:** MVP complete (Phase 3 gate met), with post-MVP work in progress.
Phases 0 (foundations), 1 (adapter boundary, GO decision in
[ADR 0001](./docs/adr/0001-phase1-go-no-go-rust-ts-split.md)), 2 (Axum API,
Roll20 adapter, Foundry relay module, SF2e) and 3 (conflict UI, WASM diff
preview, migration framework, hardening, adapter-authoring guide) are done.
432 tests (210 Rust + 222 TS; CI runs 217 TS, because 6 WASM-parity tests in
`packages/web` need `pnpm build:wasm` first). See
[`implementation-plan.md`](./implementation-plan.md) for the phased build plan
and [`cross-vtt-sync-architecture.md`](./cross-vtt-sync-architecture.md) for
the full design.

## What's built

| Component | Status | Description |
|-----------|--------|-------------|
| `crates/schema-core` | ✅ | Canonical Rust types, BLAKE3 hashing, `schemars` JsonSchema, shared PF2e/SF2e chassis, migrations |
| `crates/schema-pf2e` | ✅ | PF2e SystemPlugin |
| `crates/schema-sf2e` | ✅ | SF2e SystemPlugin: drones, Resolve Points, credits/UPB |
| `crates/sync-engine` | ✅ | Pure diff/merge/snapshot DAG, proptest laws, rollback as revert |
| `crates/sync-engine-wasm` | ✅ | WASM build of the engine for the browser diff preview |
| `crates/store` | ✅ | SQLx Postgres + SQLite, AES-256-GCM credential encryption, migration on read |
| `crates/adapter-host` | ✅ | `deno_core` V8 sandbox, boundary validation, capability enforcement |
| `crates/api` | ✅ | Axum REST API, utoipa OpenAPI, API keys, rate limiting, webhooks |
| `cli/` | ✅ | `nexus import\|push\|diff\|history`, single static binary |
| `packages/adapter-sdk` | ✅ | `CharacterAdapter` interface, contract test harness |
| `packages/foundry-mapping` | ✅ | PF2e/SF2e Actor ↔ canonical mapping, shared by adapter and relay module |
| `packages/web` | ✅ | Conflict UI: history, diff and merge views, client-side WASM diff |
| `adapters/foundry` | ✅ | Reference adapter + in-browser relay module (`nexus-character-sheet`) |
| `adapters/roll20` | ✅ | Second adapter: flat attributes, repeating sections, sandboxed collector |
| `adapters/pathbuilder` | ⚠️ Experimental | Import-only. Field names are modelled on the design doc and **not yet verified against a real Pathbuilder 2e export**; don't use it on live data yet |
| `packages/schema-types`, `packages/schema-zod` | Generated | TS types and Zod schemas from `canonical.schema.json` |

## Contributing an adapter

A contributor adding an adapter works entirely inside `adapters/` and `packages/`,
runs `pnpm test`, and never opens `crates/`. **A contributor writing a new
adapter never installs Rust.** Start from
[`docs/adapter-authoring-guide.md`](./docs/adapter-authoring-guide.md) and
`examples/example-adapter/`.

The one boundary that matters: `crates/sync-engine` depends on `schema-core` and
nothing else — enforced by Cargo, so violating it is a compile error.

## Repository layout

See §15 of the architecture doc. In short:

```
crates/        Rust core: schema-core (source of truth), schema-pf2e, schema-sf2e,
               sync-engine (pure, no I/O), sync-engine-wasm, store, adapter-host, api
packages/      TS shared: schema-types + schema-zod (GENERATED), adapter-sdk,
               foundry-mapping, web
adapters/      TS, contributed: foundry (reference adapter + relay module), roll20,
               pathbuilder
cli/           `nexus` — Rust, clap, single static binary
xtask/         cargo xtask codegen | openapi | fixtures
examples/      fixtures/ (shared JSON ground truth), example-adapter/
docs/          ADRs, adapter-authoring guide, self-hosting
```

## Build & test

### Rust

```
cargo test                              # all crates (210 tests)
cargo test -p sync-engine               # merge engine + proptest laws
cargo test -p api                       # API unit + integration tests
cargo clippy --workspace --all-targets -- -D warnings   # lint, as CI runs it
cargo fmt --all -- --check              # format check
cargo xtask codegen && git diff --exit-code  # codegen staleness check (§3.6)
cargo deny check                        # supply chain (licenses, advisories, bans)
```

### TypeScript

```
pnpm install                            # install all workspace deps
pnpm test                               # all packages + adapters (217, or 222 with WASM built)
pnpm test --filter @nexus-ttrpg/foundry-adapter      # Foundry adapter + relay module
pnpm test --filter @nexus-ttrpg/roll20-adapter       # Roll20 adapter
pnpm test --filter @nexus-ttrpg/pathbuilder-adapter  # Pathbuilder adapter
pnpm test --filter @nexus-ttrpg/adapter-sdk          # SDK + contract harness
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
- Node 22, pinned via `.nvmrc` and `package.json` `engines`; pnpm 11 via
  `packageManager`. Newer Node versions can break native dependencies.
- `cargo xtask codegen && git diff --exit-code` — the codegen staleness check
  (§3.6); the whole cross-language schema guarantee rests on this one CI job.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or
[MIT license](./LICENSE-MIT), at your option.

Pathfinder and Starfinder are trademarks of Paizo Inc. This project is not
affiliated with or endorsed by Paizo, Foundry Gaming or Roll20.
