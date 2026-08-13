//! Migration test matrix (§16, §9.3).
//!
//! Every v0.0.1 fixture must:
//! 1. Fail to deserialize into the current `CharacterBuild` without migration
//!    (proving the migration is actually needed).
//! 2. Migrate successfully through the `MigrationRegistry`.
//! 3. Deserialize into the current `CharacterBuild` after migration.
//! 4. Produce a build identical to the corresponding v0.1.0 fixture (proving
//!    the migration is lossless).

use schema_core::{CharacterBuild, MigrationRegistry, CURRENT_SCHEMA_VERSION};
use semver::Version;
use serde_json::Value;

const V001_FIGHTER: &str =
    include_str!("../../../examples/fixtures/v0.0.1/pf2e-level1-fighter.json");
const V001_WIZARD: &str =
    include_str!("../../../examples/fixtures/v0.0.1/pf2e-level10-wizard.json");
const V001_RANGER: &str = include_str!("../../../examples/fixtures/v0.0.1/pf2e-level5-ranger.json");
const V001_MECHANIC: &str =
    include_str!("../../../examples/fixtures/v0.0.1/sf2e-level5-mechanic.json");

const V010_FIGHTER: &str = include_str!("../../../examples/fixtures/pf2e-level1-fighter.json");
const V010_WIZARD: &str = include_str!("../../../examples/fixtures/pf2e-level10-wizard.json");
const V010_RANGER: &str = include_str!("../../../examples/fixtures/pf2e-level5-ranger.json");
const V010_MECHANIC: &str = include_str!("../../../examples/fixtures/sf2e-level5-mechanic.json");

const V001: Version = Version::new(0, 0, 1);

/// Migrate a v0.0.1 JSON value to current and deserialize into CharacterBuild.
fn migrate_and_parse(v001_json: &str) -> CharacterBuild {
    let value: Value = serde_json::from_str(v001_json).expect("parse v0.0.1 JSON");
    let registry = MigrationRegistry::default();
    let migrated = registry
        .migrate_to_current(&V001, value)
        .expect("migration succeeds");
    serde_json::from_value(migrated).expect("deserialize migrated build")
}

/// Assert that raw v0.0.1 JSON fails to deserialize without migration.
fn assert_needs_migration(v001_json: &str) {
    let result: Result<CharacterBuild, _> = serde_json::from_str(v001_json);
    assert!(
        result.is_err(),
        "v0.0.1 fixture should NOT deserialize without migration"
    );
}

#[test]
fn fighter_v001_fails_without_migration() {
    assert_needs_migration(V001_FIGHTER);
}

#[test]
fn wizard_v001_fails_without_migration() {
    assert_needs_migration(V001_WIZARD);
}

#[test]
fn ranger_v001_fails_without_migration() {
    assert_needs_migration(V001_RANGER);
}

#[test]
fn mechanic_v001_fails_without_migration() {
    assert_needs_migration(V001_MECHANIC);
}

#[test]
fn fighter_migrates_and_matches_v010() {
    let migrated = migrate_and_parse(V001_FIGHTER);
    let expected: CharacterBuild =
        serde_json::from_str(V010_FIGHTER).expect("parse v0.1.0 fixture");
    assert_eq!(migrated, expected);
    assert_eq!(migrated.identity.name, "Aldric Ashford");
    assert_eq!(migrated.level.get(), 1);
}

#[test]
fn wizard_migrates_and_matches_v010() {
    let migrated = migrate_and_parse(V001_WIZARD);
    let expected: CharacterBuild = serde_json::from_str(V010_WIZARD).expect("parse v0.1.0 fixture");
    assert_eq!(migrated, expected);
    assert_eq!(migrated.identity.name, "Zara Morningstar");
    assert_eq!(migrated.level.get(), 10);
}

#[test]
fn ranger_migrates_and_matches_v010() {
    let migrated = migrate_and_parse(V001_RANGER);
    let expected: CharacterBuild = serde_json::from_str(V010_RANGER).expect("parse v0.1.0 fixture");
    assert_eq!(migrated, expected);
    assert_eq!(migrated.identity.name, "Thalia Greenleaf");
    assert_eq!(migrated.level.get(), 5);
}

#[test]
fn mechanic_migrates_and_matches_v010() {
    let migrated = migrate_and_parse(V001_MECHANIC);
    let expected: CharacterBuild =
        serde_json::from_str(V010_MECHANIC).expect("parse v0.1.0 fixture");
    assert_eq!(migrated, expected);
    assert_eq!(migrated.identity.name, "Vex Quark");
    assert_eq!(migrated.level.get(), 5);
}

#[test]
fn migrated_builds_have_current_content_hash() {
    let migrated = migrate_and_parse(V001_FIGHTER);
    let expected: CharacterBuild =
        serde_json::from_str(V010_FIGHTER).expect("parse v0.1.0 fixture");
    assert_eq!(
        schema_core::content_hash(&migrated),
        schema_core::content_hash(&expected),
        "content hash should match after migration"
    );
}

#[test]
fn current_version_fixtures_need_no_migration() {
    let value: Value = serde_json::from_str(V010_FIGHTER).expect("parse v0.1.0 JSON");
    let registry = MigrationRegistry::default();
    let migrated = registry
        .migrate_to_current(&CURRENT_SCHEMA_VERSION, value.clone())
        .expect("noop migration");
    assert_eq!(migrated, value, "current version should be a no-op");
}
