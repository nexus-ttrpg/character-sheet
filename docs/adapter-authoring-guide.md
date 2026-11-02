# Adapter Authoring Guide

This guide takes you from `npm install` to a merged adapter PR. You will never
need Rust, `cargo`, or any Rust toolchain — the entire loop is TypeScript.

## The big picture

Nexus Character Sheet sits between disposable platform adapters. Your adapter translates
between a VTT's character format and the canonical model. The canonical model
and merge engine are Rust; adapters are TypeScript. The boundary is enforced in
Rust — everything your adapter returns is validated against the canonical schema
before it reaches the engine. You don't need to trust your own output; the host
does it for you.

```
VTT platform  ←→  Your Adapter (TS)  ←→  Boundary Validation (Rust)  ←→  Engine (Rust)
```

## What you need

- Node.js 22+ (see `.nvmrc`)
- pnpm (`corepack enable && corepack prepare pnpm@latest --activate`)
- The repo cloned

That's it. No Rust, no Docker, no Postgres.

## The 5-minute version

```bash
# 1. Install deps
pnpm install

# 2. Clone the template
cp -r examples/example-adapter adapters/my-platform

# 3. Edit package.json: rename to @nexus-ttrpg/my-platform-adapter

# 4. Edit src/index.ts: change id, rewrite the payload type and mappings

# 5. Edit src/bundle.ts: update the import

# 6. Edit src/index.test.ts: update the import

# 7. Test
pnpm --filter @nexus-ttrpg/my-platform-adapter test

# 8. Bundle
pnpm --filter @nexus-ttrpg/my-platform-adapter bundle

# 9. Open a PR
```

## The `CharacterAdapter<TPlatform>` interface

Your adapter implements one interface from `@nexus-ttrpg/adapter-sdk`:

```ts
import type { CharacterAdapter } from "@nexus-ttrpg/adapter-sdk";

const myAdapter: CharacterAdapter<MyPlatformPayload> = {
  id: "my-platform",
  capabilities: { ... },
  async import(raw, ctx) { ... },
  async export(build, ctx) { ... },
  async detectChanges(previous, current) { ... },
  async validate(raw) { ... },
};
```

### `id: string`

A unique identifier for your platform: `"foundry"`, `"roll20"`, `"pathbuilder"`,
etc. Used in connection records and adapter loading.

### `capabilities: AdapterCapabilities`

Static metadata declared up front — readable without touching the network:

```ts
{
  supportsImport: true,       // platform → canonical
  supportsExport: true,       // canonical → platform
  supportsPartialExport: true,
  supportsSessionRead: false, // HP, conditions, initiative (session state)
  fieldCoverage: [
    "build.identity.name",
    "build.level",
    "build.ancestry",
    // ...every canonical path you round-trip
  ],
}
```

`fieldCoverage` is the honesty contract: it declares exactly which canonical
fields your adapter can round-trip. The UI uses this to show players a coverage
preview before they push. Use `canonicalPath()` to get compile-time checking:

```ts
import { canonicalPath } from "@nexus-ttrpg/adapter-sdk";

const COVERAGE = [
  canonicalPath("build.identity.name"),
  canonicalPath("build.level"),
  canonicalPath("build.ancestry"),
];
```

A typo in a path is now a compile error, not a runtime surprise.

### `import(raw, ctx): Promise<ImportOutcome>`

Maps a platform payload to a partial canonical build. Returns:

```ts
{
  build: PartialBuild,      // canonical fields you mapped (all optional)
  warnings: AdapterWarning[], // homebrew, lossy mappings, fuzzy matches
  unmapped: string[],        // canonical paths you couldn't map
}
```

**Never throw on unknown content.** Real VTT data is full of homebrew. If you
encounter content you don't recognize, emit a warning and continue. Throwing on
unrecognized content makes your adapter unusable in real worlds.

```ts
// Good: unknown feat type → warning
warnings.push({
  path: "build.feats[2]",
  kind: { type: "unmappedField" },
  detail: `Unknown feat type "${type}" — preserved in extensions`,
});
```

You MAY throw `AdapterError` for conditions where proceeding would corrupt data:
`unsupportedSheetVersion`, `malformedPayload`, `auth`, `transport`.

### `export(build, ctx): Promise<ExportOutcome<TPlatform>>`

Maps a canonical build to your platform's payload. Returns:

```ts
{
  payload: MyPlatformPayload,
  warnings: AdapterWarning[],  // fields you had to drop
  skipped: string[],           // canonical paths with no platform representation
}
```

**Never write session state.** Current HP, conditions, spell slots, initiative
are scene-scoped, not character-scoped. Write max HP only, never current/temp HP.

### `detectChanges(previous, current): Promise<ChangeSet>`

A cheap structural diff — cheaper than a full import/merge cycle. Used for drift
detection (is the platform's data different from what we last synced?).

```ts
{
  hasChanges: boolean,
  changedPaths: string[],
}
```

A JSON stringify comparison is fine. Don't over-engineer this.

### `validate(raw): Promise<ValidationReport>`

Structural validation of a platform payload before import. Returns:

```ts
{
  valid: boolean,
  errors: string[],
  warnings: string[],
}
```

This is your chance to reject garbage early with a helpful message. You may
throw `AdapterError` for `unsupportedSheetVersion` here — that's the one case
where proceeding would silently corrupt a sheet.

## The contract test harness

Your adapter gets a correctness bar for free. One line in your test file:

```ts
import { describe } from "vitest";
import { runContractTests } from "@nexus-ttrpg/adapter-sdk/testing";

describe("my-platform", () => {
  runContractTests(myAdapter, fixtures);
});
```

The harness asserts five contracts:

1. **Adapter shape** — non-empty `id`, `fieldCoverage` is an array
2. **Round-trip** — for each fixture: `export(build)` → `import(payload)` preserves every `fieldCoverage` field present in the original
3. **Validate rejects bad input** — each fixture's `badPayload` returns `valid: false`
4. **detectChanges no-op** — `detectChanges(payload, payload)` returns `hasChanges: false`
5. **Unknown content warns, never throws** — injecting an unknown field into the payload must not throw

### Building fixtures

Fixtures come from `examples/fixtures/` — the shared ground-truth set. Each
`AdapterFixture` needs:

```ts
{
  name: "level1-fighter",
  build: CharacterBuild,       // loaded from examples/fixtures/*.json
  platformPayload: MyPayload,  // a valid platform payload for this character
  badPayload: MyPayload,       // a deliberately broken payload (fails validate)
}
```

The `platformPayload` can be built by calling your own `export()`, by a separate
sync mapping function, or by hand. The `badPayload` should be minimal — empty
name, zero level, etc.

## The bundle

The host loads your adapter as a single-file ESM module in a V8 sandbox. The
bundle entry point must export `adapter`:

```ts
// src/bundle.ts
import { myAdapter } from "./index.js";
export const adapter = myAdapter;
```

Build it:

```bash
pnpm --filter @nexus-ttrpg/my-platform-adapter bundle
# → dist/bundle/bundle.js (single-file ESM, no npm imports)
```

The bundle must be self-contained — no `node_modules`, no dynamic imports.
esbuild handles this with `--bundle --format=esm`.

## The sandbox

Your adapter runs in a `deno_core` V8 sandbox with:

- **No filesystem access**
- **No environment variables**
- **No `require()`** — ESM only
- **HTTPS-only network** — limited to manifest-allowlisted hosts
- **Two ops**: `op_nexus_fetch(url, init?)` and `op_nexus_log(level, message)`

If your platform needs network access (e.g. calling a VTT's API), declare it in
a manifest:

```json
{
  "id": "my-platform",
  "entry": "./dist/bundle/bundle.js",
  "runtime": "inProcess",
  "capabilities": {
    "fetch": { "allow": ["https://api.my-platform.com"] }
  }
}
```

## The invariants

These are enforced by the boundary validation in Rust. Your adapter output is
validated before it reaches the engine — if it violates these, it's rejected:

1. **Unknown fields are rejected.** The canonical schema uses `deny_unknown_fields`. If you return a field that doesn't exist in the schema, it's rejected. Use the generated types from `@nexus-ttrpg/schema-types` to stay within bounds.
2. **Wrong types are rejected.** If `level` must be a number and you return a string, it's rejected.
3. **Homebrew is a warning, never an error.** The contract harness asserts this directly.
4. **Session state is never written.** Max HP only, never current/temp HP.

## The file checklist

```
adapters/my-platform/
├── package.json          # name, scripts (build/bundle/test), deps
├── tsconfig.json         # extends ../../tsconfig.base.json
├── src/
│   ├── index.ts          # CharacterAdapter<TPlatform> implementation
│   ├── bundle.ts         # export const adapter = myAdapter;
│   └── index.test.ts     # runContractTests(myAdapter, fixtures)
└── manifest.json         # (if network access needed)
```

### `package.json` template

```json
{
  "name": "@nexus-ttrpg/my-platform-adapter",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" }
  },
  "scripts": {
    "build": "tsc -p tsconfig.json",
    "bundle": "esbuild src/bundle.ts --bundle --format=esm --target=es2022 --outfile=dist/bundle/bundle.js",
    "test": "vitest run"
  },
  "dependencies": {
    "@nexus-ttrpg/adapter-sdk": "workspace:*",
    "@nexus-ttrpg/schema-types": "workspace:*"
  },
  "devDependencies": {
    "esbuild": "^0.27.0"
  }
}
```

### `tsconfig.json` template

```json
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": {
    "outDir": "./dist",
    "rootDir": "./src"
  },
  "include": ["src"],
  "exclude": ["**/*.test.ts"]
}
```

Note: exclude `"**/*.test.ts"`, NOT `"src/**/*.test.ts"` — the latter misses
test files in subdirectories (AGENTS.md pitfall).

## Reference adapters

Three adapters ship in-repo as worked examples:

- **`adapters/foundry/`** — the reference. Server-side adapter + in-browser relay
  module. Uses `foundry-mapping` for the Actor ↔ canonical logic. Covers PF2e +
  SF2e (drones, vehicles). 21 canonical paths, 89 tests.

- **`adapters/roll20/`** — the structurally harder case. Flat attribute rows +
  repeating sections. Uses a declarative mapping table. Shows lossy export
  warnings (Roll20 stores only derived ability scores, not boosts/flaws). 48
  tests.

- **`adapters/pathbuilder/`** — the first **third-party-builder** adapter (§8),
  and the **import-only** example. Pathbuilder 2e is a character builder, not a
  VTT: it exports JSON but has no write-back API, so `supportsExport: false` (the
  engine won't push to it; `export()` is still implemented for manual re-import
  + the round-trip contract). Shows the **non-lossy** ability-score path: because
  Pathbuilder recomputes the final number itself, canonical's boosts/flaws/itemBonus
  components round-trip exactly (§197) — contrast with Roll20's lossy path above.
  Ships a `manifest.json` scoping fetch to `https://pathbuilder2e.com` (§5.5).
  20 tests. Its field names are a representative model of §8, not verified
  against a real export — see its README.

- **`examples/example-adapter/`** — the minimal template. Generic JSON platform,
  6 canonical paths, 18 contract tests. Start here.

## The PR checklist

Before opening a PR:

- [ ] `pnpm --filter @nexus-ttrpg/my-platform-adapter test` — all contract tests pass
- [ ] `pnpm --filter @nexus-ttrpg/my-platform-adapter bundle` — bundle builds
- [ ] `fieldCoverage` accurately reflects what you round-trip
- [ ] No session state written (max HP only)
- [ ] Unknown content warns, never throws
- [ ] `manifest.json` declares network hosts (if any)

## FAQ

**Do I need Rust?** No. Your entire loop is `pnpm test` and `pnpm bundle`.

**Can I use npm packages?** In your source, yes — but the bundle must be
self-contained (esbuild inlines everything). The V8 sandbox has no `node_modules`
and no module resolver. Keep deps minimal; they add to bundle size and attack
surface.

**How do I test against real platform data?** Export a character from your VTT,
save it as JSON, and use it as a fixture in your test file. The shared fixtures
in `examples/fixtures/` cover PF2e and SF2e characters at various levels.

**What if my platform can't represent a canonical field?** Skip it in `export()`
(add to `skipped`), and don't include it in `fieldCoverage`. The UI will show
players exactly what's covered and what's dropped. Honesty over false coverage.

**What about session state (HP, conditions)?** Session state is never synced.
If your platform exposes it, you can declare `supportsSessionRead: true` and
implement session telemetry separately — but it never enters the canonical build
or the merge engine. Write max HP only, never current/temp HP.
