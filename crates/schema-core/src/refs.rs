//! `SystemRefId` — the reference resolution primitive (§9.1).
//!
//! `slug` is the cross-platform anchor. A curated, versioned system content
//! index maps slugs to canonical rules data, so an adapter with no native ID
//! can resolve by slug plus fuzzy name match — with match confidence surfaced
//! as a warning rather than silently accepted (§9.1).
//!
//! `#[serde(flatten)]` on `other` means a new platform's ID field round-trips
//! through older builds of the software without data loss. Note: this is
//! incompatible with `deny_unknown_fields` on this struct (a serde limitation),
//! which is fine — `SystemRefId` is designed to be extensible.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::newtypes::Namespace;

/// A slug: the cross-platform anchor for rules content (§9.1). e.g.
/// `"fireball"`, `"toughness"`. Lowercase, hyphen-separated; validation is
/// light in 0.2 (non-empty string).
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default,
)]
pub struct Slug(String);

impl Slug {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Slug {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Slug {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Cross-platform reference to a piece of rules content (§9.1).
///
/// Carries a slug (the universal anchor) plus any platform-specific IDs the
/// adapters have populated. The `other` map is flattened so unknown platform
/// ID fields round-trip through older builds.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemRefId {
    /// Cross-platform anchor, e.g. `"fireball"` (§9.1).
    pub slug: Slug,

    /// Rulebook source, e.g. `"Player Core"` (§9.1).
    pub source: Option<String>,

    /// Foundry compendium UUID, when an imported item resolves to a compendium
    /// source (§7.3).
    pub foundry_uuid: Option<String>,

    /// Roll20 attribute/id, when present.
    pub roll20_id: Option<String>,

    /// Open map for future platforms. Flattened into the JSON, so a new
    /// platform's ID field round-trips through older builds (§9.1).
    #[serde(flatten)]
    pub other: BTreeMap<String, String>,
}

impl SystemRefId {
    /// Construct from a slug alone — the common case for adapters with no
    /// native ID.
    pub fn from_slug(slug: impl Into<String>) -> Self {
        Self {
            slug: Slug::new(slug),
            source: None,
            foundry_uuid: None,
            roll20_id: None,
            other: BTreeMap::new(),
        }
    }
}

// Suppress unused-import warning for Namespace — it's re-exported from the
// crate root and used by ExtensionBag (extensions.rs), but referenced here in
// the module's doc context. Keep the import so downstream code can reach it
// via `crate::refs::Namespace` if desired.
#[allow(unused_imports)]
use _Namespace as _;
type _Namespace = Namespace;
