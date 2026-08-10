# Nexus Sheet — A Cross-VTT Character Synchronization Platform for PF2e / SF2e

"Git for TTRPG characters": a canonical character model and a diff/merge engine
sitting between platform adapters. Adapters are disposable; the canonical model
and the merge engine are the asset.

**Status:** Architecture proposal, pre-implementation. See
[`cross-vtt-sync-architecture.md`](./cross-vtt-sync-architecture.md) for the full
design and [`implementation-plan.md`](./implementation-plan.md) for the phased
build plan.

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
adapters/      TS, contributed: roll20, foundry, pathbuilder, wanderers-guide, demiplane
cli/           `nexus` — Rust, clap, single static binary
xtask/         cargo xtask codegen | openapi | fixtures
examples/      fixtures/ (shared JSON ground-truth), example-adapter/
docs/          architecture, ADRs, adapter-authoring guide
```

## Tooling

- Rust toolchain pinned via `rust-toolchain.toml` (1.97.1).
- Node pinned via `.nvmrc` (22) and `package.json` `engines`; pnpm via corepack.
- `cargo xtask codegen && git diff --exit-code` — the codegen staleness check
  (§3.6); the whole cross-language schema guarantee rests on this one CI job.
