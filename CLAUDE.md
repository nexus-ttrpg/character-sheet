# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this workspace is

This is **not a code repository.** It contains a single file — `cross-vtt-sync-architecture.md` (~1090 lines) — which is a pre-implementation **architecture proposal** for *Nexus Sheet*, a cross-VTT (Virtual Tabletop) character synchronization platform for PF2e / SF2e. There is no git history, no source code, no build system, and no tests. The status line in the doc itself reads "Architecture proposal (pre-implementation)."

Work here means **editing, reviewing, or extending that design document** — or, eventually, scaffolding the implementation it describes. Do not invent or run build/test/lint commands; none exist yet. The commands and repo layout below are *design intent quoted from the doc*, not things you can execute in this workspace.

## The one artifact

`cross-vtt-sync-architecture.md` is the whole project. It is a "living reference" (its own closing line): ADRs in a future `docs/adr/` are expected to supersede specific choices as the project evolves. When editing it, preserve its conventions:

- **Section cross-references** as `§N` or `§N.M` (e.g. `§5.5`).
- **Inline citations** as `<cite index="N-M">claim</cite>` — these point to an external source bibliography that is *not* in this file; keep the index form when adding sourced claims.
- **ASCII architecture diagrams** inside fenced code blocks (see §1, §3.6).
- Numbered top-level sections `## 0` … `## 21`, with `### N.M` subsections.

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

## Intended tech stack (§14 — design intent, not installed)

Rust core: Axum, SQLx (PostgreSQL hosted + SQLite offline CLI, one codebase), serde/schemars/garde, utoipa (OpenAPI), `deno_core` (embeds V8 to run TS adapters in-process, no Node on server), BLAKE3 content hashing, `cargo-deny`/`cargo-audit`. TS side: TypeScript + pnpm + Vitest + tsup. Narrow `wasm-bindgen` use only for the browser-side diff preview (Phase 3). Two workspaces side by side (Cargo + pnpm); Turborepo deliberately not used.

## Intended repo layout (§15 — design intent, not present)

```
crates/        schema-core (SOURCE OF TRUTH), schema-pf2e, schema-sf2e,
               sync-engine (pure, no I/O, no adapter deps), store, adapter-host, api
packages/      schema-types + schema-zod (GENERATED — do not hand-edit),
               adapter-sdk, foundry-mapping
adapters/      roll20, foundry, pathbuilder, wanderers-guide, demiplane
cli/           `nexus` (Rust, clap, single static binary)
xtask/         cargo xtask codegen | openapi | fixtures
examples/      fixtures/ (shared JSON ground-truth, both languages), example-adapter/
```

The one boundary that matters: `sync-engine` depends on `schema-core` and nothing else — enforced by Cargo, so violating it is a compile error. A contributor adding an adapter works entirely inside `adapters/` and `packages/`, runs `pnpm test`, and never opens `crates/`.

## Intended commands (§16 — design intent, none runnable here)

- `cargo test` — per-crate unit tests, weighted toward `sync-engine`.
- `proptest` against the merge engine — asserts algebraic laws (merge identity, `merge(a,x,x)` never conflicts, single-sided changes never lost, `hash(x) == hash(roundtrip(x))`).
- `cargo xtask codegen && git diff --exit-code` — the codegen staleness check; the whole cross-language schema guarantee rests on this one CI job.
- `pnpm test` in an adapter directory — plain Vitest, no host, no DB, no network (adapters are stateless, §5.4). If a contributor ever needs `cargo` to iterate on an adapter, that's a bug in the SDK.
- `@nexus-sheet/adapter-sdk/testing` — `runContractTests(adapter, fixtures)` in one line; asserts `import(export(fixture))` round-trips within `fieldCoverage`, unknown content warns rather than throws.

## MVP sequencing (§19)

Phase 0 foundations (schema-core + sync-engine + store, proptest suite from day one, codegen staleness check before anything depends on generated types) → **Phase 1 adapter boundary** (`adapter-host` + `adapter-sdk` + Foundry reference adapter, built alongside each other) — this phase is the project's real risk concentration and has an explicit go/no-go checkpoint: if the cross-language machinery costs more than it saves, collapsing to all-TypeScript is still cheap here because the canonical model, adapter interface, and conceptual architecture are language-agnostic → Phase 2 Axum API + Roll20 + SF2e → Phase 3 conflict UX (sync-engine compiled to WASM for diff preview) + hardening + adapter-authoring guide.

Foundry is the reference adapter every later adapter is modeled on; Roll20 is second because its sandbox constraints (§6) make it structurally harder.
