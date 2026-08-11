//! `Character`, `CharacterMeta`, `CharacterBuild` (§3.1–3.3).
//!
//! The model splits into three top-level partitions so the sync engine can
//! apply a blanket rule ("diff/merge `build`, never touch `session`
//! automatically") instead of maintaining a field-by-field allowlist (§3.1).
//! The partition *is* the policy — in Rust it's enforced by the function
//! signature: `merge()` takes `&CharacterBuild`, so it is not *able* to see
//! `session` (§3.1, §11.3).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ability::{AbilityScoreMap, SkillEntry};
use crate::build_types::*;
use crate::extensions::ExtensionBag;
use crate::newtypes::{ContentHash, Level, SkillKey, Ulid};
use crate::proficiency::ProficiencyTable;
use crate::session::SessionState;

// ── Character (top-level, §3.1) ────────────────────────────────────────────

/// The top-level canonical character (§3.1).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Character {
    /// Identity, versioning, ownership (§3.2).
    pub meta: CharacterMeta,

    /// Permanent, sync-eligible build data (§3.3). This is what `merge()`
    /// operates on (§11.3).
    pub build: CharacterBuild,

    /// Ephemeral, per-platform, NOT auto-synced (§4). The ingest path
    /// deserializes into a `SessionTelemetry` struct that has no conversion
    /// into `CharacterBuild` (§4).
    pub session: SessionState,

    /// Unknown/future/homebrew fields, namespaced (§3.5).
    pub extensions: ExtensionBag,
}

// ── CharacterMeta (§3.2) ───────────────────────────────────────────────────

/// Identity, versioning, ownership (§3.2).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterMeta {
    pub id: Ulid,

    /// Schema version — semver. Minor/patch are additive-only; major may
    /// restructure and ship a migration (§9.3).
    #[schemars(schema_with = "schema_semver")]
    pub schema_version: semver::Version,

    /// `"pf2e"` | `"sf2e"`; extensible via plugin registry (§3.2, §10).
    pub system: crate::newtypes::SystemId,

    pub owner_id: Ulid,
    pub display_name: String,

    #[schemars(schema_with = "schema_datetime")]
    pub created_at: chrono::DateTime<chrono::Utc>,

    #[schemars(schema_with = "schema_datetime")]
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// BLAKE3 of canonicalized `build` (§9.2). Not of `session` or `meta`.
    pub content_hash: ContentHash,
}

/// JSON Schema for `semver::Version` — a semver string (§9.3).
fn schema_semver(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "string",
        "description": "Semantic version (semver). Minor/patch are additive; major may restructure (§9.3)."
    }))
    .expect("valid schema")
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

// ── CharacterBuild (§3.3) ──────────────────────────────────────────────────

/// Permanent, synchronizable build data (§3.3).
///
/// This is what the sync engine diffs and merges (§11). `merge()` takes
/// `&CharacterBuild` — it cannot see `session` or `meta` (§3.1, §11.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterBuild {
    pub identity: Identity,
    pub ancestry: Ancestry,
    pub heritage: Heritage,
    pub background: Background,
    pub class: ClassEntry,

    /// Newtype, 1..=20 enforced on construction (§3.3).
    pub level: Level,

    /// `BTreeMap` for deterministic serialization (§3.3, §9.2).
    pub ability_scores: AbilityScoreMap,

    /// Skills, including lore (§3.3). `BTreeMap<SkillKey, SkillEntry>`.
    pub skills: BTreeMap<SkillKey, SkillEntry>,

    pub proficiencies: ProficiencyTable,

    /// Feats, tagged by category (§3.3).
    pub feats: Vec<FeatEntry>,

    pub class_features: Vec<ClassFeatureEntry>,
    pub equipment: Equipment,
    pub currencies: Currencies,
    pub spellcasting: Spellcasting,
    pub actions: Vec<ActionEntry>,
    pub reactions: Vec<ReactionEntry>,
    pub exploration_activities: Vec<ExplorationActivityEntry>,
    pub companions: Vec<CompanionEntry>,
    pub familiars: Vec<FamiliarEntry>,

    /// SF2e mechanic (§3.4).
    pub drones: Vec<DroneEntry>,

    pub vehicles: Vec<VehicleEntry>,
    pub notes: Vec<NoteEntry>,
}

// Test fixtures live in `testing::minimal_build()` so they're visible to
// integration tests and downstream crates (§16), not gated behind `cfg(test)`.
