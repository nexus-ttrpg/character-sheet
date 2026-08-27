# @nexus-sheet/pathbuilder-adapter

The first **third-party-builder** adapter (architecture §8). Pathbuilder 2e is a
PF2e character *builder* (not a VTT): a standalone app where a player constructs
a character and exports it as JSON or a share code. Its export is "already close
to canonical's shape, so mapping is mostly renaming" (§8).

## Import-only (§8)

Pathbuilder has **no programmatic write-back API**, so `supportsExport` is
`false` — the engine won't offer a push to Pathbuilder. `export()` is still
implemented: it renders a Pathbuilder JSON a user can re-import by hand, and it
satisfies the round-trip contract test. Most third-party builders are import
sources, not export targets (§8).

## Non-lossy ability scores (§197)

Unlike the Roll20 adapter (which stores only the derived number and loses
boosts/flaws), Pathbuilder recomputes the final ability number itself — and
because the canonical model stores **components** (boosts/flaws/itemBonus), the
round-trip here is **non-lossy**: `boosts`, `flaws`, and `itemBonus` survive
`export → import` exactly. This is asserted in `src/index.test.ts`.

## ⚠️ Format fidelity caveat

The field names in `src/mapping.ts` are a **representative** Pathbuilder JSON
shape modelled on §8 and §197 — they are **not verified against a real
Pathbuilder 2e export**. The contract tests validate import/export *symmetry*
(round-trip), not real-format fidelity. Before importing live Pathbuilder data,
remap these field names against a real export; the canary job (§3.3) is the
mechanism that catches real-format drift.

## Commands

```
pnpm test                                   # contract + pathbuilder-specific tests
pnpm --filter @nexus-sheet/pathbuilder-adapter bundle   # esbuild single-file ESM
```

See `docs/adapter-authoring-guide.md` for the contribution workflow.
