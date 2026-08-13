//! WebAssembly bindings for `sync-engine` — browser-side diff/merge preview
//! (Phase 3.1).
//!
//! Exports four functions callable from JS:
//! - `diff(a, b)` — structural diff of two builds (§11.2)
//! - `merge(ancestor, ours, theirs)` — three-way merge preview (§11.3)
//! - `apply_merge(ours, preview, resolutions)` — apply a merge preview +
//!   resolutions to produce the final merged build
//! - `content_hash(build)` — BLAKE3 content hash (§9.2)
//!
//! All functions take JSON strings and return JSON strings. On error they
//! return `{"error": "..."}` — they never panic in WASM (the `start` function
//! installs `console_error_panic_hook` so unexpected panics surface as
//! console errors with stack traces).
//!
//! ## Why a wrapper crate (not a `wasm` feature on sync-engine)
//!
//! Invariant §15 / AGENTS.md #1: `sync-engine` depends on `schema-core` and
//! nothing else — no `wasm-bindgen`, no `cdylib` crate-type. A boundary CI
//! check asserts that dep tree stays clean. Keeping the `wasm-bindgen` dep and
//! `crate-type = ["cdylib"]` in this wrapper crate leaves `sync-engine`'s
//! `Cargo.toml` byte-identical, so the engine's purity invariant and the
//! boundary check are untouched. The wrapper is purely additive.
//!
//! ## Parity
//!
//! The server (via `sync_engine::json`) and this WASM module share ONE
//! implementation of diff serialization, merge preview serialization, and merge
//! application — the wrapper calls the same `sync_engine` functions. Parity is
//! structural, not merely test-enforced. The parity test suite
//! (`parity-test.cjs`) confirms this by comparing WASM output against native
//! output on all fixture pairs.

use schema_core::{content_hash as content_hash_impl, CharacterBuild};
use serde_json::Value;
use sync_engine::{
    apply_merge as apply_merge_impl, diff as diff_builds, field_diffs_to_json,
    merge as merge_builds, merge_preview_from_json, merge_preview_to_json, resolutions_from_json,
};
use wasm_bindgen::prelude::*;

/// Install the panic hook so unexpected panics produce readable console errors
/// instead of opaque `RuntimeError: unreachable`.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
}

/// Structural diff of two builds (§11.2).
///
/// Takes two `CharacterBuild` JSON strings, returns `{"diffs": [...]}` where
/// each diff has the wire format from `field_diff_to_json` (§13.2).
#[wasm_bindgen]
pub fn diff(a_json: &str, b_json: &str) -> String {
    let a: CharacterBuild = match serde_json::from_str(a_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `a`: {e}")),
    };
    let b: CharacterBuild = match serde_json::from_str(b_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `b`: {e}")),
    };
    let diffs = diff_builds(&a, &b);
    let arr = field_diffs_to_json(&diffs);
    match serde_json::to_string(&serde_json::json!({ "diffs": arr })) {
        Ok(s) => s,
        Err(e) => error_json(&format!("failed to serialize diffs: {e}")),
    }
}

/// Three-way merge preview (§11.3).
///
/// Takes three `CharacterBuild` JSON strings (ancestor, ours, theirs), returns
/// `{"mergePreview": {"autoMerged": [...], "conflicts": [...]}}` using the wire
/// format from `merge_preview_to_json` (§13.3).
#[wasm_bindgen]
pub fn merge(ancestor_json: &str, ours_json: &str, theirs_json: &str) -> String {
    let ancestor: CharacterBuild = match serde_json::from_str(ancestor_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `ancestor`: {e}")),
    };
    let ours: CharacterBuild = match serde_json::from_str(ours_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `ours`: {e}")),
    };
    let theirs: CharacterBuild = match serde_json::from_str(theirs_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `theirs`: {e}")),
    };
    let preview = merge_builds(&ancestor, &ours, &theirs);
    let json = merge_preview_to_json(&preview);
    match serde_json::to_string(&serde_json::json!({ "mergePreview": json })) {
        Ok(s) => s,
        Err(e) => error_json(&format!("failed to serialize merge preview: {e}")),
    }
}

/// Apply a merge preview + resolutions to produce the final merged build
/// (§11.3).
///
/// Takes:
/// - `ours_json` — the `CharacterBuild` JSON that was passed as `ours` to
///   `merge`
/// - `preview_json` — the `mergePreview` value returned by `merge` (the raw
///   `{"autoMerged": [...], "conflicts": [...]}` object)
/// - `resolutions_json` — a JSON array of `{"path": "...", "resolution":
///   "ours" | "theirs" | <value>}` (may be `[]` if there are no conflicts)
///
/// Returns `{"merged": <CharacterBuild JSON>}` or `{"error": "..."}`.
///
/// Diff paths are `build.`-prefixed (§11.2), so internally `ours` is wrapped
/// in `{"build": ...}` for `apply_merge`, then unwrapped — same as the API
/// (connections.rs).
#[wasm_bindgen]
pub fn apply_merge(ours_json: &str, preview_json: &str, resolutions_json: &str) -> String {
    let ours: CharacterBuild = match serde_json::from_str(ours_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `ours`: {e}")),
    };
    let preview_val: Value = match serde_json::from_str(preview_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("failed to parse `preview`: {e}")),
    };
    let preview = match merge_preview_from_json(&preview_val) {
        Ok(p) => p,
        Err(e) => return error_json(&format!("invalid merge preview: {e}")),
    };
    let resolutions_val: Value = match serde_json::from_str(resolutions_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("failed to parse `resolutions`: {e}")),
    };
    let resolutions = match resolutions_from_json(&resolutions_val) {
        Ok(r) => r,
        Err(e) => return error_json(&format!("invalid resolutions: {e}")),
    };

    let ours_val = serde_json::json!({"build": serde_json::to_value(&ours).unwrap_or(Value::Null)});
    match apply_merge_impl(&ours_val, &preview, &resolutions) {
        Ok(result) => {
            match serde_json::to_string(&serde_json::json!({ "merged": result["build"] })) {
                Ok(s) => s,
                Err(e) => error_json(&format!("failed to serialize result: {e}")),
            }
        }
        Err(e) => error_json(&format!("merge application failed: {e}")),
    }
}

/// BLAKE3 content hash of a build (§9.2).
///
/// Takes a `CharacterBuild` JSON string, returns `{"contentHash": "<64 hex>"}`
/// or `{"error": "..."}`.
#[wasm_bindgen]
pub fn content_hash(build_json: &str) -> String {
    let build: CharacterBuild = match serde_json::from_str(build_json) {
        Ok(b) => b,
        Err(e) => return error_json(&format!("failed to parse `build`: {e}")),
    };
    let hash = content_hash_impl(&build);
    match serde_json::to_string(&serde_json::json!({ "contentHash": hash.to_hex() })) {
        Ok(s) => s,
        Err(e) => error_json(&format!("failed to serialize hash: {e}")),
    }
}

fn error_json(msg: &str) -> String {
    serde_json::to_string(&serde_json::json!({ "error": msg }))
        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::testing::minimal_build;

    #[test]
    fn diff_identical_builds_is_empty_diffs() {
        let build = serde_json::to_string(&minimal_build()).unwrap();
        let out = diff(&build, &build);
        let v: Value = serde_json::from_str(&out).expect("output is valid JSON");
        assert!(v.get("error").is_none(), "unexpected error: {v}");
        assert_eq!(v["diffs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_name_change_is_single_modified() {
        let a = minimal_build();
        let mut b = a.clone();
        b.identity.name = "Renamed".into();
        let out = diff(
            &serde_json::to_string(&a).unwrap(),
            &serde_json::to_string(&b).unwrap(),
        );
        let v: Value = serde_json::from_str(&out).expect("output is valid JSON");
        assert!(v.get("error").is_none(), "unexpected error: {v}");
        let diffs = v["diffs"].as_array().unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0]["path"], "build.identity.name");
        assert_eq!(diffs[0]["change"], "modified");
        assert_eq!(diffs[0]["before"], "Test Character");
        assert_eq!(diffs[0]["after"], "Renamed");
    }

    #[test]
    fn diff_bad_input_returns_error_object() {
        let out = diff("not json", "{}");
        let v: Value = serde_json::from_str(&out).expect("error path still returns JSON");
        assert!(v.get("error").is_some(), "expected an error field: {v}");
    }

    #[test]
    fn merge_conflict_round_trips_through_apply_merge() {
        let ancestor = minimal_build();
        let mut ours = ancestor.clone();
        ours.identity.name = "Ours".into();
        let mut theirs = ancestor.clone();
        theirs.identity.name = "Theirs".into();

        let ancestor_json = serde_json::to_string(&ancestor).unwrap();
        let ours_json = serde_json::to_string(&ours).unwrap();
        let theirs_json = serde_json::to_string(&theirs).unwrap();

        // merge → preview with one conflict
        let merge_out = merge(&ancestor_json, &ours_json, &theirs_json);
        let merge_v: Value = serde_json::from_str(&merge_out).unwrap();
        assert!(
            merge_v.get("error").is_none(),
            "unexpected error: {merge_v}"
        );
        let preview = &merge_v["mergePreview"];
        assert_eq!(preview["conflicts"].as_array().unwrap().len(), 1);

        // apply_merge with resolution "ours" → name stays "Ours"
        let resolutions = r#"[{"path":"build.identity.name","resolution":"ours"}]"#;
        let apply_out = apply_merge(&ours_json, &preview.to_string(), resolutions);
        let apply_v: Value = serde_json::from_str(&apply_out).unwrap();
        assert!(
            apply_v.get("error").is_none(),
            "unexpected error: {apply_v}"
        );
        assert_eq!(apply_v["merged"]["identity"]["name"], "Ours");

        // apply_merge with resolution "theirs" → name becomes "Theirs"
        let resolutions_theirs = r#"[{"path":"build.identity.name","resolution":"theirs"}]"#;
        let apply_out2 = apply_merge(&ours_json, &preview.to_string(), resolutions_theirs);
        let apply_v2: Value = serde_json::from_str(&apply_out2).unwrap();
        assert_eq!(apply_v2["merged"]["identity"]["name"], "Theirs");
    }

    #[test]
    fn content_hash_returns_hex_string() {
        let build = serde_json::to_string(&minimal_build()).unwrap();
        let out = content_hash(&build);
        let v: Value = serde_json::from_str(&out).expect("output is valid JSON");
        assert!(v.get("error").is_none(), "unexpected error: {v}");
        let hash = v["contentHash"].as_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn content_hash_bad_input_returns_error_object() {
        let out = content_hash("not json");
        let v: Value = serde_json::from_str(&out).expect("error path still returns JSON");
        assert!(v.get("error").is_some(), "expected an error field: {v}");
    }
}
