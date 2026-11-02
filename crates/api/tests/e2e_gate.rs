//! End-to-end gate test: the §19 Phase 3 done criterion.
//!
//! "A player builds a PF2e character in Foundry, pushes to canonical, pulls
//! into a second Foundry world, makes divergent edits in both, and resolves a
//! real merge conflict end to end."
//!
//! This test exercises the **full cycle** through the real Foundry adapter
//! (V8 sandbox) via the HTTP API — no direct adapter-host calls:
//!
//! 1. Create a character in canonical from the real PF2e fixture.
//! 2. Create a Foundry connection.
//! 3. **Push** — canonical → Foundry actor via `adapter.export` (simulates
//!    "the character exists in Foundry world A").
//! 4. **First pull** with that Foundry actor — `adapter.import` converts it
//!    back to canonical, auto-merges (ancestor = ours on first pull), sets
//!    `last_sync_snapshot_id`.
//! 5. **World A edit**: PATCH `identity.name` on the canonical character.
//! 6. **World B edit**: modify the Foundry actor's `name` (maps to
//!    `identity.name` via foundry-mapping) to a different value.
//! 7. **Second pull** with the modified actor → 409 conflict on `identity.name`
//!    (both sides changed the same field from the ancestor).
//! 8. **Resolve** the conflict (choose "theirs") → `status: "merged"`.
//! 9. **Push** the merged build back to Foundry — verify the exported actor
//!    carries the resolved name.
//! 10. Verify the snapshot DAG has ≥3 snapshots, last one is a merge.
//!
//! **Prerequisite:** the Foundry adapter bundle must be pre-built:
//!   `pnpm --filter @nexus-ttrpg/foundry-adapter bundle`
//! The test skips gracefully if the bundle is absent.

use std::sync::Arc;

use api::app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use store::{EncryptionKey, SqliteStore, Store};
use tower::ServiceExt;

type BoxRouter =
    tower::util::BoxCloneService<Request<Body>, axum::response::Response, std::convert::Infallible>;

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn adapter_bundle_path() -> std::path::PathBuf {
    workspace_root().join("adapters/foundry/dist/bundle/bundle.js")
}

fn fighter_fixture_path() -> std::path::PathBuf {
    workspace_root().join("examples/fixtures/pf2e-level1-fighter.json")
}

/// Stage the adapter bundle into a temp dir, returning (tmp, adapters_dir).
/// The API's adapter_worker reads bundles from `<adapters_dir>/<adapter_id>/bundle.js`.
async fn setup_temp_adapters() -> (tempfile::TempDir, std::path::PathBuf) {
    let bundle = adapter_bundle_path();
    assert!(
        bundle.exists(),
        "adapter bundle not found at {} — run `pnpm --filter @nexus-ttrpg/foundry-adapter bundle` first",
        bundle.display()
    );
    let tmp = tempfile::tempdir().unwrap();
    let adapters_dir = tmp.path().join("adapters");
    let foundry_dir = adapters_dir.join("foundry");
    tokio::fs::create_dir_all(&foundry_dir).await.unwrap();
    tokio::fs::copy(&bundle, foundry_dir.join("bundle.js"))
        .await
        .unwrap();
    (tmp, adapters_dir)
}

async fn send_json(
    router: &BoxRouter,
    method: &str,
    uri: &str,
    body: serde_json::Value,
    auth: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("authorization", auth)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!(null));
    (status, json)
}

async fn send_get(router: &BoxRouter, uri: &str, auth: &str) -> (StatusCode, serde_json::Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!(null));
    (status, json)
}

#[tokio::test]
async fn e2e_foundry_divergent_merge_gate() {
    // Skip if the adapter bundle hasn't been built.
    if !adapter_bundle_path().exists() {
        eprintln!("skip: adapter bundle not built — run `pnpm --filter @nexus-ttrpg/foundry-adapter bundle`");
        return;
    }

    // ── Setup: store, auth, router ────────────────────────────────────────
    let (_tmp, adapters_dir) = setup_temp_adapters().await;

    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let key = EncryptionKey::generate();

    let owner = store::new_ulid();
    api::auth::seed_api_key(
        &store,
        "nsk_live_e2e_gate",
        owner,
        api::auth::KeyScope::Admin,
        "e2e-test",
    )
    .await
    .unwrap();

    let state = api::state::AppState::new(Arc::new(store), adapters_dir, key);
    let router = tower::util::BoxCloneService::new(app(state));
    let auth = "Bearer nsk_live_e2e_gate".to_string();

    // ── Step 1: Create character from the real PF2e fixture ──────────────
    let build_bytes = tokio::fs::read(fighter_fixture_path()).await.unwrap();
    let build_json: serde_json::Value = serde_json::from_slice(&build_bytes).unwrap();
    let original_name = build_json["identity"]["name"].as_str().unwrap().to_string();

    let (status, resp) = send_json(
        &router,
        "POST",
        "/v1/characters",
        serde_json::json!({ "build": build_json, "system": "pf2e" }),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create character: {resp}");
    let char_id = resp["character_id"].as_str().unwrap().to_string();

    // ── Step 2: Create a Foundry connection ──────────────────────────────
    let (status, resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections"),
        serde_json::json!({
            "platform": "foundry",
            "adapter_id": "foundry",
            "credentials": {},
        }),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create connection: {resp}");
    let conn_id = resp["id"].as_str().unwrap().to_string();

    // ── Step 3: Push — canonical → Foundry actor via adapter.export ──────
    let (status, push_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/push"),
        serde_json::json!({}),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "push should succeed: {push_resp}");

    let foundry_actor = push_resp["payload"].clone();
    assert_eq!(
        foundry_actor["type"], "character",
        "pushed payload should be a Foundry character: {foundry_actor}"
    );
    assert_eq!(
        foundry_actor["name"].as_str().unwrap(),
        original_name,
        "pushed actor name should match the fixture"
    );

    // ── Step 4: First pull — Foundry actor → canonical via adapter.import ─
    //    ancestor = ours (first pull, last_sync is None).
    //    If the round-trip is lossless → no_changes (still sets last_sync).
    //    If lossy → merged (sets last_sync).
    let (status, pull_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        serde_json::json!({ "payload": foundry_actor }),
        &auth,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "first pull should succeed: {pull_resp}"
    );
    let pull_status = pull_resp["status"].as_str().unwrap_or("");
    assert!(
        pull_status == "no_changes" || pull_status == "merged",
        "first pull should be no_changes or merged, got {pull_status}: {pull_resp}"
    );

    // ── Step 5: "World A" edit — PATCH identity.name ─────────────────────
    let (_, head_resp) = send_get(&router, &format!("/v1/characters/{char_id}"), &auth).await;
    let expected_hash = head_resp["content_hash"].as_str().unwrap();

    let world_a_name = "World-A-Aldric";
    let (status, _) = send_json(
        &router,
        "PATCH",
        &format!("/v1/characters/{char_id}"),
        serde_json::json!({
            "path": "identity.name",
            "value": world_a_name,
            "expected_content_hash": expected_hash,
        }),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PATCH (world A) should succeed");

    // ── Step 6: "World B" edit — modify the Foundry actor's name ─────────
    //    foundry-mapping maps actor.name ↔ identity.name (index.ts:528,827).
    let world_b_name = "World-B-Aldric";
    let mut world_b_actor = foundry_actor.clone();
    world_b_actor["name"] = serde_json::json!(world_b_name);

    // ── Step 7: Second pull → 409 conflict on identity.name ──────────────
    //    ancestor = last_sync (from step 4), ours = HEAD (world_a_name),
    //    theirs = imported (world_b_name). Both changed identity.name
    //    from the ancestor → conflict.
    let (status, conflict_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        serde_json::json!({ "payload": world_b_actor }),
        &auth,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "expected 409 conflict on identity.name: {conflict_resp}"
    );
    assert_eq!(conflict_resp["error"].as_str().unwrap(), "conflict");

    let mp = &conflict_resp["merge_preview"];
    let conflicts = mp["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("expected conflicts array: {mp}"));
    assert!(
        !conflicts.is_empty(),
        "expected at least one conflict: {mp}"
    );
    // Verify the conflict is on identity.name (or a path containing it).
    let conflict_path = conflicts[0]["path"].as_str().unwrap().to_string();
    assert!(
        conflict_path.contains("identity") && conflict_path.contains("name"),
        "conflict should be on identity.name, got {conflict_path}"
    );

    // ── Step 8: Resolve the conflict (choose "theirs") ───────────────────
    let resolutions: Vec<serde_json::Value> = conflicts
        .iter()
        .map(|c| {
            let path = c["path"].as_str().unwrap();
            serde_json::json!({ "path": path, "resolution": "theirs" })
        })
        .collect();

    let (status, resolve_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        serde_json::json!({
            "payload": world_b_actor,
            "resolutions": resolutions,
        }),
        &auth,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "resolve should succeed: {resolve_resp}"
    );
    assert_eq!(
        resolve_resp["status"].as_str().unwrap(),
        "merged",
        "resolve should produce merged status"
    );

    // ── Step 9: Push the merged build back to Foundry ────────────────────
    let (status, push_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/push"),
        serde_json::json!({}),
        &auth,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "push merged should succeed: {push_resp}"
    );

    let merged_actor = &push_resp["payload"];
    assert_eq!(
        merged_actor["type"], "character",
        "pushed payload should be a Foundry character"
    );
    // "theirs" was chosen → the merged build has world_b_name.
    assert_eq!(
        merged_actor["name"].as_str().unwrap(),
        world_b_name,
        "pushed actor should carry the resolved (theirs) name: {merged_actor}"
    );

    // ── Step 10: Verify the snapshot DAG ─────────────────────────────────
    let (status, history) =
        send_get(&router, &format!("/v1/characters/{char_id}/history"), &auth).await;
    assert_eq!(status, StatusCode::OK);

    let snapshots = history["snapshots"]
        .as_array()
        .unwrap_or_else(|| panic!("expected snapshots array: {history}"));

    // Expected DAG: initial (create) → [merge/no_changes from first pull]
    //               → patch → merge = 3-4 snapshots.
    assert!(
        snapshots.len() >= 3,
        "expected at least 3 snapshots in DAG, got {}: {snapshots:?}",
        snapshots.len()
    );

    // The last snapshot should be a merge (has merge_parent).
    let last = snapshots.last().unwrap();
    assert!(
        last["merge_parent"].as_str().is_some(),
        "last snapshot should be a merge (has merge_parent): {last:?}"
    );

    // ── Verify content hash integrity ────────────────────────────────────
    let (_, final_char) = send_get(&router, &format!("/v1/characters/{char_id}"), &auth).await;
    let content_hash = final_char["content_hash"]
        .as_str()
        .unwrap_or_else(|| panic!("expected content_hash: {final_char}"));
    assert!(
        !content_hash.is_empty(),
        "merged character should have a non-empty content hash"
    );
}
