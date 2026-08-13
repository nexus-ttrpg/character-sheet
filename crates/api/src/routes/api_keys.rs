//! API key CRUD (§13.1, §17).
//!
//! - `POST /v1/api-keys` — create a key. Bootstrap (no auth) when the store
//!   has no keys; `Admin` scope required thereafter.
//! - `GET /v1/api-keys` — list key metadata (no plaintext). `Admin` only.
//! - `DELETE /v1/api-keys/:id` — revoke a key. `Admin` only.
//!
//! Keys are Argon2-hashed and persisted in the store's `api_keys` table (§17).
//! The plaintext is shown only at creation time. A TTL cache sits in front of
//! the store for validation; the cache is invalidated on key creation/revocation.

use axum::extract::{Path, State};
use axum::Json;

use crate::auth::{hash_key, AuthPrincipal, KeyInfo, KeyScope};
use crate::dto::*;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

// ── POST /v1/api-keys ───────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/api-keys",
    request_body = CreateApiKeyRequest,
    responses(
        (status = 200, body = CreateApiKeyResponse, description = "key created — plaintext shown once (§17)"),
        (status = 401, body = ErrorBody, description = "missing/invalid auth (non-bootstrap)"),
        (status = 403, body = ErrorBody, description = "non-Admin scope"),
    )
)]
pub async fn create_api_key(
    State(state): State<AppState>,
    principal: Option<axum::Extension<AuthPrincipal>>,
    Json(req): Json<CreateApiKeyRequest>,
) -> ApiResult<Json<CreateApiKeyResponse>> {
    // Bootstrap: if the store has no keys, anyone can create the first key
    // (§13.1). After that, Admin scope is required.
    let count = state.store.api_key_count().await?;
    if count > 0 {
        let principal = principal.as_ref().ok_or_else(|| {
            ApiError::Unauthorized("authentication required to create API keys".into())
        })?;
        if principal.scope != KeyScope::Admin {
            return Err(ApiError::Forbidden(
                "only Admin-scope keys can create new API keys".into(),
            ));
        }
    }

    // The new key's owner is the principal's owner, or a fresh ULID for
    // the bootstrap key (which creates the first owner).
    let owner_id = principal
        .as_ref()
        .map(|p| p.owner_id.clone())
        .unwrap_or_else(store::new_ulid);

    let id = store::new_ulid();
    let key = crate::auth::generate_key();
    let key_hash = hash_key(&key);

    state
        .store
        .put_api_key(&store::ApiKeyRecord {
            id: id.clone(),
            owner_id: owner_id.clone(),
            key_hash,
            scope: req.scope.as_str().to_string(),
            label: req.label.clone(),
            created_at: chrono::Utc::now(),
        })
        .await?;

    // Invalidate cache so the new key is immediately usable.
    state.api_key_cache.invalidate_all().await;

    Ok(Json(CreateApiKeyResponse {
        id: id.to_string(),
        key,
        scope: req.scope,
        label: req.label,
    }))
}

// ── GET /v1/api-keys ────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/api-keys",
    responses(
        (status = 200, body = ListApiKeysResponse),
        (status = 403, body = ErrorBody, description = "non-Admin scope"),
    )
)]
pub async fn list_api_keys(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
) -> ApiResult<Json<ListApiKeysResponse>> {
    if principal.scope != KeyScope::Admin {
        return Err(ApiError::Forbidden(
            "only Admin-scope keys can list API keys".into(),
        ));
    }

    let records = state.store.list_api_keys().await?;
    let keys: Vec<KeyInfo> = records
        .iter()
        .map(|r| KeyInfo {
            id: r.id.to_string(),
            owner_id: r.owner_id.to_string(),
            scope: r.scope.parse().unwrap_or(KeyScope::ReadOnly),
            label: r.label.clone(),
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(ListApiKeysResponse { keys }))
}

// ── DELETE /v1/api-keys/:id ─────────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/api-keys/{id}",
    responses(
        (status = 204),
        (status = 403, body = ErrorBody, description = "non-Admin scope"),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete_api_key(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    if principal.scope != KeyScope::Admin {
        return Err(ApiError::Forbidden(
            "only Admin-scope keys can revoke API keys".into(),
        ));
    }

    let ulid = store::parse_ulid(&id)
        .ok_or_else(|| ApiError::NotFound(format!("API key {id} not found")))?;
    let revoked = state.store.revoke_api_key(&ulid).await?;

    if !revoked {
        return Err(ApiError::NotFound(format!("API key {id} not found")));
    }

    // Invalidate cache so the revoked key is immediately rejected.
    state.api_key_cache.invalidate_all().await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
