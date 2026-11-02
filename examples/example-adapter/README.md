# Example Adapter — Clone-Rename-Implement-Ship Template

A minimal adapter that passes the contract harness out of the box. Use this as
the starting point for a new platform adapter.

## Quick start

```bash
# From the repo root:
pnpm install
pnpm test --filter @nexus-ttrpg/example-adapter
```

All contract tests should pass.

## Clone to create a new adapter

```bash
cp -r examples/example-adapter adapters/my-platform
```

Then edit `adapters/my-platform/package.json`:
- `"name"`: `"@nexus-ttrpg/my-platform-adapter"`

Edit `adapters/my-platform/src/index.ts`:
- Change `id` from `"example"` to `"my-platform"`
- Change `ExamplePlatformPayload` to your platform's payload type
- Rewrite the `import` / `export` / `detectChanges` / `validate` methods
- Update `FIELD_COVERAGE` to declare the canonical paths you round-trip

Edit `adapters/my-platform/src/bundle.ts`:
- Change the import to match your renamed adapter

Edit `adapters/my-platform/src/index.test.ts`:
- Change the import to match your renamed adapter

## What each file does

| File | Purpose |
|------|---------|
| `src/index.ts` | The `CharacterAdapter<ExamplePlatformPayload>` implementation |
| `src/bundle.ts` | esbuild entry — exports `adapter` (the host appends `globalThis.__nexusAdapter = adapter;`) |
| `src/index.test.ts` | Contract tests via `runContractTests()` from `@nexus-ttrpg/adapter-sdk/testing` |
| `package.json` | Scripts: `build` (tsc), `bundle` (esbuild ESM), `test` (vitest) |
| `tsconfig.json` | Extends `../../tsconfig.base.json` |

## The adapter contract

Your adapter must implement `CharacterAdapter<TPlatform>`:

1. **`id`** — unique platform identifier (`"foundry"`, `"roll20"`, `"my-platform"`)
2. **`capabilities.fieldCoverage`** — canonical paths you round-trip (compile-time-checked via `canonicalPath()`)
3. **`import(raw, ctx)`** — platform payload → canonical `PartialBuild`. Never throws on unknown content (homebrew → warnings)
4. **`export(build, ctx)`** — canonical `CharacterBuild` → platform payload
5. **`detectChanges(previous, current)`** — cheap structural diff (no full import/merge)
6. **`validate(raw)`** — structural validation; may throw `AdapterError` for `unsupportedSheetVersion`

## The loop (no Rust required)

```bash
pnpm install
pnpm test --filter @nexus-ttrpg/my-platform-adapter
# iterate on src/index.ts
pnpm --filter @nexus-ttrpg/my-platform-adapter bundle
# → dist/bundle/bundle.js (single-file ESM, loaded by the V8 sandbox)
```

See `docs/adapter-authoring-guide.md` for the full guide.
