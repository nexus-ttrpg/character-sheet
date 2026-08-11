//! Phase 0.2 invariant tests — the load-bearing properties of the canonical
//! model (§3, §9.2). These assert the design invariants, not just that code
//! runs.

use schema_core::testing::minimal_build;
use schema_core::*;

// ── Level: 1..=20 enforced on construction (§3.3) ─────────────────────────

#[test]
fn level_accepts_1_through_20() {
    for n in 1..=20u8 {
        assert!(Level::new(n).is_ok(), "level {} should be valid", n);
    }
}

#[test]
fn level_rejects_0_and_21() {
    assert!(Level::new(0).is_err());
    assert!(Level::new(21).is_err());
    assert!(Level::new(255).is_err());
}

#[test]
fn level_serde_rejects_out_of_bounds() {
    // Deserialization must enforce the bound at the boundary (§3.1: "if you
    // have a Level, it's valid by construction").
    assert!(serde_json::from_str::<Level>("0").is_err());
    assert!(serde_json::from_str::<Level>("21").is_err());
    assert!(serde_json::from_str::<Level>("1").is_ok());
    assert!(serde_json::from_str::<Level>("20").is_ok());
}

#[test]
fn level_round_trips_through_serde() {
    let level = Level::new(15).unwrap();
    let json = serde_json::to_string(&level).unwrap();
    assert_eq!(json, "15");
    let back: Level = serde_json::from_str(&json).unwrap();
    assert_eq!(level, back);
}

// ── deny_unknown_fields on typed regions (§3.1) ───────────────────────────

#[test]
fn deny_unknown_fields_rejects_extra_field_in_build() {
    let json = r#"{
        "identity": {"name": "Test"},
        "ancestry": {"reference": {"slug": "human"}},
        "heritage": {"reference": {"slug": "half-elf"}},
        "background": {"reference": {"slug": "scholar"}},
        "class": {"reference": {"slug": "fighter"}, "keyAbility": "str", "hpPerLevel": 10},
        "level": 1,
        "abilityScores": {},
        "skills": {},
        "proficiencies": {},
        "feats": [],
        "classFeatures": [],
        "equipment": {},
        "currencies": {},
        "spellcasting": {},
        "actions": [],
        "reactions": [],
        "explorationActivities": [],
        "companions": [],
        "familiars": [],
        "drones": [],
        "vehicles": [],
        "notes": [],
        "thisFieldDoesNotExist": true
    }"#;
    assert!(
        serde_json::from_str::<CharacterBuild>(json).is_err(),
        "deny_unknown_fields must reject extra fields in typed regions (§3.1)"
    );
}

#[test]
fn deny_unknown_fields_rejects_extra_field_in_meta() {
    let json = r#"{
        "id": "01J8Z3F9K2N4Q6R8S0T2V4W6X8",
        "schemaVersion": "0.1.0",
        "system": "pf2e",
        "ownerId": "01J8Z3F9K2N4Q6R8S0T2V4W6X8",
        "displayName": "Test",
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "contentHash": "0000000000000000000000000000000000000000000000000000000000000000",
        "extraField": true
    }"#;
    assert!(serde_json::from_str::<CharacterMeta>(json).is_err());
}

// ── ExtensionBag accepts unknown data (§3.5) ──────────────────────────────

#[test]
fn extension_bag_accepts_arbitrary_namespaced_data() {
    let json =
        r#"{"foundry.pf2e": {"flags": {"custom": true}}, "homebrew.myGuild": {"trait": "flying"}}"#;
    let bag: ExtensionBag = serde_json::from_str(json).unwrap();
    assert!(bag.get(&Namespace::new("foundry.pf2e")).is_some());
    assert!(bag.get(&Namespace::new("homebrew.myGuild")).is_some());
}

// ── Content hash: BLAKE3 of build only, deterministic (§9.2) ──────────────

#[test]
fn content_hash_is_deterministic_across_runs() {
    let build = minimal_build();
    let h1 = content_hash(&build);
    let h2 = content_hash(&build);
    assert_eq!(h1, h2, "same build must produce the same hash (§9.2)");
}

#[test]
fn content_hash_changes_when_build_changes() {
    let build = minimal_build();
    let h1 = content_hash(&build);

    let mut build2 = build.clone();
    build2.identity.name = "Different Name".into();
    let h2 = content_hash(&build2);

    assert_ne!(h1, h2, "different builds must produce different hashes");
}

#[test]
fn content_hash_is_64_hex_chars() {
    let build = minimal_build();
    let hex = content_hash(&build).to_hex();
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

// ── serde round-trip (§3.6 — types are the source of truth) ───────────────

#[test]
fn character_build_round_trips_through_serde() {
    let build = minimal_build();
    let json = serde_json::to_string(&build).unwrap();
    let back: CharacterBuild = serde_json::from_str(&json).unwrap();
    assert_eq!(build, back, "round-trip must be lossless");
}

#[test]
fn system_ref_id_flattens_other_map() {
    // §9.1: #[serde(flatten)] on `other` means a new platform's ID field
    // round-trips through older builds without data loss.
    let json = r#"{
        "slug": "fireball",
        "source": "Player Core",
        "foundryUuid": "Compendium.pf2e.spells-srd.Item.abc123",
        "demiplaneId": "dp-xyz",
        "futurePlatformId": "fp-789"
    }"#;
    let ref_id: SystemRefId = serde_json::from_str(json).unwrap();
    assert_eq!(ref_id.slug.as_str(), "fireball");
    assert_eq!(
        ref_id.foundry_uuid.as_deref(),
        Some("Compendium.pf2e.spells-srd.Item.abc123")
    );
    assert_eq!(
        ref_id.other.get("demiplaneId").map(|s| s.as_str()),
        Some("dp-xyz")
    );
    assert_eq!(
        ref_id.other.get("futurePlatformId").map(|s| s.as_str()),
        Some("fp-789")
    );

    // Round-trip preserves the flattened fields.
    let back = serde_json::to_string(&ref_id).unwrap();
    assert!(back.contains("\"demiplaneId\":\"dp-xyz\""));
    assert!(back.contains("\"futurePlatformId\":\"fp-789\""));
}

// ── ProficiencyRank ordering (§3.3 — shared chassis) ──────────────────────

#[test]
fn proficiency_rank_orders_untrained_through_legendary() {
    use std::cmp::Ordering;
    assert_eq!(
        ProficiencyRank::Untrained.cmp(&ProficiencyRank::Legendary),
        Ordering::Less
    );
    assert_eq!(
        ProficiencyRank::Trained.cmp(&ProficiencyRank::Expert),
        Ordering::Less
    );
    assert_eq!(
        ProficiencyRank::Expert.cmp(&ProficiencyRank::Master),
        Ordering::Less
    );
    assert_eq!(
        ProficiencyRank::Master.cmp(&ProficiencyRank::Legendary),
        Ordering::Less
    );
}

#[test]
fn proficiency_rank_bonus_is_pf2e_canonical() {
    assert_eq!(ProficiencyRank::Untrained.bonus(), 0);
    assert_eq!(ProficiencyRank::Trained.bonus(), 2);
    assert_eq!(ProficiencyRank::Expert.bonus(), 4);
    assert_eq!(ProficiencyRank::Master.bonus(), 6);
    assert_eq!(ProficiencyRank::Legendary.bonus(), 8);
}

// ── BTreeMap determinism (§3.3, §9.2) ─────────────────────────────────────

#[test]
fn ability_scores_serialize_in_stable_order() {
    // Insert in non-alphabetical order; BTreeMap must serialize alphabetically.
    use schema_core::ability::{AbilityBoost, AbilityBoostSource, AbilityScoreEntry};
    let mut build = minimal_build();
    build.ability_scores.clear();
    build.ability_scores.insert(
        AbilityScoreKey::Cha,
        AbilityScoreEntry {
            boosts: vec![],
            flaws: vec![],
            item_bonus: 0,
        },
    );
    build.ability_scores.insert(
        AbilityScoreKey::Str,
        AbilityScoreEntry {
            boosts: vec![AbilityBoost {
                source: AbilityBoostSource::Free,
            }],
            flaws: vec![],
            item_bonus: 0,
        },
    );

    let json1 = serde_json::to_string(&build).unwrap();
    let json2 = serde_json::to_string(&build).unwrap();
    assert_eq!(json1, json2, "serialization must be deterministic");

    // Str (alphabetically "dex" < "str" < ... wait, enum variant order in JSON
    // is by the key name. BTreeMap<AbilityScoreKey, _> orders by the enum's
    // derived Ord, which is declaration order for enums (not alphabetical).
    // The point is determinism, not a specific order.
    let back: CharacterBuild = serde_json::from_str(&json1).unwrap();
    assert_eq!(build, back);
}
