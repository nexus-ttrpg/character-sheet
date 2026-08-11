//! `ExtensionBag` — the escape hatch for unmodeled/homebrew data (§3.5).
//!
//! Any adapter or plugin can stash platform-specific or homebrew data here,
//! namespaced. The sync engine treats values as opaque `serde_json::Value`:
//! it diffs them structurally (so conflicts are still detected) but applies no
//! schema validation beyond "is valid JSON."
//!
//! This is the pressure-release valve that keeps the core schema from needing a
//! breaking change every time Foundry adds an actor flag — and it's why
//! `deny_unknown_fields` on the typed regions is safe rather than hostile.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

/// A namespaced bag of opaque JSON values (§3.5).
///
/// e.g. `"foundry.pf2e"` -> `{ "flags": {...} }`,
///      `"homebrew.myGuild"` -> `{ "customTrait": "..." }`
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct ExtensionBag(pub BTreeMap<Namespace, serde_json::Value>);

/// The namespace key for an extension entry (§3.5). Dotted convention, e.g.
/// `"foundry.pf2e"`. Re-exported from `newtypes` for ergonomic access.
pub use crate::newtypes::Namespace;

impl ExtensionBag {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert(
        &mut self,
        ns: impl Into<Namespace>,
        value: serde_json::Value,
    ) -> Option<serde_json::Value> {
        self.0.insert(ns.into(), value)
    }

    pub fn get(&self, ns: &Namespace) -> Option<&serde_json::Value> {
        self.0.get(ns)
    }
}

impl Deref for ExtensionBag {
    type Target = BTreeMap<Namespace, serde_json::Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ExtensionBag {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
