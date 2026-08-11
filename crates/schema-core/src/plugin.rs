//! System plugin trait, registry, and supporting types (§10.2, §10.3).
//!
//! A system plugin owns "what is a valid character in this ruleset, and how are
//! its derived stats computed" (§10.2). System plugins are Rust, compiled in,
//! contributed rarely — the inverse of adapters (§10.1).
//!
//! The trait is object-safe so the registry can hold `Box<dyn SystemPlugin>`.
//! The registry resolves `system → plugin` at runtime; `(system, platform) →
//! adapter` resolution is layered on top in the adapter host (§10.3).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ability::AbilityScoreKey;
use crate::character::CharacterBuild;
use crate::newtypes::{FieldPath, SystemId};
use crate::proficiency::ProficiencyRank;
use crate::refs::Slug;

// ── SystemPlugin trait (§10.2) ─────────────────────────────────────────────

/// A compiled-in ruleset plugin (§10.2).
///
/// Owns canonical-schema validation and derived-stat computation for one game
/// system (PF2e, SF2e, …). Stays in Rust because a system plugin defines part
/// of the canonical schema, and the canonical schema is the source of truth for
/// codegen (§3.6, §10.2).
///
/// Object-safe: the registry holds `Box<dyn SystemPlugin>`.
pub trait SystemPlugin: Send + Sync {
    /// The system identifier — `"pf2e"`, `"sf2e"`, … (§3.2, §10).
    fn id(&self) -> SystemId;

    /// JSON Schema for this system's canonical types (§10.2). For PF2e/SF2e this
    /// is the `CharacterBuild` schema (they share `schema-core`'s chassis); the
    /// system-specific rules live in [`validate`](Self::validate), not in a
    /// separate schema.
    fn json_schema(&self) -> schemars::schema::RootSchema;

    /// Semantic validation the type system can't express (§3.1, §10.2).
    ///
    /// This is the second validation layer: `serde` + `deny_unknown_fields`
    /// already enforce structural validity by construction (if you have a
    /// `CharacterBuild`, it deserialized cleanly). This method enforces
    /// cross-field and ruleset rules — feat levels within character level, key
    /// ability present, spellcasting entry consistency, etc.
    fn validate(&self, build: &CharacterBuild) -> ValidationReport;

    /// Derived stats computed from the build (§10.2). Final ability scores,
    /// max HP, perception, saving throws, class DC — the things that are
    /// *derived*, not stored (§3.3: "store the inputs; let
    /// `SystemPlugin::derived_stats` compute the output").
    fn derived_stats(&self, build: &CharacterBuild) -> DerivedStats;

    /// The curated, versioned system content index (§9.1). Maps slugs to
    /// canonical rules data so an adapter with no native ID can resolve by slug.
    fn content_index(&self) -> &ContentIndex;
}

// ── ValidationReport (§10.2, §5.2) ─────────────────────────────────────────

/// The result of validating a `CharacterBuild` against a system's rules
/// (§10.2, §5.2).
///
/// Mirrors the TS-side `ValidationReport` concept: errors are hard failures
/// (the build is not valid for this system), warnings are soft (the build is
/// valid but has caveats — unknown content, lossy mappings, etc.).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    /// Hard failures — the build violates a ruleset constraint.
    pub errors: Vec<ValidationError>,

    /// Soft caveats — the build is valid but has issues worth surfacing.
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn push_error(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    pub fn push_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }
}

/// A hard validation failure (§5.2, §10.2).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    /// Canonical field path where the error was found (§11.2).
    pub path: FieldPath,
    pub message: String,
}

/// A soft validation caveat (§5.2, §10.2).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationWarning {
    /// Canonical field path where the warning was found (§11.2).
    pub path: FieldPath,
    pub message: String,
}

// ── DerivedStats (§10.2) ───────────────────────────────────────────────────

/// Stats derived from a `CharacterBuild` (§3.3, §10.2).
///
/// These are computed, not stored — the build stores the *components* (boosts,
/// flaws, proficiency ranks, class HP/level) and the system plugin computes the
/// *outputs* (final scores, max HP, modifiers).
///
/// PF2e and SF2e share the same derived-stat chassis (ability modifiers, HP,
/// perception, saving throws, class DC) because they share the same proficiency
/// and ability-score model (§3.4). System-specific derived stats that don't
/// overlap can be carried in [`extensions`](crate::extensions::ExtensionBag).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DerivedStats {
    /// Final computed ability scores (base 10 + boosts − flaws + item bonus).
    pub ability_scores: BTreeMap<AbilityScoreKey, i16>,

    /// Ability modifiers: `(score − 10) / 2` (integer division, rounded down).
    pub ability_modifiers: BTreeMap<AbilityScoreKey, i16>,

    /// Maximum HP: `class.hp_per_level × level + ancestry_hp` (§7.2).
    pub max_hp: i16,

    /// Perception modifier: `level + perception_rank.bonus + WIS modifier`.
    pub perception: i16,

    /// Fortitude save: `level + rank.bonus + CON modifier`.
    pub fortitude: i16,

    /// Reflex save: `level + rank.bonus + DEX modifier`.
    pub reflex: i16,

    /// Will save: `level + rank.bonus + WIS modifier`.
    pub will: i16,

    /// Class DC: `level + class_dc_rank.bonus + key_ability modifier`.
    pub class_dc: i16,
}

// ── ContentIndex (§9.1) ────────────────────────────────────────────────────

/// A curated, versioned index of system content, keyed by slug (§9.1).
///
/// Maps slugs to canonical rules data so an adapter with no native ID can
/// resolve by slug plus fuzzy name match. The content index is curated and grows
/// over time; in 0.3 it carries a minimal set sufficient for `derived_stats`
/// (ancestry HP, base speed).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentIndex {
    /// Semver version of the content index data.
    #[schemars(schema_with = "schema_semver")]
    pub version: semver::Version,

    /// Slug → entry map.
    pub entries: BTreeMap<Slug, ContentEntry>,
}

impl Default for ContentIndex {
    fn default() -> Self {
        Self {
            version: semver::Version::new(0, 0, 0),
            entries: BTreeMap::new(),
        }
    }
}

/// JSON Schema for `semver::Version` — a semver string (§9.3).
fn schema_semver(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "string",
        "description": "Semantic version (semver)."
    }))
    .expect("valid schema")
}

impl ContentIndex {
    pub fn new(version: semver::Version) -> Self {
        Self {
            version,
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, entry: ContentEntry) {
        self.entries.insert(entry.slug.clone(), entry);
    }

    pub fn get(&self, slug: &Slug) -> Option<&ContentEntry> {
        self.entries.get(slug)
    }
}

/// What kind of rules content an entry is (§9.1).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    Ancestry,
    Heritage,
    Background,
    Class,
    Feat,
    Spell,
    Item,
    Action,
    Condition,
}

/// A single entry in the system content index (§9.1).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentEntry {
    pub slug: Slug,
    pub kind: ContentKind,

    /// Rulebook source, e.g. `"Player Core"` (§9.1).
    pub source: Option<String>,

    /// HP bonus granted by an ancestry (None for non-ancestry entries).
    pub hp: Option<u8>,

    /// Base speed in feet (ancestries).
    pub speed: Option<u16>,
}

impl ContentEntry {
    /// Create an ancestry entry.
    pub fn ancestry(slug: impl Into<String>, hp: u8, speed: u16) -> Self {
        Self {
            slug: Slug::new(slug),
            kind: ContentKind::Ancestry,
            source: None,
            hp: Some(hp),
            speed: Some(speed),
        }
    }
}

// ── SystemPluginRegistry (§10.3) ───────────────────────────────────────────

/// Resolves `system → &dyn SystemPlugin` at runtime (§10.3).
///
/// The full `(system, platform) → adapter` resolution is layered on top in the
/// adapter host (Phase 1); this registry only owns the system axis.
///
/// Compiled-in plugins are registered once at startup; lookups are cheap
/// (`BTreeMap` by `SystemId`).
pub struct SystemPluginRegistry {
    plugins: BTreeMap<SystemId, Box<dyn SystemPlugin>>,
}

impl SystemPluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: BTreeMap::new(),
        }
    }

    /// Register a compiled-in system plugin. The last registration for a given
    /// `SystemId` wins.
    pub fn register(&mut self, plugin: Box<dyn SystemPlugin>) {
        let id = plugin.id();
        self.plugins.insert(id, plugin);
    }

    /// Look up a system plugin by system id.
    pub fn get(&self, system: &SystemId) -> Option<&dyn SystemPlugin> {
        self.plugins.get(system).map(|p| p.as_ref())
    }

    /// Iterate over all registered system ids.
    pub fn systems(&self) -> impl Iterator<Item = &SystemId> {
        self.plugins.keys()
    }
}

impl Default for SystemPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Proficiency lookup helper (shared by PF2e/SF2e) ─────────────────────────

/// Look up a proficiency rank from a `ProficiencyTable` by skill key, defaulting
/// to [`ProficiencyRank::Untrained`] when absent.
///
/// PF2e and SF2e both model saving throws, perception, and class DC as entries
/// in the shared proficiency table (keyed by `SkillKey` like `"fortitude"`,
/// `"perception"`, `"class-dc"`). This helper centralizes the defaulting logic.
pub fn proficiency_or_untrained(
    table: &crate::proficiency::ProficiencyTable,
    key: &str,
) -> ProficiencyRank {
    table
        .0
        .get(&crate::newtypes::SkillKey::new(key))
        .copied()
        .unwrap_or(ProficiencyRank::Untrained)
}

// ── Static registry helper ─────────────────────────────────────────────────
//
// A process-wide registry is not needed in 0.3 (the binary constructs one at
// startup), but the `OnceLock` pattern is here for future use if a global
// fallback is wanted. Currently unused — kept as a placeholder to document the
// intent.

#[allow(dead_code)]
static GLOBAL_REGISTRY: OnceLock<SystemPluginRegistry> = OnceLock::new();
