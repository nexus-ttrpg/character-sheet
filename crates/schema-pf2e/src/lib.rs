//! PF2e SystemPlugin (§3.4, §10.2). Compiled in; extends schema-core.
//!
//! Owns "what is a valid PF2e character, and how are its derived stats
//! computed" (§10.2). The canonical schema types are shared via `schema-core`
//! (§3.4: "PF2e and SF2e both extend a shared schema-core"); this plugin adds
//! PF2e-specific validation rules and derived-stat formulas on top.
//!
//! ## What lives here vs. schema-core
//!
//! - **schema-core** — the canonical *types* (`CharacterBuild`, `AbilityScore`,
//!   proficiency ranks, …). System-agnostic; PF2e and SF2e share it.
//! - **schema-pf2e** (this crate) — the *rules*: which ability boosts are
//!   valid, how HP is computed, what a valid spellcasting entry looks like.
//!
//! ## Validation layers (§3.1)
//!
//! 1. `serde` + `deny_unknown_fields` — structural, enforced at deserialization.
//! 2. This plugin's [`validate`] — semantic, cross-field, PF2e-specific.
//! 3. Platform-side (Foundry DataModel) — adapter-run, not our concern here (§7.6).

mod content;
mod derived;
mod validation;

use schema_core::character::CharacterBuild;
use schema_core::newtypes::SystemId;
use schema_core::{ContentIndex, DerivedStats, SystemPlugin, ValidationReport};

/// The PF2e system plugin (§10.2).
///
/// Construct with [`Pf2ePlugin::new`] or [`Default::default`]. The content
/// index is built once at construction and reused for all lookups.
pub struct Pf2ePlugin {
    content_index: ContentIndex,
}

impl Pf2ePlugin {
    pub fn new() -> Self {
        Self {
            content_index: content::build(),
        }
    }
}

impl Default for Pf2ePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPlugin for Pf2ePlugin {
    fn id(&self) -> SystemId {
        SystemId::new("pf2e")
    }

    fn json_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CharacterBuild)
    }

    fn validate(&self, build: &CharacterBuild) -> ValidationReport {
        validation::validate(build)
    }

    fn derived_stats(&self, build: &CharacterBuild) -> DerivedStats {
        derived::compute(build, &self.content_index)
    }

    fn content_index(&self) -> &ContentIndex {
        &self.content_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::testing::minimal_build;

    #[test]
    fn id_is_pf2e() {
        let plugin = Pf2ePlugin::new();
        assert_eq!(plugin.id().as_str(), "pf2e");
    }

    #[test]
    fn json_schema_is_non_empty() {
        let plugin = Pf2ePlugin::new();
        let mut schema = plugin.json_schema();
        assert!(!schema.schema.object().properties.is_empty());
    }

    #[test]
    fn content_index_has_known_ancestries() {
        let plugin = Pf2ePlugin::new();
        let idx = plugin.content_index();
        assert!(idx.get(&schema_core::Slug::new("human")).is_some());
        assert!(idx.get(&schema_core::Slug::new("elf")).is_some());
        assert!(idx.get(&schema_core::Slug::new("dwarf")).is_some());
    }

    #[test]
    fn validate_accepts_minimal_build() {
        let plugin = Pf2ePlugin::new();
        let report = plugin.validate(&minimal_build());
        assert!(
            report.is_valid(),
            "minimal build should be valid, got: {:?}",
            report.errors
        );
    }

    #[test]
    fn derived_stats_for_minimal_build() {
        let plugin = Pf2ePlugin::new();
        let stats = plugin.derived_stats(&minimal_build());
        // Fighter 10 HP/level × 1 + human ancestry 8 = 18
        assert_eq!(stats.max_hp, 18);
    }

    #[test]
    fn registry_lookup() {
        use schema_core::SystemPluginRegistry;

        let mut registry = SystemPluginRegistry::new();
        registry.register(Box::new(Pf2ePlugin::new()));

        let plugin = registry
            .get(&SystemId::new("pf2e"))
            .expect("pf2e should be registered");
        assert_eq!(plugin.id().as_str(), "pf2e");

        assert!(registry.get(&SystemId::new("sf2e")).is_none());
    }
}
