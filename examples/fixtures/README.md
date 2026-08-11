# Shared JSON ground-truth characters (§16)

One set of canonical characters, consumed by both Rust and TypeScript as plain
JSON. Each fixture is a `CharacterBuild` (the `build` partition only — no `meta`,
`session`, or `extensions`).

## Files

| File | System | Tier | Level | Description |
|------|--------|------|-------|-------------|
| `pf2e-level1-fighter.json` | PF2e | fresh | 1 | Human fighter — minimal complexity |
| `pf2e-level10-wizard.json` | PF2e | complex | 10 | Elf wizard — spellcasting, feats, lore skills |
| `pf2e-level5-ranger.json` | PF2e | companion-heavy | 5 | Elf ranger — animal companion, familiar |
| `sf2e-level5-mechanic.json` | SF2e | drone-heavy | 5 | Human mechanic — drone, credits/UPB, vehicle |
| `manifest.json` | — | — | — | Maps each fixture to system, tier, level |

## Verify gate (§0.7)

Every fixture deserializes into `CharacterBuild` and (for PF2e) passes
`schema-pf2e`'s `validate`. Tested in `crates/schema-pf2e/tests/fixtures.rs`.

The SF2e fixture deserializes but is not yet validated against a system plugin
(`schema-sf2e` is a scaffold; lands in a later phase).
