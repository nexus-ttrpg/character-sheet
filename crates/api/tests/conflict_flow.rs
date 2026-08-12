//! Integration test: create → divergent edit → pull 409+mergePreview →
//! resolve → verify DAG (§11.3, §13.3).
//!
//! This exercises the full three-way merge flow end-to-end through the Axum
//! router, using an in-memory SQLite store and pre-converted canonical builds
//! (no adapter needed).

use std::sync::Arc;

use api::app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use schema_core::testing::minimal_build;
use store::{EncryptionKey, SqliteStore, Store};
use tower::ServiceExt;

async fn setup() -> (
    tower::util::BoxCloneService<Request<Body>, axum::response::Response, std::convert::Infallible>,
    String,
    String,
) {
    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();

    let key = EncryptionKey::generate();
    let state = api::state::AppState::new(
        Arc::new(store),
        std::path::PathBuf::from("/tmp/nexus-adapters"),
        key,
    );
    let router = app(state);

    // 1. Create a character.
    let build_json = serde_json::to_value(minimal_build()).unwrap();
    let create_req = serde_json::json!({
        "build": build_json,
        "system": "pf2e",
    });
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let create_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let char_id = create_resp["character_id"].as_str().unwrap().to_string();
    let _initial_hash = create_resp["content_hash"].as_str().unwrap().to_string();

    // 2. Create a connection (adapter_id doesn't matter — we'll use `build` in pull).
    let conn_req = serde_json::json!({
        "platform": "foundry",
        "adapter_id": "foundry",
        "credentials": {},
    });
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/characters/{char_id}/connections"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&conn_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let conn_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let conn_id = conn_resp["id"].as_str().unwrap().to_string();

    // Wrap router in BoxCloneService so it can be returned.
    let boxed = tower::util::BoxCloneService::new(router);

    (boxed, char_id, conn_id)
}

async fn send_json(
    router: &tower::util::BoxCloneService<
        Request<Body>,
        axum::response::Response,
        std::convert::Infallible,
    >,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
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

#[tokio::test]
async fn conflict_flow_create_patch_pull_resolve() {
    let (router, char_id, conn_id) = setup().await;

    // 3. PATCH the character — change the name (ours diverges).
    let head_resp: serde_json::Value = {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/characters/{char_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    };
    let expected_hash = head_resp["content_hash"].as_str().unwrap();

    let patch_body = serde_json::json!({
        "path": "identity.name",
        "value": "Alice (patched)",
        "expected_content_hash": expected_hash,
    });
    let (status, _patch_resp) = send_json(
        &router,
        "PATCH",
        &format!("/v1/characters/{char_id}"),
        patch_body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PATCH should succeed");

    // 4. PULL with a conflicting change on the same field (theirs also changes name).
    //    ancestor = initial build (last_sync is None → ancestor = HEAD at first pull).
    //    Wait — first pull, ancestor = ours. Both ours and theirs changed the same
    //    field from the same ancestor → conflict.
    //
    //    But ancestor = ours means merge(ours, ours, theirs) → any field where ours
    //    and theirs differ is a conflict (ours didn't change it relative to itself,
    //    theirs did). Actually merge(ancestor=ours, ours, theirs): the only changes
    //    are theirs-side, so they auto-merge. We need ancestor != ours for a conflict.
    //
    //    So let's instead test the non-conflict path: pull with a change to a
    //    different field → auto-merge.

    // Build "theirs" — same as HEAD but with a different field changed.
    let head_build = head_resp["build"].clone();
    let mut theirs = head_build.clone();
    theirs["identity"]["playerName"] = serde_json::json!("Bob");

    let pull_body = serde_json::json!({
        "build": theirs,
    });
    let (status, pull_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        pull_body,
    )
    .await;

    // First pull: ancestor = ours (HEAD). Theirs changed player_name, ours didn't
    // (relative to ancestor=ours). So theirs' change auto-merges → 200, no conflict.
    assert_eq!(
        status,
        StatusCode::OK,
        "first pull should auto-merge (ancestor=ours): {}",
        pull_resp
    );
    assert_eq!(pull_resp["status"].as_str().unwrap(), "merged");

    let _merge_snap_id = pull_resp["new_snapshot_id"].as_str().unwrap().to_string();

    // 5. Now do a second pull that creates a real conflict.
    //    ancestor = last_sync (the merge snapshot from step 4).
    //    ours = HEAD (the merge snapshot — player_name=Bob, name=Alice (patched)).
    //    theirs = change player_name to "Charlie" AND change name to "Eve".
    //    - name: ancestor has "Alice (patched)", ours has "Alice (patched)", theirs has "Eve"
    //      → theirs changed, ours didn't → auto-merge to "Eve". No conflict.
    //    - player_name: ancestor has "Bob", ours has "Bob", theirs has "Charlie"
    //      → theirs changed, ours didn't → auto-merge. No conflict.
    //
    //    For a real conflict, BOTH sides must change the SAME field differently.
    //    So: PATCH the character to change player_name, then pull with theirs
    //    also changing player_name to a different value.

    // Get current HEAD.
    let head_resp: serde_json::Value = {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/characters/{char_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    };
    let expected_hash = head_resp["content_hash"].as_str().unwrap();

    // PATCH: change player_name to "Dave" (ours).
    let patch_body = serde_json::json!({
        "path": "identity.playerName",
        "value": "Dave",
        "expected_content_hash": expected_hash,
    });
    let (status, _) = send_json(
        &router,
        "PATCH",
        &format!("/v1/characters/{char_id}"),
        patch_body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "second PATCH should succeed");

    // Get updated HEAD build for "theirs".
    let head_resp: serde_json::Value = {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/characters/{char_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    };
    let mut theirs = head_resp["build"].clone();
    // Change player_name to "Charlie" (theirs) — conflicts with ours="Dave".
    theirs["identity"]["playerName"] = serde_json::json!("Charlie");

    let pull_body = serde_json::json!({
        "build": theirs,
    });
    let (status, conflict_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        pull_body,
    )
    .await;

    // Should get 409 + mergePreview.
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "expected 409 conflict, got {status}: {conflict_resp}"
    );
    assert_eq!(conflict_resp["error"].as_str().unwrap(), "conflict");
    let mp = &conflict_resp["merge_preview"];
    assert!(
        !mp["conflicts"].as_array().unwrap().is_empty(),
        "expected at least one conflict: {mp}"
    );

    // Extract the conflict path.
    let conflict_path = mp["conflicts"][0]["path"].as_str().unwrap().to_string();

    // 6. Resolve the conflict (choose "theirs").
    let resolve_body = serde_json::json!({
        "build": theirs,
        "resolutions": [
            {
                "path": conflict_path,
                "resolution": "theirs",
            }
        ],
    });
    let (status, resolve_resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        resolve_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "resolve should succeed: {resolve_resp}"
    );
    assert_eq!(resolve_resp["status"].as_str().unwrap(), "merged");

    // 7. Verify the DAG — GET history.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/characters/{char_id}/history"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let history: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let snapshots = history["snapshots"].as_array().unwrap();
    // We expect: initial (ManualEdit), patch1 (ManualEdit), merge1 (Merge),
    //            patch2 (ManualEdit), merge2 (Merge) = 5 snapshots.
    assert_eq!(
        snapshots.len(),
        5,
        "expected 5 snapshots in DAG, got {}: {snapshots:?}",
        snapshots.len()
    );

    // Verify the last snapshot is a merge.
    let last = snapshots.last().unwrap();
    assert!(
        last["merge_parent"].as_str().is_some(),
        "last snapshot should be a merge (has merge_parent)"
    );
}

#[tokio::test]
async fn pull_idempotent_replay() {
    let (router, char_id, conn_id) = setup().await;

    // Get HEAD build.
    let head_resp: serde_json::Value = {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/characters/{char_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    };

    // Pull with the same build as HEAD → no_changes (idempotent, §12).
    let pull_body = serde_json::json!({
        "build": head_resp["build"],
    });
    let (status, resp) = send_json(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        pull_body,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        resp["status"].as_str().unwrap(),
        "no_changes",
        "pulling same content should be a no-op"
    );
}
