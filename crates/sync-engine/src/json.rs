//! Path-based patching and diff serialization — the JSON boundary layer
//! (§13.2, §13.3).
//!
//! This module owns the single implementation of:
//! - canonical-path navigation over `serde_json::Value` (`set_at_path`,
//!   `remove_at_path`),
//! - diff → wire-JSON serialization (`field_diff_to_json`,
//!   `field_diffs_to_json`, `merge_preview_to_json`),
//! - merge application (`apply_merge`).
//!
//! Living in `sync-engine` (not `api`) means the server and a future WASM
//! module share ONE implementation — parity is structural, not test-enforced.
//!
//! Path formats produced by the diff engine (§11.2):
//! - `"build.identity.name"` — dotted object keys
//! - `"build.feats[toughness]"` — array element by identity slug
//! - `"build.spellcasting.entries[0]"` — array element by numeric index
//! - `"build.skills.athletics"` — map key

use schema_core::newtypes::FieldPath;
use serde_json::Value;

use crate::diff::{Change, FieldDiff};
use crate::merge::{Conflict, MergePreview, Resolution};

// ── Error ───────────────────────────────────────────────────────────────────

/// Errors from path navigation and merge application.
///
/// All variants represent malformed paths or resolutions in a client request
/// and map to `ApiError::BadRequest` (HTTP 400) at the api boundary — they are
/// not internal failures.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// A canonical field path could not be navigated: the JSON value at some
    /// step was the wrong type, or a key/index/slug was not found. The string
    /// carries the diagnostic message.
    #[error("{0}")]
    PathNavigation(String),

    /// A merge resolution referenced a conflict path absent from the preview.
    #[error("resolution for non-existent conflict path: {0}")]
    UnknownConflictPath(String),
}

// ── Path tokenization ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum PathStep {
    Key(String),
    Index(String),
}

/// Tokenize a canonical field path into navigation steps.
fn parse_path(path: &str) -> Vec<PathStep> {
    let mut steps = Vec::new();
    for segment in path.split('.') {
        let mut rest = segment;
        if let Some(bracket_pos) = rest.find('[') {
            let key = &rest[..bracket_pos];
            if !key.is_empty() {
                steps.push(PathStep::Key(key.to_string()));
            }
            rest = &rest[bracket_pos..];
            while rest.starts_with('[') {
                if let Some(end) = rest.find(']') {
                    let index = &rest[1..end];
                    steps.push(PathStep::Index(index.to_string()));
                    rest = &rest[end + 1..];
                } else {
                    break;
                }
            }
        } else if !rest.is_empty() {
            steps.push(PathStep::Key(rest.to_string()));
        }
    }
    steps
}

/// Set a value at a canonical field path within a `serde_json::Value`.
pub fn set_at_path(root: &mut Value, path: &str, value: Value) -> Result<(), SyncError> {
    let steps = parse_path(path);
    set_steps(root, &steps, value)
}

fn set_steps(root: &mut Value, steps: &[PathStep], value: Value) -> Result<(), SyncError> {
    if steps.is_empty() {
        *root = value;
        return Ok(());
    }
    match &steps[0] {
        PathStep::Key(k) => {
            let obj = root
                .as_object_mut()
                .ok_or_else(|| SyncError::PathNavigation(format!("expected object at '{k}'")))?;
            if !obj.contains_key(k) {
                return Err(SyncError::PathNavigation(format!("key '{k}' not found")));
            }
            set_steps(&mut obj[k], &steps[1..], value)
        }
        PathStep::Index(i) => {
            if let Ok(idx) = i.parse::<usize>() {
                let arr = root.as_array_mut().ok_or_else(|| {
                    SyncError::PathNavigation(format!("expected array at '[{i}]'"))
                })?;
                if idx >= arr.len() {
                    return Err(SyncError::PathNavigation(format!(
                        "index {idx} out of bounds (len {})",
                        arr.len()
                    )));
                }
                set_steps(&mut arr[idx], &steps[1..], value)
            } else {
                let arr = root.as_array_mut().ok_or_else(|| {
                    SyncError::PathNavigation(format!("expected array at '[{i}]'"))
                })?;
                let found = arr.iter_mut().find(|item| {
                    item.get("reference")
                        .and_then(|r| r.get("slug"))
                        .and_then(|s| s.as_str())
                        == Some(i.as_str())
                });
                match found {
                    Some(item) => set_steps(item, &steps[1..], value),
                    None => Err(SyncError::PathNavigation(format!(
                        "element with slug '{i}' not found"
                    ))),
                }
            }
        }
    }
}

/// Remove the value at a canonical field path (for `Change::Removed`).
pub fn remove_at_path(root: &mut Value, path: &str) -> Result<(), SyncError> {
    let steps = parse_path(path);
    remove_steps(root, &steps)
}

fn remove_steps(root: &mut Value, steps: &[PathStep]) -> Result<(), SyncError> {
    if steps.is_empty() {
        return Ok(());
    }
    if steps.len() == 1 {
        match &steps[0] {
            PathStep::Key(k) => {
                let obj = root
                    .as_object_mut()
                    .ok_or_else(|| SyncError::PathNavigation("expected object".into()))?;
                if obj.remove(k).is_none() {
                    return Err(SyncError::PathNavigation(format!("key '{k}' not found")));
                }
                Ok(())
            }
            PathStep::Index(i) => {
                if let Ok(idx) = i.parse::<usize>() {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| SyncError::PathNavigation("expected array".into()))?;
                    if idx >= arr.len() {
                        return Err(SyncError::PathNavigation(format!(
                            "index {idx} out of bounds"
                        )));
                    }
                    arr.remove(idx);
                    Ok(())
                } else {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| SyncError::PathNavigation("expected array".into()))?;
                    let pos = arr.iter().position(|item| {
                        item.get("reference")
                            .and_then(|r| r.get("slug"))
                            .and_then(|s| s.as_str())
                            == Some(i.as_str())
                    });
                    match pos {
                        Some(p) => {
                            arr.remove(p);
                            Ok(())
                        }
                        None => Err(SyncError::PathNavigation(format!(
                            "element with slug '{i}' not found"
                        ))),
                    }
                }
            }
        }
    } else {
        match &steps[0] {
            PathStep::Key(k) => {
                let obj = root
                    .as_object_mut()
                    .ok_or_else(|| SyncError::PathNavigation("expected object".into()))?;
                if !obj.contains_key(k) {
                    return Err(SyncError::PathNavigation(format!("key '{k}' not found")));
                }
                remove_steps(&mut obj[k], &steps[1..])
            }
            PathStep::Index(i) => {
                if let Ok(idx) = i.parse::<usize>() {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| SyncError::PathNavigation("expected array".into()))?;
                    if idx >= arr.len() {
                        return Err(SyncError::PathNavigation(format!(
                            "index {idx} out of bounds"
                        )));
                    }
                    remove_steps(&mut arr[idx], &steps[1..])
                } else {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| SyncError::PathNavigation("expected array".into()))?;
                    let found = arr.iter_mut().find(|item| {
                        item.get("reference")
                            .and_then(|r| r.get("slug"))
                            .and_then(|s| s.as_str())
                            == Some(i.as_str())
                    });
                    match found {
                        Some(item) => remove_steps(item, &steps[1..]),
                        None => Err(SyncError::PathNavigation(format!(
                            "element with slug '{i}' not found"
                        ))),
                    }
                }
            }
        }
    }
}

// ── Diff serialization ──────────────────────────────────────────────────────

/// Convert a `FieldDiff` to the JSON shape used in API responses
/// (§13.2, §13.3). The diff types don't derive `Serialize`, so we map them
/// manually to the wire format.
pub fn field_diff_to_json(d: &FieldDiff) -> Value {
    match &d.change {
        Change::Added { after } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "added",
            "after": after,
        }),
        Change::Removed { before } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "removed",
            "before": before,
        }),
        Change::Modified { before, after } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "modified",
            "before": before,
            "after": after,
        }),
    }
}

pub fn field_diffs_to_json(diffs: &[FieldDiff]) -> Vec<Value> {
    diffs.iter().map(field_diff_to_json).collect()
}

// ── MergePreview serialization ──────────────────────────────────────────────

/// Serialize a `MergePreview` to the wire format (§13.3).
pub fn merge_preview_to_json(preview: &MergePreview) -> Value {
    serde_json::json!({
        "autoMerged": preview.auto_merged.iter().map(field_diff_to_json).collect::<Vec<_>>(),
        "conflicts": preview.conflicts.iter().map(|c| {
            serde_json::json!({
                "path": c.path.as_str(),
                "ancestor": c.ancestor,
                "ours": c.ours,
                "theirs": c.theirs,
            })
        }).collect::<Vec<_>>(),
    })
}

// ── Merge application ───────────────────────────────────────────────────────

/// Apply a `MergePreview` (auto-merged + resolved conflicts) to `ours`,
/// producing the final merged build as a `serde_json::Value`.
///
/// For auto-merged diffs, the `after` value is set at the diff's path — this
/// is idempotent for ours-side changes (already present) and correct for
/// theirs-side changes (applies theirs' value to ours).
///
/// For resolved conflicts, the resolution determines the value: `"ours"` →
/// ours' value, `"theirs"` → theirs' value, anything else → used as-is.
pub fn apply_merge(
    ours: &Value,
    preview: &MergePreview,
    resolutions: &[(String, Resolution)],
) -> Result<Value, SyncError> {
    let mut result = ours.clone();

    // Apply auto-merged diffs.
    for d in &preview.auto_merged {
        match &d.change {
            Change::Added { after } | Change::Modified { after, .. } => {
                set_at_path(&mut result, d.path.as_str(), after.clone())?;
            }
            Change::Removed { .. } => {
                remove_at_path(&mut result, d.path.as_str())?;
            }
        }
    }

    // Apply resolved conflicts.
    for (path, resolution) in resolutions {
        let value = match resolution {
            Resolution::Ours => {
                let conflict = preview
                    .conflicts
                    .iter()
                    .find(|c| c.path.as_str() == path.as_str())
                    .ok_or_else(|| SyncError::UnknownConflictPath(path.clone()))?;
                conflict.ours.clone()
            }
            Resolution::Theirs => {
                let conflict = preview
                    .conflicts
                    .iter()
                    .find(|c| c.path.as_str() == path.as_str())
                    .ok_or_else(|| SyncError::UnknownConflictPath(path.clone()))?;
                conflict.theirs.clone()
            }
            Resolution::Custom(v) => v.clone(),
        };
        set_at_path(&mut result, path, value)?;
    }

    Ok(result)
}

// ── MergePreview / Resolution deserialization ───────────────────────────────
//
// Reverse of `merge_preview_to_json` and the API's resolution wire format
// (§13.3). Enables a WASM client to round-trip a merge preview through
// `apply_merge` without re-running the merge — the preview JSON is the
// contract between `merge` and `apply_merge`.

/// Parse a `MergePreview` from the wire-format JSON produced by
/// `merge_preview_to_json` (§13.3).
pub fn merge_preview_from_json(json: &Value) -> Result<MergePreview, SyncError> {
    let obj = json
        .as_object()
        .ok_or_else(|| SyncError::PathNavigation("merge preview must be a JSON object".into()))?;

    let auto_merged = parse_field_diffs(
        obj.get("autoMerged")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                SyncError::PathNavigation("merge preview missing 'autoMerged' array".into())
            })?,
    )?;

    let conflicts = obj
        .get("conflicts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| SyncError::PathNavigation("merge preview missing 'conflicts' array".into()))?
        .iter()
        .map(parse_conflict)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MergePreview {
        auto_merged,
        conflicts,
    })
}

/// Parse resolutions from the wire-format JSON array (§13.3).
/// Each entry is `{"path": "...", "resolution": "ours" | "theirs" | <value>}`.
pub fn resolutions_from_json(json: &Value) -> Result<Vec<(String, Resolution)>, SyncError> {
    let arr = json
        .as_array()
        .ok_or_else(|| SyncError::PathNavigation("resolutions must be a JSON array".into()))?;
    arr.iter().map(parse_resolution).collect()
}

fn parse_field_diffs(arr: &[Value]) -> Result<Vec<FieldDiff>, SyncError> {
    arr.iter().map(parse_field_diff).collect()
}

fn parse_field_diff(v: &Value) -> Result<FieldDiff, SyncError> {
    let obj = v
        .as_object()
        .ok_or_else(|| SyncError::PathNavigation("field diff must be a JSON object".into()))?;
    let path_str = obj
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SyncError::PathNavigation("field diff missing 'path'".into()))?;
    let change_str = obj
        .get("change")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SyncError::PathNavigation("field diff missing 'change'".into()))?;
    let change = match change_str {
        "added" => Change::Added {
            after: obj.get("after").cloned().unwrap_or(Value::Null),
        },
        "removed" => Change::Removed {
            before: obj.get("before").cloned().unwrap_or(Value::Null),
        },
        "modified" => Change::Modified {
            before: obj.get("before").cloned().unwrap_or(Value::Null),
            after: obj.get("after").cloned().unwrap_or(Value::Null),
        },
        other => {
            return Err(SyncError::PathNavigation(format!(
                "unknown change type: '{other}'"
            )));
        }
    };
    Ok(FieldDiff {
        path: FieldPath::new(path_str),
        change,
    })
}

fn parse_conflict(v: &Value) -> Result<Conflict, SyncError> {
    let obj = v
        .as_object()
        .ok_or_else(|| SyncError::PathNavigation("conflict must be a JSON object".into()))?;
    let path_str = obj
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SyncError::PathNavigation("conflict missing 'path'".into()))?;
    Ok(Conflict {
        path: FieldPath::new(path_str),
        ancestor: obj.get("ancestor").cloned().unwrap_or(Value::Null),
        ours: obj.get("ours").cloned().unwrap_or(Value::Null),
        theirs: obj.get("theirs").cloned().unwrap_or(Value::Null),
    })
}

fn parse_resolution(v: &Value) -> Result<(String, Resolution), SyncError> {
    let obj = v
        .as_object()
        .ok_or_else(|| SyncError::PathNavigation("resolution must be a JSON object".into()))?;
    let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SyncError::PathNavigation("resolution missing 'path'".into()))?;
    let resolution_val = obj
        .get("resolution")
        .ok_or_else(|| SyncError::PathNavigation("resolution missing 'resolution' field".into()))?;
    let resolution = match resolution_val {
        Value::String(s) if s == "ours" => Resolution::Ours,
        Value::String(s) if s == "theirs" => Resolution::Theirs,
        other => Resolution::Custom(other.clone()),
    };
    Ok((path.to_string(), resolution))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_dotted_path() {
        let mut val = json!({"build": {"identity": {"name": "old"}}});
        set_at_path(&mut val, "build.identity.name", json!("new")).unwrap();
        assert_eq!(val["build"]["identity"]["name"], "new");
    }

    #[test]
    fn set_array_by_slug() {
        let mut val = json!({"build": {"feats": [
            {"reference": {"slug": "a"}, "category": "class"},
            {"reference": {"slug": "b"}, "category": "general"}
        ]}});
        set_at_path(&mut val, "build.feats[b].category", json!("skill")).unwrap();
        assert_eq!(val["build"]["feats"][1]["category"], "skill");
    }

    #[test]
    fn set_array_by_index() {
        let mut val = json!({"build": {"spellcasting": {"entries": [{"tradition": "arcane"}, {"tradition": "divine"}]}}});
        set_at_path(
            &mut val,
            "build.spellcasting.entries[0].tradition",
            json!("primal"),
        )
        .unwrap();
        assert_eq!(
            val["build"]["spellcasting"]["entries"][0]["tradition"],
            "primal"
        );
    }

    #[test]
    fn remove_dotted_path() {
        let mut val = json!({"build": {"identity": {"name": "x", "extra": "y"}}});
        remove_at_path(&mut val, "build.identity.extra").unwrap();
        assert!(val["build"]["identity"].get("extra").is_none());
    }

    #[test]
    fn missing_key_returns_path_error() {
        let mut val = json!({"build": {}});
        let err = set_at_path(&mut val, "build.nonexistent", json!("x")).unwrap_err();
        assert!(matches!(err, SyncError::PathNavigation(_)));
    }

    #[test]
    fn merge_preview_round_trips_through_json() {
        use crate::merge::merge;
        use schema_core::testing::minimal_build;

        let ancestor = minimal_build();
        let mut ours = ancestor.clone();
        ours.identity.name = "Ours".into();
        let mut theirs = ancestor.clone();
        theirs.identity.name = "Theirs".into();

        let preview = merge(&ancestor, &ours, &theirs);
        assert_eq!(preview.conflicts.len(), 1);

        let json = merge_preview_to_json(&preview);
        let back = merge_preview_from_json(&json).expect("round-trip parse");

        assert_eq!(back, preview);
    }

    #[test]
    fn resolutions_round_trip_through_json() {
        let resolutions = vec![
            ("build.identity.name".to_string(), Resolution::Ours),
            ("build.level".to_string(), Resolution::Theirs),
            (
                "build.identity.pronouns".to_string(),
                Resolution::Custom(json!("they/them")),
            ),
        ];
        let json = serde_json::json!(resolutions
            .iter()
            .map(|(p, r)| {
                let res = match r {
                    Resolution::Ours => json!("ours"),
                    Resolution::Theirs => json!("theirs"),
                    Resolution::Custom(v) => v.clone(),
                };
                json!({"path": p, "resolution": res})
            })
            .collect::<Vec<_>>());

        let back = resolutions_from_json(&json).expect("resolution parse");
        assert_eq!(back, resolutions);
    }

    #[test]
    fn parse_unknown_change_type_errors() {
        let bad = json!({"path": "build.x", "change": "transmogrified"});
        let err =
            merge_preview_from_json(&json!({"autoMerged": [bad], "conflicts": []})).unwrap_err();
        assert!(matches!(err, SyncError::PathNavigation(_)));
    }
}
