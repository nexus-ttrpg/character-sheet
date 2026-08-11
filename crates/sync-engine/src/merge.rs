//! Three-way merge — the pure, I/O-free heart of the sync engine (§11.3).
//!
//! `merge` is a **pure function of three `CharacterBuild`s**: no database, no
//! `async`, no adapter (§11.3). This is what makes the highest-risk code in the
//! project exhaustively property-testable (§16) and is the concrete reason
//! `sync-engine` is I/O-free.
//!
//! ## Algorithm (§11.3)
//!
//! 1. Compute `diff(ancestor, ours)` and `diff(ancestor, theirs)`.
//! 2. Walk both diff lists by canonical path:
//!    - changed on one side only → take it (`auto_merged`).
//!    - changed on both to the same value → no-op (not even reported).
//!    - changed on both to different values → `conflict` (never auto-resolved).
//! 3. Return a `MergePreview`. Conflicts are returned for a human to resolve
//!    per field before the merge snapshot commits (§11.3).
//!
//! ## Load-bearing invariants
//!
//! - **Conflicts are never auto-resolved** (§11.3): changed on both sides to
//!   different values → conflict, returned for a human.
//! - `merge` takes `&CharacterBuild` — it structurally cannot see `session`
//!   (§3.1, §4). The partition *is* the policy.
//! - Identical inputs never produce conflicts (§21 risk row) — `merge(a, x, x)`
//!   is conflict-free because both sides changed identically.

use std::collections::BTreeMap;

use schema_core::character::CharacterBuild;
use schema_core::newtypes::FieldPath;
use serde_json::Value;

use crate::diff::{diff, Change, FieldDiff};

/// A field-level conflict: both sides changed the same path to different values
/// (§11.3). Never auto-resolved — returned for a human.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub path: FieldPath,
    pub ancestor: Value,
    pub ours: Value,
    pub theirs: Value,
}

/// The result of a three-way merge (§11.3). `auto_merged` carries the diffs
/// that were applied without conflict; `conflicts` carries the ones that need a
/// human. A merge with non-empty `conflicts` must not commit until resolved.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergePreview {
    pub auto_merged: Vec<FieldDiff>,
    pub conflicts: Vec<Conflict>,
}

/// A per-field resolution chosen by a human during conflict resolution
/// (§11.3). Applied to produce the final merged build.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Take our side.
    Ours,
    /// Take their side.
    Theirs,
    /// Take a custom value (e.g. a manual merge of both).
    Custom(Value),
}

/// Three-way merge of two builds against a common ancestor (§11.3).
///
/// Pure: no I/O, no async, no adapter. The signature takes `&CharacterBuild`
/// so it is structurally unable to see `session` (§3.1, §4).
pub fn merge(
    ancestor: &CharacterBuild,
    ours: &CharacterBuild,
    theirs: &CharacterBuild,
) -> MergePreview {
    let ours_diff = diff(ancestor, ours);
    let theirs_diff = diff(ancestor, theirs);

    // Index diffs by path for O(path) lookup. BTreeMap for deterministic
    // output order (the conflict UI shows a stable list).
    let ours_by_path: BTreeMap<&FieldPath, &FieldDiff> =
        ours_diff.iter().map(|d| (&d.path, d)).collect();
    let theirs_by_path: BTreeMap<&FieldPath, &FieldDiff> =
        theirs_diff.iter().map(|d| (&d.path, d)).collect();

    let mut auto_merged: Vec<FieldDiff> = Vec::new();
    let mut conflicts: Vec<Conflict> = Vec::new();

    // Walk the union of changed paths in sorted order (BTreeMap of paths).
    let all_paths: BTreeSet<&FieldPath> = ours_by_path
        .keys()
        .chain(theirs_by_path.keys())
        .copied()
        .collect();

    use std::collections::BTreeSet;

    for path in all_paths {
        let o = ours_by_path.get(path).copied();
        let t = theirs_by_path.get(path).copied();
        match (o, t) {
            (Some(o), None) => {
                // Changed on our side only → take it.
                auto_merged.push(o.clone());
            }
            (None, Some(t)) => {
                // Changed on their side only → take it.
                auto_merged.push(t.clone());
            }
            (Some(o), Some(t)) => {
                // Changed on both sides.
                if o.change == t.change {
                    // Both sides made the identical change → no-op, no conflict
                    // (§21: identical inputs never produce conflicts). This
                    // covers Modified|Modified, Added|Added, Removed|Removed
                    // — any Change variant, since Change derives PartialEq.
                } else {
                    // Changed on both to different values → conflict (§11.3).
                    let ancestor_val = ancestor_value(&o.change);
                    let ours_val = change_after(&o.change).unwrap_or_else(|| ancestor_val.clone());
                    let theirs_val =
                        change_after(&t.change).unwrap_or_else(|| ancestor_val.clone());
                    conflicts.push(Conflict {
                        path: path.clone(),
                        ancestor: ancestor_val,
                        ours: ours_val,
                        theirs: theirs_val,
                    });
                }
            }
            (None, None) => {}
        }
    }

    MergePreview {
        auto_merged,
        conflicts,
    }
}

/// Extract the `after` value of a `Change` (the new value), if any.
fn change_after(c: &Change) -> Option<Value> {
    match c {
        Change::Added { after } | Change::Modified { after, .. } => Some(after.clone()),
        Change::Removed { .. } => None,
    }
}

/// For a conflict, recover the ancestor's value. For a `Modified` change it's
/// the `before`; for an `Added` (field didn't exist in ancestor) it's `Null`;
/// for a `Removed` it's the `before` (the value being removed).
fn ancestor_value(c: &Change) -> Value {
    match c {
        Change::Modified { before, .. } | Change::Removed { before } => before.clone(),
        Change::Added { .. } => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::testing::minimal_build;
    use schema_core::Level;

    #[test]
    fn merge_identical_inputs_is_empty() {
        // §16: merging a build with itself is identity.
        let a = minimal_build();
        let m = merge(&a, &a, &a);
        assert!(m.auto_merged.is_empty());
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn merge_a_xx_never_conflicts() {
        // §16 / §21: merge(a, x, x) never conflicts — both sides changed
        // identically, so each path is a no-op.
        let a = minimal_build();
        let mut x = a.clone();
        x.identity.name = "Changed".into();
        x.level = Level::new(5).unwrap();
        let m = merge(&a, &x, &x);
        assert!(
            m.conflicts.is_empty(),
            "merge(a,x,x) must not conflict: {m:?}"
        );
        // auto_merged is also empty because both sides made the *same* change
        // (no divergence to record).
        assert!(m.auto_merged.is_empty(), "identical changes are no-ops");
    }

    #[test]
    fn single_sided_change_is_auto_merged() {
        // §16: auto-merged fields never lose a change made on exactly one side.
        let a = minimal_build();
        let mut ours = a.clone();
        ours.identity.name = "Ours".into();
        let theirs = a.clone();
        let m = merge(&a, &ours, &theirs);
        assert!(m.conflicts.is_empty());
        assert_eq!(m.auto_merged.len(), 1);
        assert_eq!(m.auto_merged[0].path.as_str(), "build.identity.name");
    }

    #[test]
    fn both_sides_different_values_is_conflict() {
        // §11.3: changed on both to different values → conflict, never
        // auto-resolved.
        let a = minimal_build();
        let mut ours = a.clone();
        ours.identity.name = "Ours".into();
        let mut theirs = a.clone();
        theirs.identity.name = "Theirs".into();
        let m = merge(&a, &ours, &theirs);
        assert_eq!(m.conflicts.len(), 1);
        let c = &m.conflicts[0];
        assert_eq!(c.path.as_str(), "build.identity.name");
        assert_eq!(c.ours, Value::String("Ours".into()));
        assert_eq!(c.theirs, Value::String("Theirs".into()));
        assert!(m.auto_merged.is_empty());
    }

    #[test]
    fn non_overlapping_changes_both_auto_merge() {
        // Two single-sided changes on different fields both auto-merge.
        let a = minimal_build();
        let mut ours = a.clone();
        ours.identity.name = "OursName".into();
        let mut theirs = a.clone();
        theirs.level = Level::new(7).unwrap();
        let m = merge(&a, &ours, &theirs);
        assert!(m.conflicts.is_empty());
        assert_eq!(m.auto_merged.len(), 2);
    }

    #[test]
    fn identical_inputs_never_produce_conflicts() {
        // §21 risk row: identical inputs never produce conflicts. Stress with
        // a few identical-input variants.
        let a = minimal_build();
        for _ in 0..10 {
            let m = merge(&a, &a, &a);
            assert!(m.conflicts.is_empty());
        }
    }

    #[test]
    fn one_side_unchanged_other_changed_no_conflict() {
        // ours == ancestor, theirs diverged → auto-merge theirs, no conflict.
        let a = minimal_build();
        let mut theirs = a.clone();
        theirs.identity.name = "TheirsOnly".into();
        let m = merge(&a, &a, &theirs);
        assert!(m.conflicts.is_empty());
        assert_eq!(m.auto_merged.len(), 1);
    }
}
