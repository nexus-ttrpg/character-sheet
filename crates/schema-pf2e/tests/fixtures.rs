//! Fixture verification (§16, §0.7).
//!
//! Every PF2e fixture in `examples/fixtures/` must:
//! 1. Deserialize into `CharacterBuild` (structural validity by construction).
//! 2. Pass `schema-pf2e`'s `validate` (semantic validity for PF2e fixtures).
//!
//! The SF2e fixture is validated by the parallel suite in `schema-sf2e`'s
//! `tests/fixtures.rs` against the `Sf2ePlugin` (§2.4). The shared fixture
//! manifest is asserted here since it spans both systems.

use schema_core::CharacterBuild;
use schema_core::SystemPlugin;
use schema_pf2e::Pf2ePlugin;

// ── PF2e fixtures: deserialize + validate ───────────────────────────────────

const PF2E_LEVEL1_FIGHTER: &str =
    include_str!("../../../examples/fixtures/pf2e-level1-fighter.json");
const PF2E_LEVEL10_WIZARD: &str =
    include_str!("../../../examples/fixtures/pf2e-level10-wizard.json");
const PF2E_LEVEL5_RANGER: &str = include_str!("../../../examples/fixtures/pf2e-level5-ranger.json");

// ── Manifest ────────────────────────────────────────────────────────────────

const MANIFEST: &str = include_str!("../../../examples/fixtures/manifest.json");

#[derive(serde::Deserialize)]
struct Manifest {
    fixtures: Vec<ManifestFixture>,
}

#[derive(serde::Deserialize)]
struct ManifestFixture {
    file: String,
    system: String,
    level: u8,
}

#[test]
fn manifest_lists_all_fixtures() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).expect("manifest parses");
    assert_eq!(
        manifest.fixtures.len(),
        4,
        "expected 4 fixtures in manifest"
    );

    let systems: Vec<&str> = manifest
        .fixtures
        .iter()
        .map(|f| f.system.as_str())
        .collect();
    assert!(systems.contains(&"pf2e"), "manifest has PF2e fixtures");
    assert!(systems.contains(&"sf2e"), "manifest has SF2e fixture");
}

#[test]
fn manifest_fixture_levels_are_valid() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).expect("manifest parses");
    for f in &manifest.fixtures {
        assert!(
            f.level >= 1 && f.level <= 20,
            "fixture {} has invalid level {}",
            f.file,
            f.level
        );
    }
}

// ── PF2e: deserialize + validate ────────────────────────────────────────────

fn deserialize_pf2e(json: &str, name: &str) -> CharacterBuild {
    let build: CharacterBuild = serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("{name}: failed to deserialize into CharacterBuild: {e}"));
    build
}

fn validate_pf2e(build: &CharacterBuild, name: &str) {
    let plugin = Pf2ePlugin::new();
    let report = plugin.validate(build);
    assert!(
        report.is_valid(),
        "{name}: validate returned errors: {:?}",
        report.errors
    );
}

#[test]
fn pf2e_level1_fighter_deserializes_and_validates() {
    let build = deserialize_pf2e(PF2E_LEVEL1_FIGHTER, "pf2e-level1-fighter");
    validate_pf2e(&build, "pf2e-level1-fighter");

    // Spot-check key fields.
    assert_eq!(build.identity.name, "Aldric Ashford");
    assert_eq!(build.level.get(), 1);
    assert_eq!(build.class.reference.slug.as_str(), "fighter");
    assert!(!build.ability_scores.is_empty());
    assert!(!build.feats.is_empty());
}

#[test]
fn pf2e_level10_wizard_deserializes_and_validates() {
    let build = deserialize_pf2e(PF2E_LEVEL10_WIZARD, "pf2e-level10-wizard");
    validate_pf2e(&build, "pf2e-level10-wizard");

    assert_eq!(build.identity.name, "Zara Morningstar");
    assert_eq!(build.level.get(), 10);
    assert_eq!(build.class.reference.slug.as_str(), "wizard");
    assert!(
        !build.spellcasting.entries.is_empty(),
        "wizard should have spellcasting"
    );

    // Spellcasting entry has slots (Prepared).
    let sc = &build.spellcasting.entries[0];
    assert!(!sc.slots.is_empty(), "prepared caster should have slots");
    assert_eq!(sc.spells.len(), 8, "expected 8 spells");
}

#[test]
fn pf2e_level5_ranger_deserializes_and_validates() {
    let build = deserialize_pf2e(PF2E_LEVEL5_RANGER, "pf2e-level5-ranger");
    validate_pf2e(&build, "pf2e-level5-ranger");

    assert_eq!(build.identity.name, "Thalia Greenleaf");
    assert_eq!(build.level.get(), 5);
    assert_eq!(build.class.reference.slug.as_str(), "ranger");
    assert!(
        !build.companions.is_empty(),
        "ranger should have a companion"
    );
    assert!(!build.familiars.is_empty(), "ranger should have a familiar");
}

#[test]
fn pf2e_level10_wizard_derived_stats() {
    let build = deserialize_pf2e(PF2E_LEVEL10_WIZARD, "pf2e-level10-wizard");
    let plugin = Pf2ePlugin::new();
    let stats = plugin.derived_stats(&build);

    // INT: 10 + 2×5 boosts = 20, modifier +5
    use schema_core::ability::AbilityScoreKey;
    assert_eq!(stats.ability_scores[&AbilityScoreKey::Int], 20);
    assert_eq!(stats.ability_modifiers[&AbilityScoreKey::Int], 5);

    // Max HP: wizard 6 HP/level × 10 + elf ancestry 6 = 66
    assert_eq!(stats.max_hp, 66);

    // Class DC: level 10 + expert(4) + INT mod 5 = 19
    assert_eq!(stats.class_dc, 19);
}

// ── Content hash stability (§9.2) ───────────────────────────────────────────

#[test]
fn pf2e_level1_fighter_content_hash_is_stable() {
    let build = deserialize_pf2e(PF2E_LEVEL1_FIGHTER, "pf2e-level1-fighter");
    let hash1 = schema_core::content_hash(&build);
    let hash2 = schema_core::content_hash(&build);
    assert_eq!(hash1, hash2, "content hash must be deterministic");

    // 64 hex chars (32 bytes BLAKE3).
    assert_eq!(hash1.to_hex().len(), 64);
}
