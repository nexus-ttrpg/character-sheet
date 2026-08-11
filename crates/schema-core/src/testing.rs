//! Test support — minimal fixtures for unit and integration tests across
//! the project (§16). Mirrors the `@nexus-sheet/adapter-sdk/testing` pattern
//! on the TS side: a shared, minimal character build that downstream crates'
//! tests (sync-engine proptest, store round-trip, contract tests) can use
//! without each rebuilding one.
//!
//! NOT for production use — these fixtures are intentionally minimal.

use std::collections::BTreeMap;

use crate::ability::{
    AbilityBoost, AbilityBoostSource, AbilityScoreEntry, AbilityScoreKey, AbilityScoreMap,
};
use crate::build_types::*;
use crate::character::CharacterBuild;
use crate::newtypes::Level;
use crate::proficiency::ProficiencyTable;
use crate::refs::SystemRefId;

/// A minimal valid `CharacterBuild` for tests. Fuller fixtures (multiclassed
/// level 10, companion-heavy, drone-heavy SF2e) live in `examples/fixtures/`
/// (Phase 0.7).
pub fn minimal_build() -> CharacterBuild {
    let mut ability_scores = AbilityScoreMap::new();
    ability_scores.insert(
        AbilityScoreKey::Str,
        AbilityScoreEntry {
            boosts: vec![AbilityBoost {
                source: AbilityBoostSource::Ancestry,
            }],
            flaws: vec![],
            item_bonus: 0,
        },
    );

    CharacterBuild {
        identity: Identity {
            name: "Test Character".into(),
            player_name: None,
            pronouns: None,
        },
        ancestry: Ancestry {
            reference: SystemRefId::from_slug("human"),
            display_name: None,
        },
        heritage: Heritage {
            reference: SystemRefId::from_slug("half-elf"),
            display_name: None,
        },
        background: Background {
            reference: SystemRefId::from_slug("scholar"),
            display_name: None,
        },
        class: ClassEntry {
            reference: SystemRefId::from_slug("fighter"),
            key_ability: AbilityScoreKey::Str,
            hp_per_level: 10,
        },
        level: Level::new(1).expect("1 is valid"),
        ability_scores,
        skills: BTreeMap::new(),
        proficiencies: ProficiencyTable::default(),
        feats: Vec::new(),
        class_features: Vec::new(),
        equipment: Equipment::default(),
        currencies: Currencies::default(),
        spellcasting: Spellcasting::default(),
        actions: Vec::new(),
        reactions: Vec::new(),
        exploration_activities: Vec::new(),
        companions: Vec::new(),
        familiars: Vec::new(),
        drones: Vec::new(),
        vehicles: Vec::new(),
        notes: Vec::new(),
    }
}
