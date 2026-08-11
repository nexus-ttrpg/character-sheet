//! Snapshot DAG (§11.1) and rollback-as-`revert` (§11.4).
//!
//! Every Push, Pull, or Import creates an immutable snapshot, forming a DAG in
//! the spirit of Git commits. Current canonical state is whatever `HEAD`
//! points at.
//!
//! `SnapshotOrigin` as an enum rather than a string: any future code that
//! switches on provenance gets a compile error when a new origin is added
//! (§11.1).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use schema_core::{AdapterId, CharacterBuild, ContentHash, Ulid};

/// Provenance of a snapshot (§11.1). Exhaustive enum so a new sync path can't
/// silently fall into a default branch and be mis-attributed in history.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum SnapshotOrigin {
    ManualEdit,
    Import {
        adapter: AdapterId,
    },
    Merge {
        resolved_conflicts: u32,
    },
    Migration {
        #[schemars(schema_with = "schema_semver")]
        from: semver::Version,
    },
    /// Rollback carries the snapshot id being reverted to (§11.4). Records a
    /// *new* snapshot — Git's `revert`, not `reset --hard`. History is never
    /// destructively rewritten.
    Rollback {
        to: Ulid,
    },
}

/// JSON Schema for `semver::Version` — a semver string (§9.3). Mirrors
/// `schema_core::character`'s helper; `semver::Version` has no `JsonSchema`
/// impl, so we describe it as a string.
fn schema_semver(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "string",
        "description": "Semantic version (semver). Minor/patch are additive; major may restructure (§9.3)."
    }))
    .expect("valid schema")
}

/// An immutable snapshot in the history DAG (§11.1).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub id: Ulid,
    pub character_id: Ulid,
    pub parent: Option<Ulid>,
    /// `Some(_)` only for merge commits (§11.1).
    pub merge_parent: Option<Ulid>,
    pub build: CharacterBuild,
    pub content_hash: ContentHash,
    pub origin: SnapshotOrigin,
    pub created_at: DateTime<Utc>,
    pub created_by: Ulid,
}

impl Snapshot {
    /// Construct a new head snapshot from a build, carrying forward the
    /// character id and recording provenance. The store (Phase 0.6) assigns
    /// the `id` and `created_at`; here we provide a pure constructor for tests
    /// and the engine.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Ulid,
        character_id: Ulid,
        parent: Option<Ulid>,
        build: CharacterBuild,
        origin: SnapshotOrigin,
        created_at: DateTime<Utc>,
        created_by: Ulid,
    ) -> Self {
        let content_hash = schema_core::content_hash(&build);
        Self {
            id,
            character_id,
            parent,
            merge_parent: None,
            build,
            content_hash,
            origin,
            created_at,
            created_by,
        }
    }

    /// Construct a merge snapshot with both parents set (§11.1).
    #[allow(clippy::too_many_arguments)]
    pub fn new_merge(
        id: Ulid,
        character_id: Ulid,
        parent: Option<Ulid>,
        merge_parent: Ulid,
        build: CharacterBuild,
        resolved_conflicts: u32,
        created_at: DateTime<Utc>,
        created_by: Ulid,
    ) -> Self {
        let content_hash = schema_core::content_hash(&build);
        Self {
            id,
            character_id,
            parent,
            merge_parent: Some(merge_parent),
            build,
            content_hash,
            origin: SnapshotOrigin::Merge { resolved_conflicts },
            created_at,
            created_by,
        }
    }

    /// Construct a rollback snapshot (§11.4). Records a *new* snapshot carrying
    /// the old content — `revert` semantics, not `reset --hard`.
    pub fn new_rollback(
        id: Ulid,
        character_id: Ulid,
        parent: Option<Ulid>,
        build: CharacterBuild,
        to: Ulid,
        created_at: DateTime<Utc>,
        created_by: Ulid,
    ) -> Self {
        let content_hash = schema_core::content_hash(&build);
        Self {
            id,
            character_id,
            parent,
            merge_parent: None,
            build,
            content_hash,
            origin: SnapshotOrigin::Rollback { to },
            created_at,
            created_by,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::testing::minimal_build;

    fn ulid(s: &str) -> Ulid {
        Ulid::new(s)
    }

    #[test]
    fn snapshot_records_content_hash() {
        let build = minimal_build();
        let snap = Snapshot::new(
            ulid("01J8Z3F9K2N4Q6R8S0T2V4W6X8"),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V7"),
            None,
            build,
            SnapshotOrigin::ManualEdit,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V8"),
        );
        assert_eq!(snap.content_hash, schema_core::content_hash(&snap.build));
    }

    #[test]
    fn merge_snapshot_sets_merge_parent() {
        let snap = Snapshot::new_merge(
            ulid("01J8Z3F9K2N4Q6R8S0T2V4W6X9"),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V7"),
            Some(ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V8")),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V9"),
            minimal_build(),
            2,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5VA"),
        );
        assert_eq!(snap.merge_parent, Some(ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V9")));
        assert!(matches!(
            snap.origin,
            SnapshotOrigin::Merge {
                resolved_conflicts: 2
            }
        ));
    }

    #[test]
    fn rollback_snapshot_carries_revert_origin() {
        let snap = Snapshot::new_rollback(
            ulid("01J8Z3F9K2N4Q6R8S0T2V4W6XA"),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V7"),
            Some(ulid("01J8Z3FAK1M3P5Q7R9S1T3U5V8")),
            minimal_build(),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5VB"),
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            ulid("01J8Z3FAK1M3P5Q7R9S1T3U5VC"),
        );
        assert!(
            matches!(snap.origin, SnapshotOrigin::Rollback { to } if to == ulid("01J8Z3FAK1M3P5Q7R9S1T3U5VB"))
        );
    }
}
