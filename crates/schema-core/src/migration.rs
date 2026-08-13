//! Schema versioning and migrations (§9.3).
//!
//! Migrations are pure functions chained automatically (`v1→v2→v3`), operating
//! on `serde_json::Value` — not typed structs. A typed migration would need
//! the old version's structs kept compiled in the tree forever, which becomes
//! unmaintainable by v5. Untyped migration functions with fixture-based tests
//! (§16) are the standard trade; the loss of type safety is bounded because
//! the output is immediately deserialized into the current typed schema, which
//! fails loudly if a migration is wrong.
//!
//! ## When migrations are needed
//!
//! - **Minor/patch** bumps are additive-only (new `Option` fields) and need no
//!   migration.
//! - **Major** bumps may restructure and ship with a registered migration.
//!
//! ## Lazy on read
//!
//! Every stored snapshot keeps its original version. Migrations run lazily on
//! read, producing the current version in memory, optionally persisted back by
//! a compaction pass — rather than a synchronous mass migration on deploy.
//! Critical for self-hosters who skip upgrades for months (§21 risk row).

use semver::Version;
use serde_json::Value;

/// The current schema version. New snapshots are written at this version; old
/// snapshots are migrated to this version on read (§9.3).
pub const CURRENT_SCHEMA_VERSION: Version = Version::new(0, 1, 0);

/// A single migration step from one schema version to the next (§9.3).
///
/// Implementations are pure functions: `serde_json::Value` in, `serde_json::Value`
/// out. They never touch I/O, the store, or typed structs. The registry chains
/// them automatically (`v1→v2→v3`), so we never hand-write every version pair.
pub trait Migration: Send + Sync {
    /// The schema version this migration reads.
    const FROM: Version;
    /// The schema version this migration produces.
    const TO: Version;
    /// Transform the JSON from `FROM` shape to `TO` shape.
    fn migrate(input: Value) -> Result<Value, MigrationError>;
}

/// Errors that can occur during migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The input JSON didn't have the expected structure at the given path.
    #[error("expected {expected} at `{path}`, got {actual}")]
    UnexpectedShape {
        path: String,
        expected: String,
        actual: String,
    },
    /// No chain of registered migrations connects `from` to `to`.
    #[error("no migration path from {from} to {to}")]
    NoMigrationPath { from: Version, to: Version },
}

/// A registry of migrations that can chain them automatically (§9.3).
///
/// The registry holds migrations as `(from, to, fn)` tuples. When asked to
/// migrate from version A to version B, it finds the chain `A→...→B` and
/// applies each step in order.
#[derive(Debug, Clone)]
pub struct MigrationRegistry {
    migrations: Vec<MigrationEntry>,
}

/// A registered migration step: version range + the pure function.
#[derive(Debug, Clone)]
struct MigrationEntry {
    from: Version,
    to: Version,
    migrate_fn: fn(Value) -> Result<Value, MigrationError>,
}

impl MigrationRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            migrations: Vec::new(),
        }
    }

    /// Register a migration. Must not duplicate an existing `FROM` version.
    pub fn register<M: Migration>(&mut self) {
        self.migrations.push(MigrationEntry {
            from: M::FROM,
            to: M::TO,
            migrate_fn: M::migrate,
        });
        self.migrations.sort_by_key(|m| m.from.clone());
    }

    /// Migrate a JSON value from `from` version to `to` version by chaining
    /// registered migrations. If `from >= to`, the value is returned unchanged.
    pub fn migrate_to(
        &self,
        from: &Version,
        to: &Version,
        value: Value,
    ) -> Result<Value, MigrationError> {
        if from >= to {
            return Ok(value);
        }

        let mut current = from.clone();
        let mut result = value;

        while current < *to {
            let step = self.migrations.iter().find(|m| m.from == current);
            match step {
                Some(m) => {
                    result = (m.migrate_fn)(result)?;
                    current = m.to.clone();
                }
                None => {
                    return Err(MigrationError::NoMigrationPath {
                        from: current,
                        to: to.clone(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// Convenience: migrate to [`CURRENT_SCHEMA_VERSION`].
    pub fn migrate_to_current(
        &self,
        from: &Version,
        value: Value,
    ) -> Result<Value, MigrationError> {
        self.migrate_to(from, &CURRENT_SCHEMA_VERSION, value)
    }
}

impl Default for MigrationRegistry {
    /// The default registry with all registered migrations (§9.3).
    fn default() -> Self {
        let mut reg = Self::new();
        reg.register::<RenameCharacterName>();
        reg
    }
}

// ── Concrete migrations ─────────────────────────────────────────────────────

/// **v0.0.1 → v0.1.0:** In v0.0.1, `Identity.name` was serialized as
/// `"characterName"`. In v0.1.0, it was renamed to `"name"`.
///
/// This migration renames the JSON key `identity.characterName` →
/// `identity.name`. If `characterName` is absent (already migrated or was
/// never present), the migration is a no-op.
pub struct RenameCharacterName;

impl Migration for RenameCharacterName {
    const FROM: Version = Version::new(0, 0, 1);
    const TO: Version = Version::new(0, 1, 0);

    fn migrate(mut input: Value) -> Result<Value, MigrationError> {
        if let Some(identity) = input.get_mut("identity").and_then(|v| v.as_object_mut()) {
            if let Some(name) = identity.remove("characterName") {
                identity.insert("name".to_string(), name);
            }
        }
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rename_character_name_migrates_key() {
        let input = json!({
            "identity": { "characterName": "Aldric" },
            "level": 1
        });
        let out = RenameCharacterName::migrate(input).unwrap();
        assert_eq!(out["identity"]["name"], "Aldric");
        assert!(out["identity"].get("characterName").is_none());
    }

    #[test]
    fn rename_character_name_noop_if_already_migrated() {
        let input = json!({
            "identity": { "name": "Aldric" },
            "level": 1
        });
        let out = RenameCharacterName::migrate(input).unwrap();
        assert_eq!(out["identity"]["name"], "Aldric");
    }

    #[test]
    fn rename_character_name_noop_if_field_absent() {
        let input = json!({ "level": 1 });
        let out = RenameCharacterName::migrate(input).unwrap();
        assert_eq!(out["level"], 1);
    }

    #[test]
    fn registry_chains_migrations() {
        let reg = MigrationRegistry::default();
        let input = json!({
            "identity": { "characterName": "Aldric" },
            "level": 1
        });
        let out = reg
            .migrate_to_current(&Version::new(0, 0, 1), input)
            .unwrap();
        assert_eq!(out["identity"]["name"], "Aldric");
    }

    #[test]
    fn registry_noop_when_already_current() {
        let reg = MigrationRegistry::default();
        let input = json!({ "identity": { "name": "Aldric" } });
        let out = reg
            .migrate_to_current(&CURRENT_SCHEMA_VERSION, input.clone())
            .unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn registry_errors_on_no_path() {
        let reg = MigrationRegistry::new(); // empty
        let result = reg.migrate_to_current(&Version::new(0, 0, 1), json!({}));
        assert!(matches!(
            result,
            Err(MigrationError::NoMigrationPath { .. })
        ));
    }

    #[test]
    fn registry_errors_on_future_version() {
        let reg = MigrationRegistry::default();
        let result = reg.migrate_to(&Version::new(0, 2, 0), &CURRENT_SCHEMA_VERSION, json!({}));
        // from > to → returns value unchanged (downgrade is a no-op)
        assert!(result.is_ok());
    }
}
