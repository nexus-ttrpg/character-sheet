//! PF2e derived-stat computation (§3.3, §10.2).
//!
//! The build stores *components* (boosts, flaws, proficiency ranks, class
//! HP/level); this module computes *outputs* (final scores, max HP, modifiers).
//! See §3.3: "store the inputs; let `SystemPlugin::derived_stats` compute the
//! output."

use std::collections::BTreeMap;

use schema_core::ability::AbilityScoreKey;
use schema_core::character::CharacterBuild;
use schema_core::plugin::{proficiency_or_untrained, DerivedStats};
use schema_core::proficiency::ProficiencyRank;
use schema_core::ContentIndex;

/// Compute derived stats for a PF2e build (§10.2).
pub fn compute(build: &CharacterBuild, content_index: &ContentIndex) -> DerivedStats {
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
    // PF2e Remaster: untrained checks do NOT add level; trained+ do.
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

    // Class DC defaults to Trained — every PF2e class grants it.
    let class_dc_rank = build
        .proficiencies
        .0
        .get(&schema_core::SkillKey::new("class-dc"))
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
/// PF2e rounds toward negative infinity, so score 7 → modifier −2, not −1.
fn ability_modifier(score: i16) -> i16 {
    (score - 10).div_euclid(2)
}

/// PF2e Remaster proficiency modifier: untrained does NOT add level (just
/// ability mod); trained+ adds `level + rank.bonus + ability_mod`.
fn proficiency_modifier(level: i16, rank: ProficiencyRank, ability_mod: i16) -> i16 {
    match rank {
        ProficiencyRank::Untrained => ability_mod,
        _ => level + rank.bonus() as i16 + ability_mod,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::ability::AbilityScoreKey;
    use schema_core::testing::minimal_build;

    #[test]
    fn minimal_build_derived_stats() {
        let build = minimal_build();
        let idx = crate::content::build();
        let stats = compute(&build, &idx);

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
