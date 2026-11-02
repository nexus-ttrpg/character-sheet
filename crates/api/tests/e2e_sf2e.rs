//! SF2e parity e2e test — exercises SF2e-specific fields (drones, credits, UPB,
//! vehicles) through the full API cycle: create → push → pull → divergent
//! merge on an SF2e field → resolve → push.
//!
//! This is the "SF2e at parity for the fields SF2e uses" half of the §19
//! Phase 3 gate criterion. It proves that the merge engine, adapter, and API
//! all handle SF2e fields correctly — not just the PF2e fields that the
//! primary e2e_gate.rs exercises.
//!
//! Flow:
//! 1. Create an SF2e character from `sf2e-level5-mechanic.json` (system="sf2e").
//! 2. Create a Foundry connection.
//! 3. Push — `adapter.export` with system="sf2e" (not the old hardcoded "pf2e").
//!    Verify the exported actor carries drones and credits.
//! 4. First pull with the Foundry actor — `adapter.import` converts it back.
//!    Verify drones/credits round-trip.
//! 5. "World A" edit: PATCH `currencies.credits` to a different value.
//! 6. "World B" edit: modify the Foundry actor's credits to yet another value.
//! 7. Pull → 409 conflict on `build.currencies.credits`.
//! 8. Resolve (theirs).
//! 9. Push the merged build — verify the exported actor has the resolved credits.
//!
//! **Prerequisite:** the Foundry adapter bundle must be pre-built.

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

fn mechanic_fixture_path() -> std::path::PathBuf {
    workspace_root().join("examples/fixtures/sf2e-level5-mechanic.json")
}

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
async fn e2e_sf2e_divergent_merge_on_credits() {
    if !adapter_bundle_path().exists() {
        eprintln!("skip: adapter bundle not built — run `pnpm --filter @nexus-ttrpg/foundry-adapter bundle`");
        return;
    }

    let (_tmp, adapters_dir) = setup_temp_adapters().await;

    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let key = EncryptionKey::generate();

    let owner = store::new_ulid();
    api::auth::seed_api_key(
        &store,
        "nsk_live_sf2e_gate",
        owner,
        api::auth::KeyScope::Admin,
        "sf2e-test",
    )
    .await
    .unwrap();

    let state = api::state::AppState::new(Arc::new(store), adapters_dir, key);
    let router = tower::util::BoxCloneService::new(app(state));
    let auth = "Bearer nsk_live_sf2e_gate".to_string();

    // ── Step 1: Create SF2e character from the mechanic fixture ──────────
    let build_bytes = tokio::fs::read(mechanic_fixture_path()).await.unwrap();
    let build_json: serde_json::Value = serde_json::from_slice(&build_bytes).unwrap();
    let original_credits = build_json["currencies"]["credits"].as_u64().unwrap();

    let (status, resp) = send_json(
        &router,
        "POST",
        "/v1/characters",
        serde_json::json!({ "build": build_json, "system": "sf2e" }),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create SF2e character: {resp}");
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

    // ── Step 3: Push — adapter.export with system="sf2e" ─────────────────
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
        "pushed payload should be a Foundry character"
    );

    // Verify SF2e fields made it into the exported actor.
    let items = foundry_actor["items"]
        .as_array()
        .unwrap_or_else(|| panic!("actor should have items: {foundry_actor}"));
    let has_drone = items.iter().any(|i| i["type"] == "drone");
    let has_vehicle = items.iter().any(|i| i["type"] == "vehicle");
    assert!(
        has_drone,
        "pushed actor should contain a drone item (SF2e field): {foundry_actor}"
    );
    assert!(
        has_vehicle,
        "pushed actor should contain a vehicle item (SF2e field): {foundry_actor}"
    );
    assert_eq!(
        foundry_actor["system"]["currency"]["credits"].as_u64(),
        Some(original_credits),
        "pushed actor should carry credits"
    );

    // ── Step 4: First pull — adapter.import converts actor back ──────────
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

    // Verify drones round-tripped through import.
    let (_, head_resp) = send_get(&router, &format!("/v1/characters/{char_id}"), &auth).await;
    let drones = head_resp["build"]["drones"]
        .as_array()
        .unwrap_or_else(|| panic!("HEAD build should have drones: {head_resp}"));
    assert!(
        !drones.is_empty(),
        "drones should survive the push→pull round-trip"
    );

    // ── Step 5: "World A" edit — PATCH currencies.credits ────────────────
    let (_, head_resp) = send_get(&router, &format!("/v1/characters/{char_id}"), &auth).await;
    let expected_hash = head_resp["content_hash"].as_str().unwrap();

    let world_a_credits = original_credits + 100;
    let (status, _) = send_json(
        &router,
        "PATCH",
        &format!("/v1/characters/{char_id}"),
        serde_json::json!({
            "path": "currencies.credits",
            "value": world_a_credits,
            "expected_content_hash": expected_hash,
        }),
        &auth,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH credits (world A) should succeed"
    );

    // ── Step 6: "World B" edit — modify the Foundry actor's credits ──────
    let world_b_credits = original_credits + 200;
    let mut world_b_actor = foundry_actor.clone();
    world_b_actor["system"]["currency"]["credits"] = serde_json::json!(world_b_credits);

    // ── Step 7: Pull → 409 conflict on currencies.credits ────────────────
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
        "expected 409 conflict on currencies.credits: {conflict_resp}"
    );

    let mp = &conflict_resp["merge_preview"];
    let conflicts = mp["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("expected conflicts array: {mp}"));
    assert!(
        !conflicts.is_empty(),
        "expected at least one conflict: {mp}"
    );

    let conflict_path = conflicts[0]["path"].as_str().unwrap().to_string();
    assert!(
        conflict_path.contains("currencies"),
        "conflict should be on currencies, got {conflict_path}"
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

    // ── Step 9: Push the merged build — verify resolved credits ──────────
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
        merged_actor["system"]["currency"]["credits"].as_u64(),
        Some(world_b_credits),
        "pushed actor should carry the resolved (theirs) credits: {merged_actor}"
    );

    // Verify SF2e fields survived the full cycle.
    let merged_items = merged_actor["items"]
        .as_array()
        .unwrap_or_else(|| panic!("merged actor should have items: {merged_actor}"));
    assert!(
        merged_items.iter().any(|i| i["type"] == "drone"),
        "merged actor should still have a drone after the full cycle"
    );

    // ── Verify the snapshot DAG ──────────────────────────────────────────
    let (status, history) =
        send_get(&router, &format!("/v1/characters/{char_id}/history"), &auth).await;
    assert_eq!(status, StatusCode::OK);

    let snapshots = history["snapshots"]
        .as_array()
        .unwrap_or_else(|| panic!("expected snapshots array: {history}"));
    assert!(
        snapshots.len() >= 3,
        "expected at least 3 snapshots in DAG, got {}: {snapshots:?}",
        snapshots.len()
    );

    let last = snapshots.last().unwrap();
    assert!(
        last["merge_parent"].as_str().is_some(),
        "last snapshot should be a merge (has merge_parent): {last:?}"
    );
}
