# ADR 0001: Phase 1 GO — retain the Rust core / TypeScript adapter split

- **Status:** Accepted
- **Date:** 2026-08-12
- **Supersedes:** none (first ADR). Records the decision the architecture doc
  defers to this point — §0.2, §19, §21 risk row 1.
- ** consulted sections:** §0.2 (what this costs, honestly), §1 (overall
  architecture), §5.5 (adapter host), §11 (diff/merge/DAG), §16 (testing
  philosophy), §19 (MVP roadmap — Phase 1 gate), §21 (risks, row 1).

## Context

The architecture (§0.2) makes an explicit bet: a Rust core (schema,
sync-engine, store, adapter-host, API) plus TypeScript adapters, connected by
one-way codegen and an embedded V8 runtime. The bet is worth revisiting at the
end of Phase 1, when the host, the codegen, and one real adapter all exist and
the maintenance cost is *measurable rather than speculative* (§21 risk row 1).
Deferring the decision past Phase 1 is what makes it expensive — the canonical
model, the adapter interface, and the conceptual architecture are all
language-agnostic and would survive a collapse to all-TypeScript, but only
while the surface is still small.

This ADR records the measurement and the decision.

## The gate criteria (§19, §21)

All five criteria from the implementation plan's Phase 1 gate are met:

### 1. The Foundry adapter round-trips in the real adapter-host, not mocked

**Met.** The CLI integration tests (`cli/src/commands.rs`) drive the full
path: a canonical build is exported to a Foundry Actor JSON via the real
`adapter-host` (V8 sandbox), the Actor JSON is re-imported through the same
host (boundary-validated), and the result is stored as a snapshot in SQLite.
Six tests cover import, re-import with parent linkage, diff against HEAD,
push back out, history listing, and idempotent replay. The adapter running
in these tests is the same esbuild bundle a real deployment loads — no test
shim, no mocked V8.

Evidence: `cli/src/commands.rs:607` (test module), 6 tests, all passing.

### 2. Boundary tests prove the core rejects malformed adapter output and enforces capability grants

**Met.** `crates/adapter-host/tests/boundary.rs` has 19 boundary tests
covering:

- **Boundary rejection** (§5.5): unknown fields in `ImportOutcome`, in the
  build, and in `ExportOutcome` are rejected; wrong types for fields are
  rejected; valid imports/exports pass. The `deny_unknown_fields` serde
  attribute is the mechanism, and the tests prove it fires.
- **Capability enforcement** (§5.5): fetch to an empty allowlist is denied;
  fetch to a non-allowlisted host is denied; cleartext HTTP is denied; fetch
  to an allowlisted host succeeds. Capabilities are granted, not ambient.
- **Containment** (§21): a hanging adapter times out (30s default); a
  throwing adapter returns an error rather than panicking; the host survives
  a bad adapter and can serve the next call; module load failures (syntax
  errors, missing `adapter` export) are caught.

Plus 14 unit tests in `validation.rs` and `manifest.rs` for the boundary
deserialization and capability-matching logic. All 33 pass.

### 3. The contract harness runs standalone in pure TS, no Rust in an adapter author's loop

**Met.** `packages/adapter-sdk/src/testing.ts` exports
`runContractTests(adapter, fixtures)`. The Foundry adapter's test suite
(`adapters/foundry/src/index.test.ts`) runs 29 tests via `pnpm test` with no
Rust toolchain, no V8, no host, no database — plain Vitest. A contributor
writing an adapter runs `npm install && npm test` and gets the full contract
suite (round-trip within `fieldCoverage`, unknown content warns rather than
throws, `import(export(fixture))` is stable). If they ever need `cargo` to
iterate on an adapter, that's a bug in the SDK (§16) — and they don't.

Evidence: `adapters/foundry/src/index.test.ts:50` (`runContractTests`
invocation), 29 tests passing in 322ms with no Rust in the loop.

### 4. The codegen staleness check is green after Phase 1 changes

**Met.** `cargo xtask codegen` regenerates `canonical.schema.json`,
`packages/schema-types/src/index.ts`, and `packages/schema-zod/src/index.ts`
from `crates/schema-core`. After all Phase 1 work (Foundry adapter, CLI,
bundle), regenerating produces zero diff in the committed generated files.
The CI job (`git diff --exit-code` after codegen) is the actual control
(§21 risk row 3); the header comments and `package.json` descriptions are
signage.

Evidence: `cargo xtask codegen && git diff --exit-code -- packages/ crates/schema-core/canonical.schema.json` exits clean.

### 5. `deno_core`'s maintenance burden is measurable, not speculative

This is the criterion that required Phase 1 to complete before assessing.
The measurement, from the code that exists:

**What's in the codebase:** ~1,300 lines across `adapter-host` (src + tests).
The op surface is deliberately tiny — `ops.rs` is 144 lines, exposing only
`nexus_fetch` (host-side HTTP, allowlist-enforced) and `nexus_log`. No
filesystem, no environment, no DB handle (§5.5). The host creates a fresh
`JsRuntime` per sync operation (adapters are stateless, §5.4), so there's no
long-lived V8 isolate to manage.

**Where the friction actually was, ranked by cost:**

1. **V8 scope API (one-time, ~15 lines).** Extracting a return value from a
   resolved V8 promise required learning the v8 150.4.0 scope dance:
   `HandleScope::new` → `pin!` → `init()` → `ContextScope`. This is dense,
   undocumented-in-our-code, and version-tied to the `v8` crate that
   `deno_core` re-exports. It cost ~2 hours once, is now captured in a
   comment at `host.rs:134`, and has not recurred. It is the single most
   fragile snippet in the host.

2. **Promise resolution + event loop polling (one-time, ~10 lines).** The
   `resolve()` / `with_event_loop_promise` pattern (`host.rs:121-131`) is a
   deno_core 0.410 API quirk: resolving a promise requires concurrent event
   loop polling or the future never completes. Captured in a comment; has
   not recurred.

3. **`!Send` threading (per-call-site, ~3 lines each).** `JsRuntime` is
   `!Send` (V8 isolates are thread-local), so every entry point that touches
   the host must run inside a `tokio::task::LocalSet`. This surfaced three
   times: the host's own doc comment (`host.rs:4`), the CLI's `main.rs`
   (`main.rs:104-110`), and the CLI test helper (`commands.rs:627`). It's a
   one-line `LocalSet::new()` + `block_on`, not a structural problem — but
   it is a thing callers must know, and it will recur on every new
   host-calling site.

4. **Bundle step for the adapter (one-time, build-script, not runtime).**
   `deno_core` loads an ES module from source, with no Node module resolver.
   The Foundry adapter's workspace deps (`foundry-mapping`,
   `adapter-sdk`) must be bundled into a single self-contained JS file via
   esbuild before the host can load them. This is a build-time concern
   (`adapters/foundry/package.json` `bundle` script, 17.8 KB output), not a
   runtime or maintenance burden on the host — but it is a step that
   all-TypeScript would not have.

**What did *not* turn out to be a cost:**

- The op layer (`ops.rs`) is 144 lines and stable. The "keep the op surface
  tiny" mitigation from §21 risk row 2 is working.
- Boundary validation (`validation.rs`, 154 lines) is plain serde
  deserialization with `deny_unknown_fields`. No V8 involvement.
- The codegen pipeline (§3.6) is an `xtask` that shells out to `json2ts` and
  `json-schema-to-zod`. It has not needed attention since Phase 0.
- Cross-boundary debugging has not been an issue because the boundary is
  JSON-over-op, not FFI. An adapter bug is a JS bug, debugged in the adapter;
  a host bug is a Rust bug, debugged in the host.

**The escape hatch is real, not theoretical (§21 risk row 2).** The
`"runtime": "external"` manifest option (`manifest.rs:17-23`,
`external.rs`) spawns the adapter as a subprocess speaking JSON-RPC over
stdio. The `AdapterHost` enum (`lib.rs:44`) dispatches transparently. If
`deno_core` becomes untenable, promoting external to primary is a config
change, not a rewrite — the adapter interface is identical either way.

## The other side of the bet: what the Rust core earns

The maintenance cost above is weighed against what the all-TypeScript
alternative could not provide:

- **`sync-engine` is pure and I/O-free (§11.3).** `merge()` is a pure
  function of three `CharacterBuild`s — no DB, no HTTP, no `async`. This is
  enforced *structurally* (it's in a different language behind an op
  interface), not by convention. In all-TypeScript this would be a
  gentleman's agreement.
- **Property-tested merge laws (§16).** The merge engine has proptest
  coverage asserting algebraic laws: `merge(a,x,x)` never conflicts,
  single-sided changes are never lost, identical inputs never conflict. The
  diff engine has proptest coverage for symmetry. These are exhaustive over
  generated inputs, catching interleaving bugs that example-based tests
  structurally cannot.
- **`deny_unknown_fields` at the boundary (§5.5).** A malformed adapter
  payload fails loudly at deserialization, before the engine sees it. In
  TS-only Zod this is runtime validation; in Rust it's a compile-time-ish
  guarantee (if you have a `CharacterBuild`, it's structurally valid by
  construction).
- **Conflict exhaustiveness (§11.3).** The `Resolution` enum is exhaustive —
  a new merge outcome is a compile error across every match, forcing
  coverage. In TS this is a runtime default branch.

## Decision

**GO. Retain the Rust core / TypeScript adapter split.**

The maintenance burden of the cross-language boundary is *measurable and
bounded*: ~25 lines of version-tied V8 scope/promise code (captured in
comments, not recurred), a `LocalSet` requirement at host call sites (one
line each), and an esbuild bundle step in the adapter build script. Against
that, the sync-engine's purity, property-testability, and structural
boundary enforcement are earning their keep — and are properties the
all-TypeScript alternative could only approximate by convention.

The escape hatch (`"runtime": "external"`, §21 risk row 2) remains available
if `deno_core`'s upgrade cadence or V8 API churn becomes unmanageable. The
decision to retain the split does not foreclose it.

## What would change this decision

- If a `deno_core` or `v8` crate upgrade breaks the scope/promise code
  (`host.rs:121-145`) and the fix is non-trivial, reconsider external as
  primary.
- If Phase 2's Roll20 adapter or the Foundry relay module
  (`foundry-mapping` shared between server adapter and browser relay, §7.1)
  reveals that the bundle step is a recurring source of bugs across
  multiple adapters, reconsider.
- If a second adapter author reports that the contract harness is
  insufficient and they need to debug *through* the V8 boundary to
  understand a host rejection, the boundary is costing more than it's
  saving.
- If the merge engine's property tests stop finding bugs that example-based
  tests would have missed, the correctness advantage is theoretical and the
  cost-benefit shifts.

## Consequences

- Phase 2 proceeds with the current architecture: Axum API reusing
  `store`/`sync-engine`/`adapter-host`; Roll20 adapter following the Foundry
  pattern; Foundry relay module sharing `foundry-mapping` with the adapter.
- The `deno_core` version is pinned in `Cargo.toml` and tracked. Upgrades
  are a deliberate step, not a `cargo update` side effect.
- The esbuild bundle step is part of the adapter authoring contract. The
  adapter-authoring guide (Phase 3 deliverable, §19) must document it.
- This ADR is the first; future decisions that supersede specific choices in
  `cross-vtt-sync-architecture.md` follow the same convention
  (`docs/adr/README.md`).
