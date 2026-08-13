//! Shared PF2e/SF2e chassis — derived stats and validation common to both
//! systems (§3.4, §10.2).
//!
//! PF2e and SF2e share the same ability-score, proficiency, three-action, and
//! derived-stat chassis (§3.4: "SF2e reuses most of the PF2e chassis"). Rather
//! than duplicate the math in each system plugin — where it would silently drift
//! — the shared computation lives here in `schema-core`, which both
//! `schema-pf2e` and `schema-sf2e` depend on (they cannot depend on each other).
//!
//! A system plugin calls [`validate_chassis`] / [`derived_stats`] and may layer
//! system-specific rules on top; this module owns only the chassis. The
//! `DerivedStats` struct already documents the shared chassis (§10.2):
//! "PF2e and SF2e share the same derived-stat chassis."

use std::collections::BTreeMap;

use crate::ability::AbilityScoreKey;
use crate::character::CharacterBuild;
use crate::plugin::{proficiency_or_untrained, DerivedStats};
use crate::proficiency::ProficiencyRank;
use crate::{ContentIndex, ValidationError, ValidationReport, ValidationWarning};

// ── Derived stats (§3.3, §10.2) ─────────────────────────────────────────────

/// Compute the shared-chassis derived stats for a build (§3.4, §10.2).
///
/// Both PF2e and SF2e use the same formulas:
/// - Ability score: `10 + 2×boosts − 2×flaws + item_bonus`.
/// - Ability modifier: `(score − 10) / 2`, floored.
/// - Max HP: `class.hp_per_level × level + ancestry_hp` (§7.2).
/// - Perception/saves/class DC: `level + rank.bonus + ability_mod`, with the
///   Remaster rule that **untrained omits level** (just the ability modifier).
///
/// `ancestry_hp` is resolved from the system plugin's [`ContentIndex`] so each
/// system curates its own ancestry data without re-implementing the formula.
pub fn derived_stats(build: &CharacterBuild, content_index: &ContentIndex) -> DerivedStats {
    let level = build.level.get() as i16;

    // ── Final ability scores: 10 + 2×boosts − 2×flaws + item_bonus ───────
    let mut ability_scores = BTreeMap::new();
    let mut ability_modifiers = BTreeMap::new();

    for key in [
        AbilityScoreKey::Str,
        AbilityScoreKey::Dex,
        AbilityScoreKey::Con,
        AbilityScoreKey::Int,
        AbilityScoreKey::Wis,
        AbilityScoreKey::Cha,
    ] {
        let entry = build.ability_scores.get(&key);
        let boosts = entry.map(|e| e.boosts.len()).unwrap_or(0) as i16;
        let flaws = entry.map(|e| e.flaws.len()).unwrap_or(0) as i16;
        let item_bonus = entry.map(|e| e.item_bonus as i16).unwrap_or(0);

        let score = 10 + 2 * boosts - 2 * flaws + item_bonus;
        ability_scores.insert(key, score);
        ability_modifiers.insert(key, ability_modifier(score));
    }

    // ── Max HP: class.hp_per_level × level + ancestry_hp ─────────────────
    let ancestry_hp = content_index
        .get(&build.ancestry.reference.slug)
        .and_then(|e| e.hp)
        .map(|hp| hp as i16)
        .unwrap_or(0);

    let max_hp = build.class.hp_per_level as i16 * level + ancestry_hp;

    // ── Perception, saves, class DC ──────────────────────────────────────
    // Remaster: untrained checks do NOT add level; trained+ do.
    let wis_mod = ability_modifiers[&AbilityScoreKey::Wis];
    let con_mod = ability_modifiers[&AbilityScoreKey::Con];
    let dex_mod = ability_modifiers[&AbilityScoreKey::Dex];
    let key_mod = ability_modifiers[&build.class.key_ability];

    let perception = proficiency_modifier(
        level,
        proficiency_or_untrained(&build.proficiencies, "perception"),
        wis_mod,
    );
    let fortitude = proficiency_modifier(
        level,
        proficiency_or_untrained(&build.proficiencies, "fortitude"),
        con_mod,
    );
    let reflex = proficiency_modifier(
        level,
        proficiency_or_untrained(&build.proficiencies, "reflex"),
        dex_mod,
    );
    let will = proficiency_modifier(
        level,
        proficiency_or_untrained(&build.proficiencies, "will"),
        wis_mod,
    );

    // Class DC defaults to Trained — every PF2e/SF2e class grants it.
    let class_dc_rank = build
        .proficiencies
        .0
        .get(&crate::newtypes::SkillKey::new("class-dc"))
        .copied()
        .unwrap_or(ProficiencyRank::Trained);
    let class_dc = proficiency_modifier(level, class_dc_rank, key_mod);

    DerivedStats {
        ability_scores,
        ability_modifiers,
        max_hp,
        perception,
        fortitude,
        reflex,
        will,
        class_dc,
    }
}

/// Ability modifier: `(score − 10) / 2`, rounded down (floor, not truncation).
/// PF2e/SF2e round toward negative infinity, so score 7 → modifier −2, not −1.
pub fn ability_modifier(score: i16) -> i16 {
    (score - 10).div_euclid(2)
}

/// Remaster proficiency modifier: untrained does NOT add level (just ability
/// mod); trained+ adds `level + rank.bonus + ability_mod`.
pub fn proficiency_modifier(level: i16, rank: ProficiencyRank, ability_mod: i16) -> i16 {
    match rank {
        ProficiencyRank::Untrained => ability_mod,
        _ => level + rank.bonus() as i16 + ability_mod,
    }
}

// ── Validation (§3.1, §10.2) ───────────────────────────────────────────────

/// Validate the shared-chassis rules (§3.4, §10.2).
///
/// These cross-field rules apply to both PF2e and SF2e: class HP > 0, key
/// ability present in `abilityScores`, ability scores non-empty and each
/// computing to ≥ 1, feat and class-feature levels within character level, and
/// spellcasting-entry consistency. A system plugin calls this and may append
/// system-specific errors/warnings.
///
/// Philosophy (§5.2): unknown/homebrew content is a **warning**, never an
/// error. Errors are for hard ruleset violations only.
pub fn validate_chassis(build: &CharacterBuild) -> ValidationReport {
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
            path: crate::newtypes::FieldPath::new("build.class.hpPerLevel"),
            message: "class hpPerLevel must be greater than 0".into(),
        });
    }

    // Key ability must be present in the ability_scores map.
    if !build.ability_scores.contains_key(&build.class.key_ability) {
        report.push_error(ValidationError {
            path: crate::newtypes::FieldPath::new("build.class.keyAbility"),
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
            path: crate::newtypes::FieldPath::new("build.abilityScores"),
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
                path: crate::newtypes::FieldPath::new(format!("build.abilityScores.{key:?}")),
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
                path: crate::newtypes::FieldPath::new(format!("build.feats[{i}].level")),
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
                path: crate::newtypes::FieldPath::new(format!("build.classFeatures[{i}].level")),
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
            crate::SpellPreparedness::Prepared | crate::SpellPreparedness::Spontaneous => {
                if entry.slots.is_empty() {
                    report.push_warning(ValidationWarning {
                        path: crate::newtypes::FieldPath::new(format!("{base_path}.slots")),
                        message: format!(
                            "{:?} spellcasting entry has no spell slots",
                            entry.preparedness
                        ),
                    });
                }
            }
            crate::SpellPreparedness::Innate => {
                // Innate spellcasting should not have a slot table. If it does,
                // warn — the data is preserved but likely a mapping error.
                if !entry.slots.is_empty() {
                    report.push_warning(ValidationWarning {
                        path: crate::newtypes::FieldPath::new(format!("{base_path}.slots")),
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
                    path: crate::newtypes::FieldPath::new(format!("{base_path}.slots.rank{rank}")),
                    message: format!("spell slot rank {rank} is out of range (1..=10)"),
                });
            }
            if *count > 4 {
                report.push_warning(ValidationWarning {
                    path: crate::newtypes::FieldPath::new(format!("{base_path}.slots.rank{rank}")),
                    message: format!("spell slot count {count} is unusually high (typically 1–4)"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Chassis tests (relocated from `schema-pf2e`'s validation/derived suites).
    //! The math is system-agnostic; `minimal_build()` is a fighter/human/level-1
    //! build that exercises the shared chassis without favouring either system.

    use super::*;
    use crate::ability::AbilityScoreKey;
    use crate::build_types::*;
    use crate::newtypes::Level;
    use crate::refs::SystemRefId;
    use crate::testing::minimal_build;
    use crate::ContentEntry;
    use std::collections::BTreeMap;

    /// A minimal content index carrying only the ancestry `minimal_build()`
    /// references (human), so chassis tests need no system plugin.
    fn test_content_index() -> ContentIndex {
        let mut idx = ContentIndex::new(semver::Version::new(0, 0, 0));
        idx.insert(ContentEntry::ancestry("human", 8, 25));
        idx
    }

    // ── Validation ───────────────────────────────────────────────────────

    #[test]
    fn minimal_build_is_valid() {
        let build = minimal_build();
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
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
        let report = validate_chassis(&build);
        assert!(!report.is_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.path.as_str().contains("build.classFeatures[0].level")));
    }

    // ── Derived stats ────────────────────────────────────────────────────

    #[test]
    fn minimal_build_derived_stats() {
        let build = minimal_build();
        let stats = derived_stats(&build, &test_content_index());

        // STR: 10 + 2×1 boost = 12, modifier = 1
        assert_eq!(stats.ability_scores[&AbilityScoreKey::Str], 12);
        assert_eq!(stats.ability_modifiers[&AbilityScoreKey::Str], 1);

        // DEX/CON/etc not in the map → base 10, modifier 0
        assert_eq!(stats.ability_scores[&AbilityScoreKey::Dex], 10);
        assert_eq!(stats.ability_modifiers[&AbilityScoreKey::Dex], 0);

        // Max HP: fighter 10 HP/level × 1 + human ancestry 8 = 18
        assert_eq!(stats.max_hp, 18);

        // Perception: level 1 + untrained(0) + WIS mod 0 = 0 (Remaster: no level for untrained)
        assert_eq!(stats.perception, 0);

        // Class DC: level 1 + trained(2) + STR mod 1 = 4
        assert_eq!(stats.class_dc, 4);
    }

    #[test]
    fn ability_modifier_rounds_down() {
        assert_eq!(ability_modifier(10), 0);
        assert_eq!(ability_modifier(12), 1);
        assert_eq!(ability_modifier(13), 1);
        assert_eq!(ability_modifier(14), 2);
        assert_eq!(ability_modifier(8), -1);
        assert_eq!(ability_modifier(7), -2);
    }

    #[test]
    fn trained_adds_level() {
        assert_eq!(
            proficiency_modifier(5, ProficiencyRank::Trained, 3),
            5 + 2 + 3
        );
        assert_eq!(
            proficiency_modifier(20, ProficiencyRank::Legendary, 5),
            20 + 8 + 5
        );
    }

    #[test]
    fn untrained_omits_level() {
        assert_eq!(proficiency_modifier(10, ProficiencyRank::Untrained, 2), 2);
    }
}
