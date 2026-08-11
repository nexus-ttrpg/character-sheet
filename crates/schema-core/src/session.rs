//! Session state — what never auto-syncs (§4).
//!
//! These fields live outside canonical sync *structurally* rather than by
//! convention: `SessionState` has no conversion into `CharacterBuild`. The
//! ingest path deserializes session data into a `SessionTelemetry` struct (lands
//! with the API/store in Phase 2) that has no `Into<CharacterBuild>`.
//!
//! The sync engine's `merge()` takes `&CharacterBuild`, so it is not *able* to
//! see `session` (§3.1, §4) — the partition *is* the policy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::newtypes::{PlatformId, SlotKey};
use crate::refs::SystemRefId;

/// Ephemeral, per-platform session state — NOT auto-synced (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionState {
    /// Which platform this session state belongs to (§4).
    pub platform: PlatformId,

    /// Current HP — scene-scoped, not character-scoped (§4).
    pub hit_points: HitPoints,

    /// Spell slot usage — scene-scoped (§4).
    pub spell_slots: BTreeMap<SlotKey, SlotUsage>,

    /// Focus points — scene-scoped (§4).
    pub focus_points: PointPool,

    /// Resolve Points — SF2e only (§3.4, §4).
    pub resolve_points: Option<PointPool>,

    /// Hero points (§4).
    pub hero_points: u8,

    /// Active conditions, including value (e.g. frightened 2) (§4).
    pub conditions: Vec<ConditionInstance>,

    /// Initiative roll — scene-scoped (§4).
    pub initiative: Option<i16>,

    /// Temporary modifiers — scene-scoped (§4).
    pub temporary_modifiers: Vec<TemporaryModifier>,

    /// Encounter effects — scene-scoped (§4).
    pub encounter_effects: Vec<EncounterEffect>,

    /// When this session state was last observed from the platform (§4).
    #[schemars(schema_with = "schema_datetime")]
    pub last_observed_at: chrono::DateTime<chrono::Utc>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            platform: PlatformId::default(),
            hit_points: HitPoints::default(),
            spell_slots: BTreeMap::new(),
            focus_points: PointPool::default(),
            resolve_points: None,
            hero_points: 0,
            conditions: Vec::new(),
            initiative: None,
            temporary_modifiers: Vec::new(),
            encounter_effects: Vec::new(),
            last_observed_at: chrono::DateTime::from_timestamp(0, 0).unwrap_or_default(),
        }
    }
}

/// JSON Schema for `chrono::DateTime<Utc>` — a date-time string (RFC 3339).
fn schema_datetime(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "string",
        "format": "date-time",
        "description": "ISO 8601 / RFC 3339 timestamp"
    }))
    .expect("valid schema")
}

/// Hit points — current, temporary, max (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HitPoints {
    /// Current HP. Can be negative (dying) in PF2e.
    pub current: i16,

    /// Temporary HP.
    pub temporary: i16,

    /// Max HP. Only the max is derivable from `build`; current/temp are
    /// scene-scoped (§4, §7.2).
    pub max: i16,
}

/// A point pool like focus points or Resolve Points (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PointPool {
    pub current: u8,
    pub max: u8,
}

/// Spell slot usage for a given rank (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlotUsage {
    pub expended: u8,
    pub max: u8,
}

/// An active condition, including its value (§4). e.g. frightened 2.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConditionInstance {
    pub reference: SystemRefId,

    /// The condition's numeric value, if it has one (e.g. frightened 2).
    pub value: Option<u8>,
}

/// A temporary modifier — scene-scoped (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemporaryModifier {
    pub reference: SystemRefId,

    /// The modifier value (can be negative).
    pub bonus: i16,
}

/// An encounter effect — scene-scoped (§4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EncounterEffect {
    pub reference: SystemRefId,
}
