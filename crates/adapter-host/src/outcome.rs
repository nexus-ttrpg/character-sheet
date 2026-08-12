//! Rust mirrors of the TS adapter outcome types (§5.1, §5.2).
//!
//! These are the types the host deserializes adapter output into — the
//! boundary check is that deserialization succeeds against the canonical
//! schema with `deny_unknown_fields` (§5.5, §1).

use schema_core::{CharacterBuild, SystemId};
use serde::{Deserialize, Serialize};

/// A warning from the adapter — the operation succeeded with caveats (§5.2).
/// Homebrew content must always be a warning, never an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterWarning {
    /// Canonical field path, e.g. `build.feats[2]`.
    pub path: String,

    /// What kind of warning this is (§5.2).
    pub kind: WarningKind,

    /// Human-readable detail.
    pub detail: String,
}

/// Warning categories (§5.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", deny_unknown_fields)]
pub enum WarningKind {
    /// A field from the platform payload had no canonical mapping — preserved
    /// in `extensions`.
    UnmappedField,

    /// A canonical field was dropped on export because the target platform has
    /// no representation for it.
    LossyExport { reason: String },

    /// A fuzzy match was used (e.g. feat name → compendium UUID).
    FuzzyMatch {
        /// 0.0–1.0 confidence.
        confidence: f64,
    },
}

/// The outcome of `adapter.import()` (§5.1).
///
/// The `build` field is a `PartialCharacterBuild` — all fields optional because
/// an adapter may not cover every canonical field. `deny_unknown_fields` ensures
/// unknown fields are rejected at the boundary (§5.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportOutcome {
    pub build: PartialCharacterBuild,
    pub warnings: Vec<AdapterWarning>,
    /// Canonical field paths the adapter could not map.
    #[serde(default)]
    pub unmapped: Vec<String>,
}

/// The outcome of `adapter.export()` (§5.1).
///
/// The `payload` is the platform-specific format — opaque to the host, passed
/// through as `serde_json::Value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportOutcome {
    pub payload: serde_json::Value,
    pub warnings: Vec<AdapterWarning>,
    /// Canonical field paths that were skipped (no platform representation).
    #[serde(default)]
    pub skipped: Vec<String>,
}

/// A change set from `adapter.detectChanges()` (§5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSet {
    /// True if anything changed since the last sync.
    pub has_changes: bool,
    /// Field paths that changed.
    #[serde(default)]
    pub changed_paths: Vec<String>,
}

/// A validation report from `adapter.validate()` (§5.1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationReport {
    pub valid: bool,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ── PartialCharacterBuild ──────────────────────────────────────────────────
//
// Mirrors `CharacterBuild` (§3.3) with all fields optional — this is the Rust
// equivalent of the TS `PartialBuild`. An adapter may not cover every canonical
// field; the `Option<>` represents that. `deny_unknown_fields` ensures unknown
// field names are rejected at the boundary (§5.5).
//
// PRESENT fields are type-checked by serde; the structural validation (level
// bounds, etc.) happens when the partial build is completed and merged.

use schema_core::build_types::*;

/// A partial `CharacterBuild` — all fields optional (§5.1, §5.2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PartialCharacterBuild {
    pub identity: Option<Identity>,
    pub ancestry: Option<Ancestry>,
    pub heritage: Option<Heritage>,
    pub background: Option<Background>,
    pub class: Option<ClassEntry>,
    pub level: Option<schema_core::Level>,
    pub ability_scores: Option<schema_core::AbilityScoreMap>,
    pub skills: Option<schema_core::SkillMap>,
    pub proficiencies: Option<schema_core::ProficiencyTable>,
    pub feats: Option<Vec<FeatEntry>>,
    pub class_features: Option<Vec<ClassFeatureEntry>>,
    pub equipment: Option<Equipment>,
    pub currencies: Option<Currencies>,
    pub spellcasting: Option<Spellcasting>,
    pub actions: Option<Vec<ActionEntry>>,
    pub reactions: Option<Vec<ReactionEntry>>,
    pub exploration_activities: Option<Vec<ExplorationActivityEntry>>,
    pub companions: Option<Vec<CompanionEntry>>,
    pub familiars: Option<Vec<FamiliarEntry>>,
    pub drones: Option<Vec<DroneEntry>>,
    pub vehicles: Option<Vec<VehicleEntry>>,
    pub notes: Option<Vec<NoteEntry>>,
}

impl PartialCharacterBuild {
    /// Returns true if no build fields are present.
    pub fn is_empty(&self) -> bool {
        self.identity.is_none()
            && self.ancestry.is_none()
            && self.heritage.is_none()
            && self.background.is_none()
            && self.class.is_none()
            && self.level.is_none()
            && self.ability_scores.is_none()
            && self.skills.is_none()
            && self.proficiencies.is_none()
            && self.feats.is_none()
            && self.class_features.is_none()
            && self.equipment.is_none()
            && self.currencies.is_none()
            && self.spellcasting.is_none()
            && self.actions.is_none()
            && self.reactions.is_none()
            && self.exploration_activities.is_none()
            && self.companions.is_none()
            && self.familiars.is_none()
            && self.drones.is_none()
            && self.vehicles.is_none()
            && self.notes.is_none()
    }

    /// Count how many top-level fields are present (for coverage metrics).
    pub fn field_count(&self) -> usize {
        let mut n = 0;
        if self.identity.is_some() {
            n += 1;
        }
        if self.ancestry.is_some() {
            n += 1;
        }
        if self.heritage.is_some() {
            n += 1;
        }
        if self.background.is_some() {
            n += 1;
        }
        if self.class.is_some() {
            n += 1;
        }
        if self.level.is_some() {
            n += 1;
        }
        if self.ability_scores.is_some() {
            n += 1;
        }
        if self.skills.is_some() {
            n += 1;
        }
        if self.proficiencies.is_some() {
            n += 1;
        }
        if self.feats.is_some() {
            n += 1;
        }
        if self.class_features.is_some() {
            n += 1;
        }
        if self.equipment.is_some() {
            n += 1;
        }
        if self.currencies.is_some() {
            n += 1;
        }
        if self.spellcasting.is_some() {
            n += 1;
        }
        if self.actions.is_some() {
            n += 1;
        }
        if self.reactions.is_some() {
            n += 1;
        }
        if self.exploration_activities.is_some() {
            n += 1;
        }
        if self.companions.is_some() {
            n += 1;
        }
        if self.familiars.is_some() {
            n += 1;
        }
        if self.drones.is_some() {
            n += 1;
        }
        if self.vehicles.is_some() {
            n += 1;
        }
        if self.notes.is_some() {
            n += 1;
        }
        n
    }
}

// ── Context types (§5.4) ───────────────────────────────────────────────────

/// Context passed to `adapter.import()` (§5.4). Carries credentials from the
/// connection record — adapters never hold credentials themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportContext {
    /// The system this import targets (e.g. "pf2e").
    pub system: SystemId,
    /// Platform credentials (API tokens, etc.) — from the connection record.
    /// Opaque to the host; the adapter knows how to use them.
    pub credentials: serde_json::Value,
}

/// Context passed to `adapter.export()` (§5.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportContext {
    pub system: SystemId,
    pub credentials: serde_json::Value,
}

/// The complete canonical build — used when the host needs to pass a full
/// `CharacterBuild` to `adapter.export()`.
impl From<CharacterBuild> for PartialCharacterBuild {
    fn from(b: CharacterBuild) -> Self {
        Self {
            identity: Some(b.identity),
            ancestry: Some(b.ancestry),
            heritage: Some(b.heritage),
            background: Some(b.background),
            class: Some(b.class),
            level: Some(b.level),
            ability_scores: Some(b.ability_scores),
            skills: Some(b.skills),
            proficiencies: Some(b.proficiencies),
            feats: Some(b.feats),
            class_features: Some(b.class_features),
            equipment: Some(b.equipment),
            currencies: Some(b.currencies),
            spellcasting: Some(b.spellcasting),
            actions: Some(b.actions),
            reactions: Some(b.reactions),
            exploration_activities: Some(b.exploration_activities),
            companions: Some(b.companions),
            familiars: Some(b.familiars),
            drones: Some(b.drones),
            vehicles: Some(b.vehicles),
            notes: Some(b.notes),
        }
    }
}
