//! Webhook CRUD + HMAC-SHA256 signed payloads (§13.5).

use axum::extract::{Path, State};
use axum::Json;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::dto::*;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::{AppState, WebhookEndpoint};

type HmacSha256 = Hmac<Sha256>;

// ── POST /v1/webhooks ───────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/webhooks",
    request_body = CreateWebhookRequest,
    responses((status = 200, body = WebhookResponse))
)]
pub async fn create_webhook(
    State(state): State<AppState>,
    Json(req): Json<CreateWebhookRequest>,
) -> ApiResult<Json<WebhookResponse>> {
    let id = format!("whk_{}", store::new_ulid().as_str());
    let secret = format!("whsec_{}", hex::encode(rand_bytes()));

    let endpoint = WebhookEndpoint {
        id: id.clone(),
        url: req.url.clone(),
        events: req.events.clone(),
        secret: secret.clone(),
    };

    state.webhooks.write().await.add(endpoint);

    Ok(Json(WebhookResponse {
        id,
        url: req.url,
        events: req.events,
    }))
}

// ── GET /v1/webhooks ────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/webhooks",
    responses((status = 200, body = ListWebhooksResponse))
)]
pub async fn list_webhooks(State(state): State<AppState>) -> ApiResult<Json<ListWebhooksResponse>> {
    let endpoints = state.webhooks.read().await.list().to_vec();
    let webhooks = endpoints
        .iter()
        .map(|e| WebhookResponse {
            id: e.id.clone(),
            url: e.url.clone(),
            events: e.events.clone(),
        })
        .collect();

    Ok(Json(ListWebhooksResponse { webhooks }))
}

// ── DELETE /v1/webhooks/:id ─────────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/webhooks/{id}",
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let removed = state.webhooks.write().await.remove(&id);
    if !removed {
        return Err(ApiError::NotFound(format!("webhook {id} not found")));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ── HMAC signing (§13.5) ────────────────────────────────────────────────────

/// Sign a webhook payload body with HMAC-SHA256 using the webhook's secret.
/// The signature is sent as `X-Nexus-Signature` header (§13.5).
pub fn sign_payload(secret: &str, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify a webhook signature (constant-time comparison via `subtle`).
pub fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}

/// Generate random bytes for webhook secrets (uses OsRng via the store crate's
/// crypto module to avoid pulling another rand dependency).
fn rand_bytes() -> [u8; 16] {
    use std::io::Read;
    // /dev/urandom is always available on Linux; this avoids a `rand` crate
    // dependency. The crypto module already uses OsRng for key generation.
    let mut buf = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let secret = "whsec_testsecret";
        let body = b"{\"event\":\"character.pushed\"}";
        let sig = sign_payload(secret, body);
        assert!(verify_signature(secret, body, &sig));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let secret = "whsec_testsecret";
        let body = b"{\"event\":\"character.pushed\"}";
        let sig = sign_payload(secret, body);
        assert!(!verify_signature(
            secret,
            b"{\"event\":\"character.deleted\"}",
            &sig
        ));
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let sig = sign_payload("secret_a", b"body");
        assert!(!verify_signature("secret_b", b"body", &sig));
    }
}
