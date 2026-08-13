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

Every fixture deserializes into `CharacterBuild` and passes its system plugin's
`validate`: PF2e fixtures against `schema-pf2e` (tested in
`crates/schema-pf2e/tests/fixtures.rs`), the SF2e fixture against `schema-sf2e`
(tested in `crates/schema-sf2e/tests/fixtures.rs`, §2.4). The SF2e fixture also
asserts sane derived stats from the shared chassis.

The SF2e fixture additionally round-trips through the Foundry adapter contract
harness (`adapters/foundry/src/index.test.ts`, §2.4): drones (chassis + mods),
vehicles (role), and credits/UPB survive `import(export(build))` within the
declared `fieldCoverage`.
