//! Integration tests for API key auth (§13.1) and rate limiting (§17).
//!
//! Tests:
//! - Bootstrap: first key creation without auth
//! - Missing auth → 401
//! - Invalid key → 401
//! - Scope enforcement: ReadOnly can't POST, Sync can't DELETE, Admin can
//! - Key listing: Admin only
//! - Key revocation: revoked key no longer works
//! - Rate limiting: exhausting the write bucket → 429

use std::sync::Arc;

use api::app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use schema_core::testing::minimal_build;
use store::{EncryptionKey, SqliteStore, Store};
use tower::ServiceExt;

type Router = axum::Router;

async fn build_state() -> api::state::AppState {
    let store = SqliteStore::connect_in_memory().await.unwrap();
    store.migrate().await.unwrap();
    let key = EncryptionKey::generate();
    api::state::AppState::new(
        Arc::new(store),
        std::path::PathBuf::from("/tmp/nexus-adapters"),
        key,
    )
}

async fn bootstrap_admin(router: &Router) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/api-keys")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "label": "admin",
                        "scope": "admin",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "bootstrap should succeed");
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["key"].as_str().unwrap().to_string()
}

async fn create_key(router: &Router, admin_key: &str, label: &str, scope: &str) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin_key}"))
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "label": label,
                        "scope": scope,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "create key should succeed");
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["key"].as_str().unwrap().to_string()
}

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    auth: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = auth {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
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
async fn bootstrap_creates_first_key_without_auth() {
    let state = build_state().await;
    let router = app(state);

    let key = bootstrap_admin(&router).await;
    assert!(key.starts_with("nsk_live_"));
}

#[tokio::test]
async fn second_key_requires_admin_auth() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;

    // Without auth → 401 (registry is no longer empty).
    let (status, _) = send(
        &router,
        "POST",
        "/v1/api-keys",
        None,
        Some(serde_json::json!({"label": "second", "scope": "sync"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With admin auth → 200.
    let (status, _) = send(
        &router,
        "POST",
        "/v1/api-keys",
        Some(&admin_key),
        Some(serde_json::json!({"label": "second", "scope": "sync"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn missing_auth_returns_401() {
    let state = build_state().await;
    let router = app(state);

    let _ = bootstrap_admin(&router).await;

    let (status, json) = send(&router, "GET", "/v1/characters", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn invalid_key_returns_401() {
    let state = build_state().await;
    let router = app(state);

    let _ = bootstrap_admin(&router).await;

    let (status, _) = send(
        &router,
        "GET",
        "/v1/characters",
        Some("nsk_live_invalid"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_auth_header_returns_401() {
    let state = build_state().await;
    let router = app(state);

    let _ = bootstrap_admin(&router).await;

    // No "Bearer " prefix.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/characters")
                .header("authorization", "nsk_live_abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn scope_read_only_cannot_post() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let ro_key = create_key(&router, &admin_key, "ro", "read_only").await;

    let build_json = serde_json::to_value(minimal_build()).unwrap();
    let (status, _) = send(
        &router,
        "POST",
        "/v1/characters",
        Some(&ro_key),
        Some(serde_json::json!({"build": build_json, "system": "pf2e"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn scope_read_only_can_get() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let ro_key = create_key(&router, &admin_key, "ro", "read_only").await;

    let (status, _) = send(&router, "GET", "/v1/characters", Some(&ro_key), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn scope_sync_can_post_but_not_delete() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let sync_key = create_key(&router, &admin_key, "sync", "sync").await;

    // POST character → OK (Sync allows POST).
    let build_json = serde_json::to_value(minimal_build()).unwrap();
    let (status, resp) = send(
        &router,
        "POST",
        "/v1/characters",
        Some(&sync_key),
        Some(serde_json::json!({"build": build_json, "system": "pf2e"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Sync should allow POST: {resp}");

    // DELETE character → Forbidden (Sync doesn't allow DELETE).
    let char_id = resp["character_id"].as_str().unwrap();
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/v1/characters/{char_id}"),
        Some(&sync_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn scope_admin_can_delete() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;

    // POST character.
    let build_json = serde_json::to_value(minimal_build()).unwrap();
    let (_, resp) = send(
        &router,
        "POST",
        "/v1/characters",
        Some(&admin_key),
        Some(serde_json::json!({"build": build_json, "system": "pf2e"})),
    )
    .await;
    let char_id = resp["character_id"].as_str().unwrap();

    // DELETE → 204.
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/v1/characters/{char_id}"),
        Some(&admin_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn list_keys_admin_only() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let sync_key = create_key(&router, &admin_key, "sync", "sync").await;

    // Admin can list.
    let (status, json) = send(&router, "GET", "/v1/api-keys", Some(&admin_key), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["keys"].as_array().unwrap().len() >= 2);

    // Sync cannot list.
    let (status, _) = send(&router, "GET", "/v1/api-keys", Some(&sync_key), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revoke_key_makes_it_invalid() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let sync_key = create_key(&router, &admin_key, "sync", "sync").await;

    // List keys to get the sync key's id.
    let (_, json) = send(&router, "GET", "/v1/api-keys", Some(&admin_key), None).await;
    let sync_key_id = json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["label"].as_str() == Some("sync"))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Revoke it.
    let (status, _) = send(
        &router,
        "DELETE",
        &format!("/v1/api-keys/{sync_key_id}"),
        Some(&admin_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Now the revoked key returns 401.
    let (status, _) = send(&router, "GET", "/v1/characters", Some(&sync_key), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoke_nonexistent_key_returns_404() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;

    let (status, _) = send(
        &router,
        "DELETE",
        "/v1/api-keys/01J00000000000000000000000",
        Some(&admin_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn openapi_spec_is_public() {
    let state = build_state().await;
    let router = app(state);

    // No auth needed for OpenAPI.
    let (status, _) = send(&router, "GET", "/v1/openapi.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_write_bucket_exhausts() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;

    // The write bucket capacity is 30 (WRITE_CONFIG). Send 31 POST requests
    // to /v1/api-keys — the 31st should get 429.
    let mut got_429 = false;
    for i in 0..35 {
        let (status, _) = send(
            &router,
            "POST",
            "/v1/api-keys",
            Some(&admin_key),
            Some(serde_json::json!({"label": format!("key-{i}"), "scope": "sync"})),
        )
        .await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            got_429 = true;
            break;
        }
    }
    assert!(
        got_429,
        "should have been rate-limited after exhausting write bucket"
    );
}

#[tokio::test]
async fn rate_limit_read_bucket_is_more_generous() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;

    // The read bucket capacity is 100 (READ_CONFIG). Send 50 GET requests —
    // none should be rate-limited.
    for _ in 0..50 {
        let (status, _) = send(&router, "GET", "/v1/characters", Some(&admin_key), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "read requests under limit should pass"
        );
    }
}

#[tokio::test]
async fn rate_limit_separate_keys_are_independent() {
    let state = build_state().await;
    let router = app(state);

    let admin_key = bootstrap_admin(&router).await;
    let sync_key = create_key(&router, &admin_key, "sync", "sync").await;

    // Exhaust admin's write bucket.
    for i in 0..35 {
        let (status, _) = send(
            &router,
            "POST",
            "/v1/api-keys",
            Some(&admin_key),
            Some(serde_json::json!({"label": format!("k-{i}"), "scope": "sync"})),
        )
        .await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            break;
        }
    }

    // sync_key is on a separate bucket — should still work.
    let (status, _) = send(&router, "GET", "/v1/characters", Some(&sync_key), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "separate key should not be rate-limited"
    );
}
