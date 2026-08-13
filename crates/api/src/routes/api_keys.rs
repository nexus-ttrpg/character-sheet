//! API key CRUD (§13.1, §17).
//!
//! - `POST /v1/api-keys` — create a key. Bootstrap (no auth) when the registry
//!   is empty; `Admin` scope required thereafter.
//! - `GET /v1/api-keys` — list key metadata (no plaintext). `Admin` only.
//! - `DELETE /v1/api-keys/:id` — revoke a key. `Admin` only.

use axum::extract::{Path, State};
use axum::Json;

use crate::auth::{AuthPrincipal, KeyScope};
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
    // Bootstrap: if the registry is empty, anyone can create the first key
    // (§13.1). After that, Admin scope is required.
    {
        let registry = state.api_keys.read().await;
        if !registry.is_empty() {
            let principal = principal.as_ref().ok_or_else(|| {
                ApiError::Unauthorized("authentication required to create API keys".into())
            })?;
            if principal.scope != KeyScope::Admin {
                return Err(ApiError::Forbidden(
                    "only Admin-scope keys can create new API keys".into(),
                ));
            }
        }
    }

    // The new key's owner is the principal's owner, or a fresh ULID for
    // the bootstrap key (which creates the first owner).
    let owner_id = principal
        .as_ref()
        .map(|p| p.owner_id.clone())
        .unwrap_or_else(store::new_ulid);

    let generated = {
        let mut registry = state.api_keys.write().await;
        registry.create(owner_id, req.scope, req.label.clone())
    };

    Ok(Json(CreateApiKeyResponse {
        id: generated.id.to_string(),
        key: generated.key,
        scope: generated.scope,
        label: generated.label,
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

    let keys = {
        let registry = state.api_keys.read().await;
        registry.list()
    };

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

    let revoked = {
        let mut registry = state.api_keys.write().await;
        registry.revoke(&id)
    };

    if !revoked {
        return Err(ApiError::NotFound(format!("API key {id} not found")));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}
