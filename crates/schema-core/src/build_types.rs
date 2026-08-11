//! Sub-structs referenced by `CharacterBuild` (§3.3).
//!
//! The shapes here are the base canonical chassis. PF2e-specific validation and
//! derived-stat computation live in `schema-pf2e` (§10.2, 0.3); SF2e-specific
//! shapes (drones, Resolve Points, credits) are carried here as optional/SF2e
//! fields per §3.4 so a crossover campaign carrying both isn't structurally
//! impossible.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ability::{AbilityScoreKey, SkillEntry};
use crate::newtypes::{Level, SkillKey};
use crate::refs::SystemRefId;

// ── Identity ───────────────────────────────────────────────────────────────

/// Character identity (§3.3). The export example uses `build.identity.name`
/// (§7.2).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub name: String,
    pub player_name: Option<String>,
    pub pronouns: Option<String>,
}

// ── Ancestry / Heritage / Background ───────────────────────────────────────
//
// These carry a `SystemRefId` (the cross-platform anchor, §9.1) plus an
// optional display name for platforms that surface free text. The Foundry
// adapter's `refOrText(build.ancestry)` helper (§7.2) reads from this shape.

/// Ancestry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ancestry {
    pub reference: SystemRefId,
    pub display_name: Option<String>,
}

/// Heritage (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Heritage {
    pub reference: SystemRefId,
    pub display_name: Option<String>,
}

/// Background (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Background {
    pub reference: SystemRefId,
    pub display_name: Option<String>,
}

// ── Class ──────────────────────────────────────────────────────────────────

/// Class entry (§3.3). Carries the class reference plus the key ability and
/// HP-per-level, which are needed for derived-stat computation (§10.2).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassEntry {
    pub reference: SystemRefId,
    pub key_ability: AbilityScoreKey,
    pub hp_per_level: u16,
}

// ── Feats ──────────────────────────────────────────────────────────────────

/// Feat category tag (§3.3, "tagged by category").
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub enum FeatCategory {
    Ancestry,
    Class,
    Skill,
    General,
    Bonus,
}

/// A feat entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatEntry {
    pub reference: SystemRefId,
    pub category: FeatCategory,
    pub level: Level,
}

/// A class feature entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassFeatureEntry {
    pub reference: SystemRefId,
    pub level: Level,
}

// ── Actions ────────────────────────────────────────────────────────────────

/// Action type for the three-action economy (§3.4 — shared chassis).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub enum ActionType {
    Single,
    TwoAction,
    ThreeAction,
    Free,
    Reaction,
    OneMinute,
}

/// An action entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionEntry {
    pub reference: SystemRefId,
    pub action_type: ActionType,
}

/// A reaction entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReactionEntry {
    pub reference: SystemRefId,
}

/// An exploration activity entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplorationActivityEntry {
    pub reference: SystemRefId,
}

// ── Equipment & Currencies ─────────────────────────────────────────────────

/// A reference to an item with a quantity (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemRef {
    pub reference: SystemRefId,
    pub quantity: u32,
}

/// Equipment (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Equipment {
    pub weapons: Vec<ItemRef>,
    pub armor: Vec<ItemRef>,
    pub inventory: Vec<ItemRef>,
}

/// Currencies (§3.4). One struct with optional fields per denomination so a
/// crossover campaign carrying both gp and credits isn't structurally
/// impossible (§3.4).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Currencies {
    /// PF2e: platinum.
    pub pp: Option<u32>,
    /// PF2e: gold.
    pub gp: Option<u32>,
    /// PF2e: silver.
    pub sp: Option<u32>,
    /// PF2e: copper.
    pub cp: Option<u32>,
    /// SF2e: credits (§3.4).
    pub credits: Option<u32>,
    /// SF2e: UPB — Universal Point Base (§3.4).
    pub upb: Option<u32>,
}

// ── Spellcasting ───────────────────────────────────────────────────────────

/// Magic tradition (§3.4 — shared chassis).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "lowercase")]
pub enum MagicTradition {
    Arcane,
    Divine,
    Occult,
    Primal,
}

/// How spells are prepared (§3.1 — garde asserts "a spellcasting entry with
/// tradition `prepared` must have a slot table").
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "lowercase")]
pub enum SpellPreparedness {
    Prepared,
    Spontaneous,
    Innate,
}

/// A spellcasting entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpellcastingEntry {
    pub tradition: MagicTradition,
    pub preparedness: SpellPreparedness,
    pub attribute: AbilityScoreKey,

    /// Slot table — rank (1..=10) to slot count. Required for `Prepared` and
    /// `Spontaneous`; empty for `Innate`. Asserted by garde (§3.1).
    pub slots: BTreeMap<u8, u8>,

    /// Known/prepared spells.
    pub spells: Vec<SystemRefId>,
}

/// Spellcasting (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Spellcasting {
    pub entries: Vec<SpellcastingEntry>,
}

// ── Companions / Familiars / Drones / Vehicles ─────────────────────────────

/// Companion type (§3.4, §7.5).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub enum CompanionType {
    Animal,
    Eidolon,
    Construct,
}

/// A companion entry (§3.3, §7.5). Treated as its own mini sub-tree.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionEntry {
    pub reference: SystemRefId,
    pub companion_type: CompanionType,
}

/// A familiar entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FamiliarEntry {
    pub reference: SystemRefId,
}

/// A drone entry — SF2e mechanic (§3.4). Structurally similar to companions but
/// with chassis/mod slots.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DroneEntry {
    pub reference: SystemRefId,

    /// Chassis type (e.g. "combat", "hover").
    pub chassis: Option<String>,

    /// Installed mods.
    pub mods: Vec<SystemRefId>,
}

/// A vehicle entry (§3.3, §3.4). Kept lightweight: role + relevant skill.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VehicleEntry {
    pub reference: SystemRefId,
    pub role: Option<String>,
}

// ── Notes ──────────────────────────────────────────────────────────────────

/// A note entry (§3.3).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteEntry {
    pub title: String,
    pub body: String,
}

// ── Re-exports for the skills map shape ────────────────────────────────────

/// Convenience alias for the skills map shape (§3.3).
pub type SkillMap = BTreeMap<SkillKey, SkillEntry>;
