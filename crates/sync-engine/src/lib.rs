//! Diff, merge, and snapshot DAG — pure and I/O-free (§1, §11).
//!
//! `merge()` is a pure function of three `CharacterBuild`s: no database, no
//! async, no adapter (§11.3). This is what makes the highest-risk code in the
//! project exhaustively property-testable (§16) and is the concrete reason
//! sync-engine is I/O-free.
//!
//! ## Load-bearing invariants (§11)
//!
//! - `merge` is a pure function of three `CharacterBuild`s — no I/O, no async.
//! - Conflicts are never auto-resolved (§11.3): changed on both sides to
//!   different values → conflict, returned for a human.
//! - Diffing walks the schema structurally, with identity-matched collection
//!   pairing (§11.2) so inserting one feat mid-list doesn't report the whole
//!   tail as changed.
//! - Rollback is `revert`, not `reset --hard` (§11.4): history is an immutable
//!   snapshot DAG, never destructively rewritten.

pub mod diff;
pub mod json;
pub mod merge;
pub mod snapshot;

pub use diff::{diff, Change, FieldDiff};
pub use json::{
    apply_merge, field_diff_to_json, field_diffs_to_json, merge_preview_from_json,
    merge_preview_to_json, remove_at_path, resolutions_from_json, set_at_path, SyncError,
};
pub use merge::{merge, Conflict, MergePreview, Resolution};
pub use snapshot::{Snapshot, SnapshotOrigin};
