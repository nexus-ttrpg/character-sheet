//! SF2e SystemPlugin (§3.4, §10.2). Compiled in; extends schema-core.
//!
//! Owns "what is a valid SF2e character, and how are its derived stats
//! computed" (§10.2). The canonical schema types are shared via `schema-core`
//! (§3.4: "SF2e reuses most of the PF2e chassis"; the SF2e-specific *shapes* —
//! drones, Resolve Points, credits/UPB, vehicles — already live in
//! `schema-core`'s `CharacterBuild`/`SessionState`). This plugin adds
//! SF2e-specific validation rules and derived-stat formulas on top, both of
//! which currently delegate to the shared chassis.
//!
//! ## What lives here vs. schema-core
//!
//! - **schema-core** — the canonical *types* plus the shared *chassis* math
//!   (`chassis::derived_stats`, `chassis::validate_chassis`) that PF2e and SF2e
//!   share (§3.4, §10.2).
//! - **schema-sf2e** (this crate) — the SF2e *content index* and the delegation
//!   point for SF2e-specific rules (drones, etc.) as they are added.
//!
//! ## Validation layers (§3.1)
//!
//! 1. `serde` + `deny_unknown_fields` — structural, enforced at deserialization.
//! 2. This plugin's [`validate`] — semantic, cross-field; chassis rules now,
//!    SF2e-specific rules to follow.
//! 3. Platform-side (Foundry DataModel) — adapter-run, not our concern here (§7.6).

mod content;
mod derived;
mod validation;

use schema_core::character::CharacterBuild;
use schema_core::newtypes::SystemId;
use schema_core::{ContentIndex, DerivedStats, SystemPlugin, ValidationReport};

/// The SF2e system plugin (§10.2).
///
/// Construct with [`Sf2ePlugin::new`] or [`Default::default`]. The content
/// index is built once at construction and reused for all lookups.
pub struct Sf2ePlugin {
    content_index: ContentIndex,
}

impl Sf2ePlugin {
    pub fn new() -> Self {
        Self {
            content_index: content::build(),
        }
    }
}

impl Default for Sf2ePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPlugin for Sf2ePlugin {
    fn id(&self) -> SystemId {
        SystemId::new("sf2e")
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
    fn id_is_sf2e() {
        let plugin = Sf2ePlugin::new();
        assert_eq!(plugin.id().as_str(), "sf2e");
    }

    #[test]
    fn json_schema_is_non_empty() {
        let plugin = Sf2ePlugin::new();
        let mut schema = plugin.json_schema();
        assert!(!schema.schema.object().properties.is_empty());
    }

    #[test]
    fn content_index_has_human() {
        let plugin = Sf2ePlugin::new();
        let idx = plugin.content_index();
        assert!(idx.get(&schema_core::Slug::new("human")).is_some());
    }

    #[test]
    fn content_index_has_sf2e_ancestries() {
        let plugin = Sf2ePlugin::new();
        let idx = plugin.content_index();
        for slug in ["human", "kasatha", "lashunta", "shirren", "android", "vesk"] {
            assert!(
                idx.get(&schema_core::Slug::new(slug)).is_some(),
                "content index should have ancestry '{slug}'"
            );
        }
    }

    #[test]
    fn validate_accepts_minimal_build() {
        // `minimal_build()` is a PF2e-flavoured fighter, but it exercises the
        // shared chassis — the SF2e plugin validates it through the same rules.
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&minimal_build());
        assert!(
            report.is_valid(),
            "minimal build should be valid, got: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_sf2e_fixture_is_valid() {
        let json = include_str!("../../../examples/fixtures/sf2e-level5-mechanic.json");
        let build: CharacterBuild = serde_json::from_str(json).unwrap();
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&build);
        assert!(
            report.is_valid(),
            "sf2e-level5-mechanic should be valid, got errors: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_drone_without_chassis_is_warning() {
        let mut build = minimal_build();
        build.drones.push(schema_core::build_types::DroneEntry {
            reference: schema_core::SystemRefId::from_slug("test-drone"),
            chassis: None,
            mods: vec![],
        });
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&build);
        assert!(
            report.is_valid(),
            "drone without chassis should not be an error"
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.message.contains("chassis")),
            "should warn about missing chassis: {:?}",
            report.warnings
        );
    }

    #[test]
    fn validate_drone_mod_empty_slug_is_error() {
        let mut build = minimal_build();
        build.drones.push(schema_core::build_types::DroneEntry {
            reference: schema_core::SystemRefId::from_slug("test-drone"),
            chassis: Some("combat".into()),
            mods: vec![schema_core::SystemRefId::from_slug("")],
        });
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&build);
        assert!(
            !report.is_valid(),
            "drone mod with empty slug should be an error"
        );
    }

    #[test]
    fn validate_crossover_currencies_is_warning() {
        let mut build = minimal_build();
        build.currencies.gp = Some(50);
        build.currencies.credits = Some(100);
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&build);
        assert!(
            report.is_valid(),
            "crossover currencies should not be an error"
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.message.contains("crossover")),
            "should warn about crossover currencies: {:?}",
            report.warnings
        );
    }

    #[test]
    fn validate_sf2e_fixture_has_no_crossover_warning() {
        let json = include_str!("../../../examples/fixtures/sf2e-level5-mechanic.json");
        let build: CharacterBuild = serde_json::from_str(json).unwrap();
        let plugin = Sf2ePlugin::new();
        let report = plugin.validate(&build);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.message.contains("crossover")),
            "SF2e fixture should not trigger crossover currency warning"
        );
    }

    #[test]
    fn derived_stats_for_minimal_build() {
        let plugin = Sf2ePlugin::new();
        let stats = plugin.derived_stats(&minimal_build());
        // Fighter 10 HP/level × 1 + human ancestry 8 = 18 (shared chassis).
        assert_eq!(stats.max_hp, 18);
    }

    #[test]
    fn registry_lookup() {
        use schema_core::SystemPluginRegistry;

        let mut registry = SystemPluginRegistry::new();
        registry.register(Box::new(Sf2ePlugin::new()));

        let plugin = registry
            .get(&SystemId::new("sf2e"))
            .expect("sf2e should be registered");
        assert_eq!(plugin.id().as_str(), "sf2e");

        assert!(registry.get(&SystemId::new("pf2e")).is_none());
    }
}
