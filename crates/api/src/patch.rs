//! Path-based patching and diff serialization helpers.
//!
//! The PATCH endpoint (§13.2) applies a single field edit by canonical path
//! (e.g. `"build.skills.athletics.rank"`). The merge resolution flow (§11.3)
//! applies resolved values at conflict paths. Both need to navigate a
//! `serde_json::Value` by canonical field path and set/remove a value.
//!
//! Path formats produced by the diff engine (§11.2):
//! - `"build.identity.name"` — dotted object keys
//! - `"build.feats[toughness]"` — array element by identity slug
//! - `"build.spellcasting.entries[0]"` — array element by numeric index
//! - `"build.skills.athletics"` — map key

use serde_json::Value;

use crate::error::{ApiError, ApiResult};

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
pub fn set_at_path(root: &mut Value, path: &str, value: Value) -> ApiResult<()> {
    let steps = parse_path(path);
    set_steps(root, &steps, value)
}

fn set_steps(root: &mut Value, steps: &[PathStep], value: Value) -> ApiResult<()> {
    if steps.is_empty() {
        *root = value;
        return Ok(());
    }
    match &steps[0] {
        PathStep::Key(k) => {
            let obj = root
                .as_object_mut()
                .ok_or_else(|| ApiError::BadRequest(format!("expected object at '{k}'")))?;
            if !obj.contains_key(k) {
                return Err(ApiError::BadRequest(format!("key '{k}' not found")));
            }
            set_steps(&mut obj[k], &steps[1..], value)
        }
        PathStep::Index(i) => {
            if let Ok(idx) = i.parse::<usize>() {
                let arr = root
                    .as_array_mut()
                    .ok_or_else(|| ApiError::BadRequest(format!("expected array at '[{i}]'")))?;
                if idx >= arr.len() {
                    return Err(ApiError::BadRequest(format!(
                        "index {idx} out of bounds (len {})",
                        arr.len()
                    )));
                }
                set_steps(&mut arr[idx], &steps[1..], value)
            } else {
                let arr = root
                    .as_array_mut()
                    .ok_or_else(|| ApiError::BadRequest(format!("expected array at '[{i}]'")))?;
                let found = arr.iter_mut().find(|item| {
                    item.get("reference")
                        .and_then(|r| r.get("slug"))
                        .and_then(|s| s.as_str())
                        == Some(i.as_str())
                });
                match found {
                    Some(item) => set_steps(item, &steps[1..], value),
                    None => Err(ApiError::BadRequest(format!(
                        "element with slug '{i}' not found"
                    ))),
                }
            }
        }
    }
}

/// Remove the value at a canonical field path (for `Change::Removed`).
pub fn remove_at_path(root: &mut Value, path: &str) -> ApiResult<()> {
    let steps = parse_path(path);
    remove_steps(root, &steps)
}

fn remove_steps(root: &mut Value, steps: &[PathStep]) -> ApiResult<()> {
    if steps.is_empty() {
        return Ok(());
    }
    if steps.len() == 1 {
        match &steps[0] {
            PathStep::Key(k) => {
                let obj = root
                    .as_object_mut()
                    .ok_or_else(|| ApiError::BadRequest("expected object".into()))?;
                if obj.remove(k).is_none() {
                    return Err(ApiError::BadRequest(format!("key '{k}' not found")));
                }
                Ok(())
            }
            PathStep::Index(i) => {
                if let Ok(idx) = i.parse::<usize>() {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| ApiError::BadRequest("expected array".into()))?;
                    if idx >= arr.len() {
                        return Err(ApiError::BadRequest(format!("index {idx} out of bounds")));
                    }
                    arr.remove(idx);
                    Ok(())
                } else {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| ApiError::BadRequest("expected array".into()))?;
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
                        None => Err(ApiError::BadRequest(format!(
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
                    .ok_or_else(|| ApiError::BadRequest("expected object".into()))?;
                if !obj.contains_key(k) {
                    return Err(ApiError::BadRequest(format!("key '{k}' not found")));
                }
                remove_steps(&mut obj[k], &steps[1..])
            }
            PathStep::Index(i) => {
                if let Ok(idx) = i.parse::<usize>() {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| ApiError::BadRequest("expected array".into()))?;
                    if idx >= arr.len() {
                        return Err(ApiError::BadRequest(format!("index {idx} out of bounds")));
                    }
                    remove_steps(&mut arr[idx], &steps[1..])
                } else {
                    let arr = root
                        .as_array_mut()
                        .ok_or_else(|| ApiError::BadRequest("expected array".into()))?;
                    let found = arr.iter_mut().find(|item| {
                        item.get("reference")
                            .and_then(|r| r.get("slug"))
                            .and_then(|s| s.as_str())
                            == Some(i.as_str())
                    });
                    match found {
                        Some(item) => remove_steps(item, &steps[1..]),
                        None => Err(ApiError::BadRequest(format!(
                            "element with slug '{i}' not found"
                        ))),
                    }
                }
            }
        }
    }
}

// ── Diff serialization ──────────────────────────────────────────────────────

/// Convert a `sync_engine::FieldDiff` to the JSON shape used in API responses
/// (§13.2, §13.3). The diff types don't derive `Serialize`, so we map them
/// manually to the wire format.
pub fn field_diff_to_json(d: &sync_engine::FieldDiff) -> Value {
    match &d.change {
        sync_engine::Change::Added { after } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "added",
            "after": after,
        }),
        sync_engine::Change::Removed { before } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "removed",
            "before": before,
        }),
        sync_engine::Change::Modified { before, after } => serde_json::json!({
            "path": d.path.as_str(),
            "change": "modified",
            "before": before,
            "after": after,
        }),
    }
}

pub fn field_diffs_to_json(diffs: &[sync_engine::FieldDiff]) -> Vec<Value> {
    diffs.iter().map(field_diff_to_json).collect()
}

// ── MergePreview serialization ──────────────────────────────────────────────

/// Serialize a `MergePreview` to the wire format (§13.3).
pub fn merge_preview_to_json(preview: &sync_engine::MergePreview) -> Value {
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
    preview: &sync_engine::MergePreview,
    resolutions: &[(String, sync_engine::Resolution)],
) -> ApiResult<Value> {
    let mut result = ours.clone();

    // Apply auto-merged diffs.
    for d in &preview.auto_merged {
        match &d.change {
            sync_engine::Change::Added { after } | sync_engine::Change::Modified { after, .. } => {
                set_at_path(&mut result, d.path.as_str(), after.clone())?;
            }
            sync_engine::Change::Removed { .. } => {
                remove_at_path(&mut result, d.path.as_str())?;
            }
        }
    }

    // Apply resolved conflicts.
    for (path, resolution) in resolutions {
        let value = match resolution {
            sync_engine::Resolution::Ours => {
                let conflict = preview
                    .conflicts
                    .iter()
                    .find(|c| c.path.as_str() == path.as_str())
                    .ok_or_else(|| {
                        ApiError::BadRequest(format!(
                            "resolution for non-existent conflict path: {path}"
                        ))
                    })?;
                conflict.ours.clone()
            }
            sync_engine::Resolution::Theirs => {
                let conflict = preview
                    .conflicts
                    .iter()
                    .find(|c| c.path.as_str() == path.as_str())
                    .ok_or_else(|| {
                        ApiError::BadRequest(format!(
                            "resolution for non-existent conflict path: {path}"
                        ))
                    })?;
                conflict.theirs.clone()
            }
            sync_engine::Resolution::Custom(v) => v.clone(),
        };
        set_at_path(&mut result, path, value)?;
    }

    Ok(result)
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
    fn missing_key_returns_bad_request() {
        let mut val = json!({"build": {}});
        let err = set_at_path(&mut val, "build.nonexistent", json!("x")).unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }
}
