//! Proficiency ranks and the proficiency table (§3.3, §10.2).
//!
//! Proficiency ranks are part of the shared `schema-core` chassis — PF2e and
//! SF2e both use the same five-tier system (§3.4, §10.2).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::newtypes::SkillKey;

/// The five PF2e/SF2e proficiency tiers (§3.3).
///
/// Order is `Untrained < Trained < Expert < Master < Legendary`; derived
/// `PartialOrd` gives the right ordering, which matters for derived-stat
/// computation (§10.2).
#[derive(
    Serialize,
    Deserialize,
    JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ProficiencyRank {
    #[default]
    Untrained,
    Trained,
    Expert,
    Master,
    Legendary,
}

impl ProficiencyRank {
    /// The numeric bonus this rank grants: +0, +2, +4, +6, +8 (PF2e).
    pub fn bonus(self) -> i8 {
        match self {
            Self::Untrained => 0,
            Self::Trained => 2,
            Self::Expert => 4,
            Self::Master => 6,
            Self::Legendary => 8,
        }
    }
}

/// A table of proficiencies keyed by skill key (§3.3). `BTreeMap` for stable
/// serialization order (§9.2). Includes lore skills.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct ProficiencyTable(pub BTreeMap<SkillKey, ProficiencyRank>);
