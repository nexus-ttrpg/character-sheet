//! Request/response DTOs for the REST API (§13).
//!
//! These are the wire types — they map to/from the canonical schema types
//! (`CharacterBuild`, `Snapshot`, etc.) in the handlers. Annotated with
//! `utoipa::ToSchema` so OpenAPI is generated from the same types (§13, §14).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ── Character CRUD (§13.2) ──────────────────────────────────────────────────

/// `POST /v1/characters` — create a new character from a canonical build.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCharacterRequest {
    /// The full canonical build for the character's initial snapshot.
    ///
    /// Passed as raw JSON — the handler deserializes into `CharacterBuild`
    /// (which validates structurally via `deny_unknown_fields`, §3.1).
    pub build: serde_json::Value,

    /// The system id — `"pf2e"` or `"sf2e"` (§3.2).
    pub system: String,

    /// Display name (defaults to `build.identity.name` if omitted).
    #[serde(default)]
    pub display_name: Option<String>,
}

/// `POST /v1/characters` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCharacterResponse {
    pub character_id: String,
    pub snapshot_id: String,
    pub content_hash: String,
}

/// `GET /v1/characters/:id` response — the current HEAD build + metadata.
#[derive(Debug, Serialize, ToSchema)]
pub struct GetCharacterResponse {
    pub id: String,
    pub system: String,
    pub display_name: String,
    pub content_hash: String,
    pub head_snapshot_id: Option<String>,
    /// The canonical build as raw JSON (§3.3).
    pub build: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `GET /v1/characters` query params — cursor pagination (§18).
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListCharactersQuery {
    /// Exclusive cursor — return rows with `id > cursor`.
    pub cursor: Option<String>,
    /// Max rows per page (default 50, max 200).
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    50
}

/// `GET /v1/characters` response — a page of characters.
#[derive(Debug, Serialize, ToSchema)]
pub struct ListCharactersResponse {
    pub characters: Vec<CharacterSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CharacterSummary {
    pub id: String,
    pub system: String,
    pub display_name: String,
    pub content_hash: String,
    pub head_snapshot_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// `PATCH /v1/characters/:id` — manual edit → new snapshot (§13.2, §18).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchCharacterRequest {
    /// Canonical field path, e.g. `"build.skills.athletics.rank"`.
    pub path: String,
    /// The new value for that field.
    pub value: serde_json::Value,
    /// Optimistic-locking token — the content hash the client last saw (§18).
    /// Mismatch returns `409`.
    pub expected_content_hash: String,
    /// Human-readable reason for the edit (stored in snapshot provenance).
    #[serde(default)]
    pub reason: Option<String>,
}

/// `PATCH /v1/characters/:id` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PatchCharacterResponse {
    pub character_id: String,
    pub new_snapshot_id: String,
    pub content_hash: String,
    pub diff: Vec<serde_json::Value>,
}

// ── History & rollback (§13.4) ──────────────────────────────────────────────

/// `GET /v1/characters/:id/history` response — the snapshot DAG.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryResponse {
    pub snapshots: Vec<SnapshotSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotSummary {
    pub id: String,
    pub parent: Option<String>,
    pub merge_parent: Option<String>,
    pub content_hash: String,
    pub origin: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// `POST /v1/characters/:id/rollback` — revert to a prior snapshot (§11.4).
#[derive(Debug, Deserialize, ToSchema)]
pub struct RollbackRequest {
    pub to_snapshot_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RollbackResponse {
    pub character_id: String,
    pub new_snapshot_id: String,
    pub content_hash: String,
}

/// `GET /v1/characters/:id/diff?from=…&to=…` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiffResponse {
    pub diffs: Vec<serde_json::Value>,
}

// ── Connections (§13.3) ─────────────────────────────────────────────────────

/// `POST /v1/characters/:id/connections` — register a platform connection.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConnectionRequest {
    pub platform: String,
    pub adapter_id: String,
    /// Platform-specific credentials (encrypted at rest, §17).
    pub credentials: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionResponse {
    pub id: String,
    pub character_id: String,
    pub platform: String,
    pub adapter_id: String,
    pub last_sync_snapshot_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListConnectionsResponse {
    pub connections: Vec<ConnectionResponse>,
}

/// `POST …/pull` — platform → canonical, with optional conflict resolutions.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PullRequest {
    /// Platform-specific payload (converted by the adapter). Mutually
    /// exclusive with `build`.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    /// Pre-converted canonical build (skip the adapter). Mutually exclusive
    /// with `payload`. Passed as raw JSON — the handler deserializes into
    /// `CharacterBuild` for validation.
    #[serde(default)]
    pub build: Option<serde_json::Value>,
    /// Per-field resolutions for a previously-returned conflict (§11.3).
    /// Each entry's `path` must match a conflict path from the 409 response.
    #[serde(default)]
    pub resolutions: Vec<ResolutionDto>,
}

/// A single field resolution (§11.3).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolutionDto {
    /// Canonical field path matching a conflict path.
    pub path: String,
    /// `"ours"`, `"theirs"`, or a custom JSON value.
    pub resolution: serde_json::Value,
}

/// `POST …/pull` response — merged or conflict.
#[derive(Debug, Serialize, ToSchema)]
pub struct PullResponse {
    pub status: String,
    pub new_snapshot_id: Option<String>,
    pub content_hash: Option<String>,
    /// Present only when `status == "conflict"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_preview: Option<serde_json::Value>,
}

/// `POST …/push` — canonical → platform payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct PushResponse {
    pub payload: serde_json::Value,
    pub snapshot_id: String,
    pub content_hash: String,
}

/// `POST …/diff` — dry-run pull, no state change.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionDiffResponse {
    pub merge_preview: serde_json::Value,
}

/// `GET …/drift` — detect_changes result (§6.2, §13.3).
#[derive(Debug, Serialize, ToSchema)]
pub struct DriftResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
}

// ── Webhooks (§13.5) ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWebhookRequest {
    pub url: String,
    pub events: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookResponse {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListWebhooksResponse {
    pub webhooks: Vec<WebhookResponse>,
}
