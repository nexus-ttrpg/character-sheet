//! SF2e fixture verification (§16, §0.7, §2.4).
//!
//! The SF2e fixture must:
//! 1. Deserialize into `CharacterBuild` (structural validity by construction).
//! 2. Pass `schema-sf2e`'s `validate` (semantic validity).
//! 3. Produce sane derived stats from the shared chassis (§3.4, §10.2).
//!
//! This is the parallel of `schema-pf2e`'s fixture suite for the SF2e side.

use schema_core::ability::AbilityScoreKey;
use schema_core::{CharacterBuild, SystemPlugin};
use schema_sf2e::Sf2ePlugin;

const SF2E_LEVEL5_MECHANIC: &str =
    include_str!("../../../examples/fixtures/sf2e-level5-mechanic.json");

fn deserialize(json: &str) -> CharacterBuild {
    serde_json::from_str(json).unwrap_or_else(|e| {
        panic!("sf2e-level5-mechanic: failed to deserialize into CharacterBuild: {e}")
    })
}

#[test]
fn sf2e_level5_mechanic_deserializes_and_validates() {
    let build = deserialize(SF2E_LEVEL5_MECHANIC);
    let plugin = Sf2ePlugin::new();
    let report = plugin.validate(&build);
    assert!(
        report.is_valid(),
        "sf2e-level5-mechanic: validate returned errors: {:?}",
        report.errors
    );

    // Spot-check identity + class.
    assert_eq!(build.identity.name, "Vex Quark");
    assert_eq!(build.level.get(), 5);
    assert_eq!(build.class.reference.slug.as_str(), "mechanic");

    // Exercises §3.4 shapes: drones, credits/UPB, vehicles.
    assert!(!build.drones.is_empty(), "mechanic should have a drone");
    assert_eq!(build.drones.len(), 1);

    let drone = &build.drones[0];
    assert_eq!(drone.chassis.as_deref(), Some("combat"));
    assert_eq!(drone.mods.len(), 3, "drone should have 3 mods");

    assert!(
        build.currencies.credits.is_some(),
        "SF2e fixture should have credits"
    );
    assert!(
        build.currencies.upb.is_some(),
        "SF2e fixture should have UPB"
    );

    assert!(
        !build.vehicles.is_empty(),
        "SF2e fixture should have a vehicle"
    );
}

#[test]
fn sf2e_level5_mechanic_derived_stats() {
    let build = deserialize(SF2E_LEVEL5_MECHANIC);
    let plugin = Sf2ePlugin::new();
    let stats = plugin.derived_stats(&build);

    // INT: 10 + 2×4 boosts (ancestry, class, free, free) = 18, modifier +4.
    assert_eq!(stats.ability_scores[&AbilityScoreKey::Int], 18);
    assert_eq!(stats.ability_modifiers[&AbilityScoreKey::Int], 4);

    // DEX: 10 + 2×2 boosts (background, free) = 14, modifier +2.
    assert_eq!(stats.ability_scores[&AbilityScoreKey::Dex], 14);
    assert_eq!(stats.ability_modifiers[&AbilityScoreKey::Dex], 2);

    // Max HP: mechanic 6 HP/level × 5 + human ancestry 8 = 38.
    assert_eq!(stats.max_hp, 38);

    // Reflex: level 5 + expert(4) + DEX mod 2 = 11.
    assert_eq!(stats.reflex, 11);

    // Class DC: level 5 + trained(2) + key ability (INT) mod 4 = 11.
    assert_eq!(stats.class_dc, 11);
}
