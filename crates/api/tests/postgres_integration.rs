//! Postgres integration test via testcontainers (§12, §14).
//!
//! Spins up a real Postgres instance in a Docker container, then runs a
//! full API round-trip (create character → patch → pull → history) against it.
//! This validates that the PgStore backend works correctly end-to-end through
//! the Axum router — not just the SqliteStore used by other integration tests.
//!
//! **Requires Docker.** If Docker is not available (e.g. in a sandboxed CI
//! runner), the test skips gracefully with a printed message and passes.

use std::sync::Arc;

use api::app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use schema_core::testing::minimal_build;
use store::{EncryptionKey, PgStore, Store};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

type Router = axum::Router;

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {auth}"));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let body_bytes = body
        .map(|b| serde_json::to_vec(&b).unwrap())
        .unwrap_or_default();
    let resp = router
        .clone()
        .oneshot(builder.body(Body::from(body_bytes)).unwrap())
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
async fn postgres_api_round_trip() {
    // Start a Postgres container. If Docker isn't available, skip gracefully.
    let container = match Postgres::default().start().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping postgres_api_round_trip: Docker not available ({e})");
            return;
        }
    };

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get container port");
    // Ask testcontainers for the host rather than assuming 127.0.0.1, which is
    // wrong whenever Docker is not local (Docker-in-Docker, remote daemons).
    let host = container.get_host().await.expect("get container host");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // Build the PgStore from a connection pool.
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to Postgres container");
    let store = PgStore::from_pool(pool);
    store.migrate().await.expect("migrate");

    // Bootstrap an Admin API key.
    let owner = store::new_ulid();
    api::auth::seed_api_key(
        &store,
        "nsk_live_pg_test_key",
        owner,
        api::auth::KeyScope::Admin,
        "pg-test",
    )
    .await
    .expect("seed api key");

    let encryption_key = EncryptionKey::generate();
    let state = api::state::AppState::new(
        Arc::new(store),
        std::path::PathBuf::from("/tmp/nexus-adapters"),
        encryption_key,
    );
    let router = app(state);
    let auth = "nsk_live_pg_test_key";

    // 1. Create a character.
    let build_json = serde_json::to_value(minimal_build()).unwrap();
    let (status, resp) = send(
        &router,
        "POST",
        "/v1/characters",
        auth,
        Some(serde_json::json!({"build": build_json, "system": "pf2e"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create character: {resp}");
    let char_id = resp["character_id"].as_str().unwrap().to_string();
    let original_hash = resp["content_hash"].as_str().unwrap().to_string();

    // 2. GET the character — verify it round-tripped through Postgres.
    let (status, resp) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}"),
        auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["content_hash"].as_str().unwrap(), original_hash);

    // 3. PATCH the character — change the name.
    let patch_body = serde_json::json!({
        "path": "identity.name",
        "value": "Postgres Hero",
        "expected_content_hash": original_hash,
    });
    let (status, resp) = send(
        &router,
        "PATCH",
        &format!("/v1/characters/{char_id}"),
        auth,
        Some(patch_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "patch character: {resp}");
    let new_hash = resp["content_hash"].as_str().unwrap().to_string();
    assert_ne!(new_hash, original_hash, "hash should change after patch");

    // 4. GET history — verify the DAG has two snapshots.
    let (status, resp) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}/history"),
        auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshots = resp["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 2, "expected 2 snapshots in DAG: {resp}");

    // 5. Create a connection and do a pull (pre-converted build, no adapter).
    let conn_req = serde_json::json!({
        "platform": "foundry",
        "adapter_id": "foundry",
        "credentials": {},
    });
    let (status, resp) = send(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections"),
        auth,
        Some(conn_req),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create connection: {resp}");
    let conn_id = resp["id"].as_str().unwrap().to_string();

    // 6. Pull with a different field change → should auto-merge.
    let (_, char_resp) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}"),
        auth,
        None,
    )
    .await;
    let mut theirs = char_resp["build"].clone();
    theirs["identity"]["playerName"] = serde_json::json!("PG Player");

    let pull_body = serde_json::json!({"build": theirs});
    let (status, resp) = send(
        &router,
        "POST",
        &format!("/v1/characters/{char_id}/connections/{conn_id}/pull"),
        auth,
        Some(pull_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pull should auto-merge: {resp}");
    assert_eq!(resp["status"].as_str().unwrap(), "merged");

    // 7. Verify the DAG now has 3 snapshots (initial, patch, merge).
    let (status, resp) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}/history"),
        auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshots = resp["snapshots"].as_array().unwrap();
    assert_eq!(
        snapshots.len(),
        3,
        "expected 3 snapshots after pull: {resp}"
    );

    // The last snapshot should be a merge (has merge_parent).
    let last = snapshots.last().unwrap();
    assert!(
        last["merge_parent"].as_str().is_some(),
        "last snapshot should be a merge"
    );

    // Container is dropped here → Postgres shuts down.
}
