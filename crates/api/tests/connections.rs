//! Integration test for connection lifecycle, focused on `DELETE
//! /v1/characters/:id/connections/:connId` (§13.3). Exercises the real
//! `Store::delete_connection` against an in-memory SQLite store through the
//! Axum router — no adapter needed.

use std::sync::Arc;

use api::app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use schema_core::testing::minimal_build;
use store::{EncryptionKey, SqliteStore, Store};
use tower::ServiceExt;

/// Build a router with a migrated in-memory SQLite store, one Admin API key,
/// one character, and one connection. Returns the router, the character id,
/// the connection id, and the auth header.
async fn setup() -> (
    tower::util::BoxCloneService<Request<Body>, axum::response::Response, std::convert::Infallible>,
    String,
    String,
    String,
) {
    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();

    let key = EncryptionKey::generate();

    // Bootstrap an Admin API key so the test can authenticate (§13.1).
    let owner = store::new_ulid();
    api::auth::seed_api_key(
        &store,
        "nsk_live_test_admin_key",
        owner,
        api::auth::KeyScope::Admin,
        "test",
    )
    .await
    .unwrap();

    let state = api::state::AppState::new(
        Arc::new(store),
        std::path::PathBuf::from("/tmp/nexus-adapters"),
        key,
    );
    let router = app(state);
    let auth_header = "Bearer nsk_live_test_admin_key".to_string();

    // Create a character.
    let create_req = serde_json::json!({
        "build": serde_json::to_value(minimal_build()).unwrap(),
        "system": "pf2e",
    });
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/characters")
                .header("content-type", "application/json")
                .header("authorization", &auth_header)
                .body(Body::from(serde_json::to_vec(&create_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let char_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["character_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Create a connection.
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
                .header("authorization", &auth_header)
                .body(Body::from(serde_json::to_vec(&conn_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let conn_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    (
        tower::util::BoxCloneService::new(router),
        char_id,
        conn_id,
        auth_header,
    )
}

async fn send(
    router: &tower::util::BoxCloneService<
        Request<Body>,
        axum::response::Response,
        std::convert::Infallible,
    >,
    method: &str,
    uri: &str,
    auth_header: &str,
) -> (StatusCode, Option<serde_json::Value>) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", auth_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    // 204 has no body; everything else we parse as JSON.
    if status == StatusCode::NO_CONTENT {
        return (status, None);
    }
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let val = serde_json::from_slice::<serde_json::Value>(&body).ok();
    (status, val)
}

#[tokio::test]
async fn delete_connection_removes_it_and_404s_after() {
    let (router, char_id, conn_id, auth) = setup().await;

    // List before: one connection present.
    let (status, body) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}/connections"),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_ref().unwrap()["connections"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Delete it.
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/v1/characters/{char_id}/connections/{conn_id}"),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // List after: empty.
    let (status, body) = send(
        &router,
        "GET",
        &format!("/v1/characters/{char_id}/connections"),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_ref().unwrap()["connections"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Deleting again → 404 (the connection no longer exists).
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/v1/characters/{char_id}/connections/{conn_id}"),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_unknown_connection_404s() {
    let (router, char_id, _conn_id, auth) = setup().await;

    // A well-formed ULID that was never created.
    let bogus = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/v1/characters/{char_id}/connections/{bogus}"),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_none() || !body.unwrap()["error"].is_null());
}

#[tokio::test]
async fn delete_connection_requires_admin_scope() {
    // A read_only / sync key must not be allowed to DELETE (§13.1, auth.rs).
    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let key = EncryptionKey::generate();
    let owner = store::new_ulid();
    api::auth::seed_api_key(
        &store,
        "nsk_live_test_sync_key",
        owner,
        api::auth::KeyScope::Sync,
        "test",
    )
    .await
    .unwrap();
    let state = api::state::AppState::new(
        Arc::new(store),
        std::path::PathBuf::from("/tmp/nexus-adapters"),
        key,
    );
    let router = tower::util::BoxCloneService::new(app(state));
    let auth = "Bearer nsk_live_test_sync_key".to_string();

    let (status, _) = send(
        &router,
        "DELETE",
        "/v1/characters/01ARZ3NDEKTSV4RRFFQ69G5FAV/connections/01ARZ3NDEKTSV4RRFFQ69G5FAV",
        &auth,
    )
    .await;
    // Sync scope is forbidden from DELETE — 403 before the store is touched.
    assert_eq!(status, StatusCode::FORBIDDEN);
}
