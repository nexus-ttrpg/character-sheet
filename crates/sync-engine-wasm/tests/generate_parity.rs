//! Generates `parity-expected.json` from native `sync-engine` output.
//!
//! Run with `cargo test -p sync-engine-wasm --test generate_parity`. The
//! resulting file is consumed by `parity-test.cjs`, which calls the same
//! operations through the WASM module and compares byte-identical.
//!
//! Parity is structural (the WASM wrapper calls the same `sync_engine`
//! functions), but this test suite confirms it empirically on all fixture
//! pairs and merge/apply scenarios.

use schema_core::{content_hash, CharacterBuild, Level};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use sync_engine::{
    apply_merge, diff, field_diffs_to_json, merge, merge_preview_to_json, Resolution,
};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/fixtures")
}

fn load_fixture(name: &str) -> Value {
    let path = fixtures_dir().join(format!("{name}.json"));
    let json_str =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    serde_json::from_str(&json_str)
        .unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

fn load_build(name: &str) -> CharacterBuild {
    serde_json::from_value(load_fixture(name))
        .unwrap_or_else(|e| panic!("failed to deserialize fixture {name}: {e}"))
}

fn resolutions_to_json(res: &[(String, Resolution)]) -> Value {
    let arr: Vec<Value> = res
        .iter()
        .map(|(p, r)| {
            let res_val = match r {
                Resolution::Ours => json!("ours"),
                Resolution::Theirs => json!("theirs"),
                Resolution::Custom(v) => v.clone(),
            };
            json!({"path": p, "resolution": res_val})
        })
        .collect();
    Value::Array(arr)
}

#[test]
fn generate_parity_expected() {
    let output_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("parity-expected.json");

    let fixture_names = [
        "pf2e-level1-fighter",
        "pf2e-level10-wizard",
        "pf2e-level5-ranger",
        "sf2e-level5-mechanic",
    ];

    let fixtures: Vec<(String, Value)> = fixture_names
        .iter()
        .map(|name| (name.to_string(), load_fixture(name)))
        .collect();

    // ── Diff pairs (4×4 = 16, including self-diffs) ──────────────────────

    let mut diffs: Vec<Value> = Vec::new();
    for (name_a, val_a) in &fixtures {
        for (name_b, val_b) in &fixtures {
            let build_a: CharacterBuild = serde_json::from_value(val_a.clone()).unwrap();
            let build_b: CharacterBuild = serde_json::from_value(val_b.clone()).unwrap();
            let result = diff(&build_a, &build_b);
            diffs.push(json!({
                "name": format!("{name_a}-vs-{name_b}"),
                "a": val_a,
                "b": val_b,
                "expected": {"diffs": field_diffs_to_json(&result)},
            }));
        }
    }

    // ── Merge scenarios (constructed from the fighter fixture) ───────────

    let fighter = load_build("pf2e-level1-fighter");

    let mut ours_name = fighter.clone();
    ours_name.identity.name = "Brodric the Brave".into();

    let mut theirs_name = fighter.clone();
    theirs_name.identity.name = "Alistair the Bold".into();

    let mut theirs_level = fighter.clone();
    theirs_level.level = Level::new(5).unwrap();

    let fighter_val = serde_json::to_value(&fighter).unwrap();
    let ours_name_val = serde_json::to_value(&ours_name).unwrap();
    let theirs_name_val = serde_json::to_value(&theirs_name).unwrap();
    let theirs_level_val = serde_json::to_value(&theirs_level).unwrap();

    // Scenario 1: both sides change the same field differently → conflict.
    let preview1 = merge(&fighter, &ours_name, &theirs_name);
    let preview1_json = merge_preview_to_json(&preview1);

    // Scenario 2: non-overlapping changes → auto-merge, no conflict.
    let preview2 = merge(&fighter, &ours_name, &theirs_level);
    let preview2_json = merge_preview_to_json(&preview2);

    // Scenario 3: one side unchanged → auto-merge, no conflict.
    let preview3 = merge(&fighter, &fighter, &theirs_name);
    let preview3_json = merge_preview_to_json(&preview3);

    let merges = json!([
        {
            "name": "conflict-name",
            "ancestor": fighter_val,
            "ours": ours_name_val,
            "theirs": theirs_name_val,
            "expected": {"mergePreview": preview1_json},
        },
        {
            "name": "auto-merge-name-and-level",
            "ancestor": fighter_val.clone(),
            "ours": ours_name_val.clone(),
            "theirs": theirs_level_val,
            "expected": {"mergePreview": preview2_json},
        },
        {
            "name": "one-sided-name-only",
            "ancestor": fighter_val.clone(),
            "ours": fighter_val.clone(),
            "theirs": theirs_name_val.clone(),
            "expected": {"mergePreview": preview3_json},
        },
    ]);

    // ── Apply_merge scenarios ────────────────────────────────────────────

    let wrapped_ours = json!({"build": ours_name_val.clone()});

    // 1. Resolve conflict → ours
    let res_ours = vec![("build.identity.name".to_string(), Resolution::Ours)];
    let result_ours = apply_merge(&wrapped_ours, &preview1, &res_ours).unwrap();

    // 2. Resolve conflict → theirs
    let res_theirs = vec![("build.identity.name".to_string(), Resolution::Theirs)];
    let result_theirs = apply_merge(&wrapped_ours, &preview1, &res_theirs).unwrap();

    // 3. Resolve conflict → custom value
    let res_custom = vec![(
        "build.identity.name".to_string(),
        Resolution::Custom(json!("Eve")),
    )];
    let result_custom = apply_merge(&wrapped_ours, &preview1, &res_custom).unwrap();

    // 4. Auto-merge, no resolutions needed
    let result_auto = apply_merge(&wrapped_ours, &preview2, &[]).unwrap();

    let apply_merges = json!([
        {
            "name": "resolve-ours",
            "ours": ours_name_val.clone(),
            "preview": preview1_json,
            "resolutions": resolutions_to_json(&res_ours),
            "expected": {"merged": result_ours["build"]},
        },
        {
            "name": "resolve-theirs",
            "ours": ours_name_val.clone(),
            "preview": preview1_json.clone(),
            "resolutions": resolutions_to_json(&res_theirs),
            "expected": {"merged": result_theirs["build"]},
        },
        {
            "name": "resolve-custom",
            "ours": ours_name_val.clone(),
            "preview": preview1_json.clone(),
            "resolutions": resolutions_to_json(&res_custom),
            "expected": {"merged": result_custom["build"]},
        },
        {
            "name": "auto-merge-no-resolutions",
            "ours": ours_name_val,
            "preview": preview2_json,
            "resolutions": resolutions_to_json(&[]),
            "expected": {"merged": result_auto["build"]},
        },
    ]);

    // ── Content hashes (all 4 fixtures) ──────────────────────────────────

    let content_hashes: Vec<Value> = fixtures
        .iter()
        .map(|(name, val)| {
            let build: CharacterBuild = serde_json::from_value(val.clone()).unwrap();
            let hash = content_hash(&build);
            json!({
                "name": name,
                "build": val,
                "expected": {"contentHash": hash.to_hex()},
            })
        })
        .collect();

    // ── Write output ─────────────────────────────────────────────────────

    let output = json!({
        "diffs": diffs,
        "merges": merges,
        "applyMerges": apply_merges,
        "contentHashes": content_hashes,
    });

    let pretty = serde_json::to_string_pretty(&output).unwrap();
    fs::write(&output_path, pretty).expect("failed to write parity-expected.json");
}
