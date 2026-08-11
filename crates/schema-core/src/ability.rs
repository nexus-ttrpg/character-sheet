//! Ability scores (§3.3).
//!
//! `AbilityScoreEntry` deliberately stores the *components* that produce the
//! final score rather than only the final number, because several source
//! platforms (Pathbuilder, Wanderer's Guide) recompute the final number
//! themselves. Store the inputs; let `SystemPlugin::derived_stats` (§10)
//! compute the output.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The six PF2e ability score keys (§3.3). SF2e reuses the same chassis (§3.4).
/// Used as a `BTreeMap` key, so derived `Ord` (alphabetical) gives stable
/// serialization order for content hashing (§9.2).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "lowercase")]
pub enum AbilityScoreKey {
    Str,
    Dex,
    Con,
    Int,
    Wis,
    Cha,
}

/// The source of an ability boost or flaw (§3.3).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub enum AbilityBoostSource {
    Ancestry,
    Background,
    Class,
    Free,
}

/// An ability boost or flaw (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AbilityBoost {
    pub source: AbilityBoostSource,
}

/// The components that produce an ability score (§3.3).
///
/// The final score is *derived*, not stored — computed by
/// `SystemPlugin::derived_stats` (§10). Storing components lets adapters import
/// from platforms that recompute the final number without reverse-engineering.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbilityScoreEntry {
    /// Boosts applied to this ability (ancestry, background, class, free).
    pub boosts: Vec<AbilityBoost>,

    /// Flaws applied to this ability.
    pub flaws: Vec<AbilityBoost>,

    /// Item bonuses (or penalties, if negative) applied to this ability.
    pub item_bonus: i8,
}

/// A skill entry (§3.3). Includes lore skills.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillEntry {
    /// Proficiency rank for this skill.
    pub rank: crate::proficiency::ProficiencyRank,

    /// Whether this skill is a lore skill, and if so, its name.
    pub lore: Option<String>,

    /// Item bonus to this skill.
    pub item_bonus: i8,
}

/// Convenience alias for the ability-scores map shape (§3.3).
pub type AbilityScoreMap = BTreeMap<AbilityScoreKey, AbilityScoreEntry>;
