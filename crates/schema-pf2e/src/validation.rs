//! PF2e semantic validation (§3.1, §10.2).
//!
//! This is the second validation layer. `serde` + `deny_unknown_fields` already
//! enforce structural validity by construction (if you have a `CharacterBuild`,
//! it deserialized cleanly). This module enforces cross-field and ruleset rules
//! the type system can't express.
//!
//! Philosophy (§5.2): unknown/homebrew content is a **warning**, never an error.
//! Errors are for hard ruleset violations — feat level above character level,
//! key ability missing, class HP zero, etc.

use schema_core::character::CharacterBuild;
use schema_core::newtypes::FieldPath;
use schema_core::{ValidationError, ValidationReport, ValidationWarning};

/// Validate a PF2e `CharacterBuild` (§10.2).
pub fn validate(build: &CharacterBuild) -> ValidationReport {
    let mut report = ValidationReport::default();

    validate_class(build, &mut report);
    validate_ability_scores(build, &mut report);
    validate_feats(build, &mut report);
    validate_class_features(build, &mut report);
    validate_spellcasting(build, &mut report);

    report
}

// ── Class ──────────────────────────────────────────────────────────────────

fn validate_class(build: &CharacterBuild, report: &mut ValidationReport) {
    // Class HP per level must be positive.
    if build.class.hp_per_level == 0 {
        report.push_error(ValidationError {
            path: FieldPath::new("build.class.hpPerLevel"),
            message: "class hpPerLevel must be greater than 0".into(),
        });
    }

    // Key ability must be present in the ability_scores map.
    if !build.ability_scores.contains_key(&build.class.key_ability) {
        report.push_error(ValidationError {
            path: FieldPath::new("build.class.keyAbility"),
            message: format!(
                "key ability {:?} is not present in build.abilityScores",
                build.class.key_ability
            ),
        });
    }
}

// ── Ability scores ─────────────────────────────────────────────────────────

fn validate_ability_scores(build: &CharacterBuild, report: &mut ValidationReport) {
    // At least one ability score must be present.
    if build.ability_scores.is_empty() {
        report.push_error(ValidationError {
            path: FieldPath::new("build.abilityScores"),
            message: "abilityScores must not be empty".into(),
        });
    }

    // Each ability score's final value must be >= 1.
    for (key, entry) in &build.ability_scores {
        let boosts = entry.boosts.len() as i16;
        let flaws = entry.flaws.len() as i16;
        let item_bonus = entry.item_bonus as i16;
        let final_score = 10 + 2 * boosts - 2 * flaws + item_bonus;

        if final_score < 1 {
            report.push_error(ValidationError {
                path: FieldPath::new(format!("build.abilityScores.{key:?}")),
                message: format!("ability score {key:?} computes to {final_score}, must be >= 1"),
            });
        }
    }
}

// ── Feats ──────────────────────────────────────────────────────────────────

fn validate_feats(build: &CharacterBuild, report: &mut ValidationReport) {
    let char_level = build.level.get();

    for (i, feat) in build.feats.iter().enumerate() {
        if feat.level.get() > char_level {
            report.push_error(ValidationError {
                path: FieldPath::new(format!("build.feats[{i}].level")),
                message: format!(
                    "feat level {} exceeds character level {char_level}",
                    feat.level.get()
                ),
            });
        }
    }
}

// ── Class features ─────────────────────────────────────────────────────────

fn validate_class_features(build: &CharacterBuild, report: &mut ValidationReport) {
    let char_level = build.level.get();

    for (i, feature) in build.class_features.iter().enumerate() {
        if feature.level.get() > char_level {
            report.push_error(ValidationError {
                path: FieldPath::new(format!("build.classFeatures[{i}].level")),
                message: format!(
                    "class feature level {} exceeds character level {char_level}",
                    feature.level.get()
                ),
            });
        }
    }
}

// ── Spellcasting ───────────────────────────────────────────────────────────

fn validate_spellcasting(build: &CharacterBuild, report: &mut ValidationReport) {
    for (i, entry) in build.spellcasting.entries.iter().enumerate() {
        let base_path = format!("build.spellcasting.entries[{i}]");

        // Prepared/Spontaneous should have non-empty slots (warning, not error
        // — edge cases like a multiclass caster with no slots yet are legal).
        match entry.preparedness {
            schema_core::SpellPreparedness::Prepared
            | schema_core::SpellPreparedness::Spontaneous => {
                if entry.slots.is_empty() {
                    report.push_warning(ValidationWarning {
                        path: FieldPath::new(format!("{base_path}.slots")),
                        message: format!(
                            "{:?} spellcasting entry has no spell slots",
                            entry.preparedness
                        ),
                    });
                }
            }
            schema_core::SpellPreparedness::Innate => {
                // Innate spellcasting should not have a slot table. If it does,
                // warn — the data is preserved but likely a mapping error.
                if !entry.slots.is_empty() {
                    report.push_warning(ValidationWarning {
                        path: FieldPath::new(format!("{base_path}.slots")),
                        message:
                            "innate spellcasting entry has spell slots (usually empty for innate)"
                                .into(),
                    });
                }
            }
        }

        // Slot ranks must be 1..=10.
        for (rank, count) in &entry.slots {
            if !(*rank >= 1 && *rank <= 10) {
                report.push_error(ValidationError {
                    path: FieldPath::new(format!("{base_path}.slots.rank{rank}")),
                    message: format!("spell slot rank {rank} is out of range (1..=10)"),
                });
            }
            if *count > 4 {
                report.push_warning(ValidationWarning {
                    path: FieldPath::new(format!("{base_path}.slots.rank{rank}")),
                    message: format!("spell slot count {count} is unusually high (typically 1–4)"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::ability::AbilityScoreKey;
    use schema_core::build_types::*;
    use schema_core::newtypes::Level;
    use schema_core::refs::SystemRefId;
    use schema_core::testing::minimal_build;
    use std::collections::BTreeMap;

    #[test]
    fn minimal_build_is_valid() {
        let build = minimal_build();
        let report = validate(&build);
        assert!(
            report.is_valid(),
            "expected minimal build to be valid, got errors: {:?}",
            report.errors
        );
    }

    #[test]
    fn rejects_feat_level_above_character_level() {
        let mut build = minimal_build();
        build.feats.push(FeatEntry {
            reference: SystemRefId::from_slug("power-attack"),
            category: FeatCategory::Class,
            level: Level::new(5).unwrap(),
        });
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.feats[0].level")));
    }

    #[test]
    fn rejects_key_ability_not_in_ability_scores() {
        let mut build = minimal_build();
        // minimal_build has STR in ability_scores; switch key to INT.
        build.class.key_ability = AbilityScoreKey::Int;
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.class.keyAbility")));
    }

    #[test]
    fn rejects_zero_class_hp() {
        let mut build = minimal_build();
        build.class.hp_per_level = 0;
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.class.hpPerLevel")));
    }

    #[test]
    fn rejects_empty_ability_scores() {
        let mut build = minimal_build();
        build.ability_scores.clear();
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.abilityScores")));
    }

    #[test]
    fn warns_on_prepared_spellcasting_without_slots() {
        let mut build = minimal_build();
        build.spellcasting.entries.push(SpellcastingEntry {
            tradition: MagicTradition::Arcane,
            preparedness: SpellPreparedness::Prepared,
            attribute: AbilityScoreKey::Int,
            slots: BTreeMap::new(),
            spells: Vec::new(),
        });
        let report = validate(&build);
        assert!(report.is_valid(), "should be valid (warning, not error)");
        assert!(!report.warnings.is_empty(), "should have a warning");
    }

    #[test]
    fn warns_on_innate_spellcasting_with_slots() {
        let mut build = minimal_build();
        let mut slots = BTreeMap::new();
        slots.insert(1, 2);
        build.spellcasting.entries.push(SpellcastingEntry {
            tradition: MagicTradition::Divine,
            preparedness: SpellPreparedness::Innate,
            attribute: AbilityScoreKey::Cha,
            slots,
            spells: Vec::new(),
        });
        let report = validate(&build);
        assert!(report.is_valid());
        assert!(!report.warnings.is_empty());
    }

    #[test]
    fn rejects_spell_slot_rank_out_of_range() {
        let mut build = minimal_build();
        let mut slots = BTreeMap::new();
        slots.insert(11, 2); // rank 11 is invalid
        build.spellcasting.entries.push(SpellcastingEntry {
            tradition: MagicTradition::Arcane,
            preparedness: SpellPreparedness::Prepared,
            attribute: AbilityScoreKey::Int,
            slots,
            spells: Vec::new(),
        });
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("rank11")));
    }

    #[test]
    fn rejects_class_feature_level_above_character_level() {
        let mut build = minimal_build();
        build.class_features.push(ClassFeatureEntry {
            reference: SystemRefId::from_slug("weapon-mastery"),
            level: Level::new(10).unwrap(),
        });
        let report = validate(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.classFeatures[0].level")));
    }
}
