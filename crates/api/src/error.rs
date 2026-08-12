//! API error → HTTP response mapping (§13).
//!
//! The key mappings:
//! - `Conflict` → `409` (merge conflicts, optimistic-lock mismatch §18)
//! - `NotFound` → `404`
//! - `BadRequest` → `400`
//! - `Stale` → `409` (content hash mismatch on PATCH)
//! - `Store` → `500`
//! - `Adapter` → `502` (upstream adapter failure)

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// Errors produced by API handlers, mapped to HTTP status codes.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("content hash mismatch: expected {expected}, got {got}")]
    Stale { expected: String, got: String },

    #[error("merge conflict: {} unresolved field(s)", merge_preview.conflicts.len())]
    Conflict {
        merge_preview: sync_engine::MergePreview,
    },

    #[error("adapter error: {0}")]
    Adapter(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("internal error: {0}")]
    Internal(String),
}

/// The JSON body returned on error.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_preview: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ApiError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "not_found".into(),
                    message: msg.clone(),
                    merge_preview: None,
                    current_hash: None,
                },
            ),
            ApiError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "bad_request".into(),
                    message: msg.clone(),
                    merge_preview: None,
                    current_hash: None,
                },
            ),
            ApiError::Stale { expected, got: _ } => (
                StatusCode::CONFLICT,
                ErrorBody {
                    error: "stale_content_hash".into(),
                    message: self.to_string(),
                    merge_preview: None,
                    current_hash: Some(expected.clone()),
                },
            ),
            ApiError::Conflict { merge_preview } => {
                let mp_json = crate::patch::merge_preview_to_json(merge_preview);
                let n = merge_preview.conflicts.len();
                (
                    StatusCode::CONFLICT,
                    ErrorBody {
                        error: "conflict".into(),
                        message: format!("{n} unresolved field(s)"),
                        merge_preview: Some(mp_json),
                        current_hash: None,
                    },
                )
            }
            ApiError::Adapter(msg) => (
                StatusCode::BAD_GATEWAY,
                ErrorBody {
                    error: "adapter_error".into(),
                    message: msg.clone(),
                    merge_preview: None,
                    current_hash: None,
                },
            ),
            ApiError::Store(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    error: "store_error".into(),
                    message: msg.clone(),
                    merge_preview: None,
                    current_hash: None,
                },
            ),
            ApiError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    error: "internal_error".into(),
                    message: msg.clone(),
                    merge_preview: None,
                    current_hash: None,
                },
            ),
        };
        (status, Json(body)).into_response()
    }
}

impl From<store::StoreError> for ApiError {
    fn from(e: store::StoreError) -> Self {
        match e {
            store::StoreError::NotFound(msg) => ApiError::NotFound(msg),
            other => ApiError::Store(other.to_string()),
        }
    }
}

impl From<adapter_host::HostError> for ApiError {
    fn from(e: adapter_host::HostError) -> Self {
        ApiError::Adapter(e.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::BadRequest(format!("serialization error: {e}"))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
