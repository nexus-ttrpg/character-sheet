//! Connections, pull/push/diff/drift (§13.3).
//!
//! The `pull` endpoint is the three-way merge flow (§11.3):
//! 1. Get the connection → adapter_id, credentials, last_sync_snapshot_id (ancestor)
//! 2. Get HEAD → ours
//! 3. Get theirs → from `req.build` (pre-converted) or adapter.import(`req.payload`)
//! 4. ancestor = last_sync_snapshot build, or HEAD if no prior sync (first pull)
//! 5. merge(ancestor, ours, theirs)
//! 6. If conflicts and no resolutions → 409 + mergePreview
//! 7. If clean (or all resolved) → apply merge, create merge snapshot, update HEAD + connection

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use secrecy::ExposeSecret;

use schema_core::{content_hash, testing::minimal_build, CharacterBuild, SystemId};
use store::{new_ulid, parse_ulid, ConnectionRecord};
use sync_engine::{apply_merge, merge, merge::Resolution, merge_preview_to_json, Snapshot};

use crate::adapter_worker;
use crate::auth::AuthPrincipal;
use crate::dto::*;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

// ── POST /v1/characters/:id/connections ─────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/characters/{id}/connections",
    request_body = CreateConnectionRequest,
    responses(
        (status = 200, body = ConnectionResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn create_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateConnectionRequest>,
) -> ApiResult<Json<ConnectionResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    // Verify the character exists.
    let _char = state
        .store
        .get_character(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} not found")))?;

    let conn_id = new_ulid();
    let now = Utc::now();
    let credentials_str = serde_json::to_string(&req.credentials)?;

    let conn = ConnectionRecord {
        id: conn_id.clone(),
        character_id: cid.clone(),
        platform: req.platform.as_str().into(),
        adapter_id: req.adapter_id.as_str().into(),
        credentials: secrecy::SecretString::from(credentials_str),
        last_sync_snapshot_id: None,
        created_at: now,
        updated_at: now,
    };

    state
        .store
        .put_connection(&conn, &state.encryption_key)
        .await?;

    Ok(Json(ConnectionResponse {
        id: conn_id.to_string(),
        character_id: cid.to_string(),
        platform: req.platform,
        adapter_id: req.adapter_id,
        last_sync_snapshot_id: None,
        created_at: now,
        updated_at: now,
    }))
}

// ── GET /v1/characters/:id/connections ──────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters/{id}/connections",
    responses(
        (status = 200, body = ListConnectionsResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn list_connections(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ListConnectionsResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let conns = state
        .store
        .list_connections(&cid, &state.encryption_key)
        .await?;

    let connections = conns
        .iter()
        .map(|c| ConnectionResponse {
            id: c.id.to_string(),
            character_id: c.character_id.to_string(),
            platform: c.platform.as_str().into(),
            adapter_id: c.adapter_id.as_str().into(),
            last_sync_snapshot_id: c.last_sync_snapshot_id.as_ref().map(|u| u.to_string()),
            created_at: c.created_at,
            updated_at: c.updated_at,
        })
        .collect();

    Ok(Json(ListConnectionsResponse { connections }))
}

// ── DELETE /v1/characters/:id/connections/:connId ───────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/characters/{id}/connections/{conn_id}",
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete_connection(
    State(state): State<AppState>,
    Path((id, conn_id)): Path<(String, String)>,
) -> ApiResult<axum::http::StatusCode> {
    let _cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;
    let conn_ulid = parse_ulid(&conn_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid connection ULID: {conn_id}")))?;

    // The store trait doesn't have delete_connection yet — for now, this is a
    // no-op ack. TODO: add delete_connection to the Store trait.
    let _ = state
        .store
        .get_connection(&conn_ulid, &state.encryption_key)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connection {conn_id} not found")))?;

    // Note: Store::delete_connection is not yet implemented in the trait.
    // When it lands, call it here. For now, return 204 as a soft ack.
    tracing::warn!(
        "delete_connection not yet implemented in Store trait — returning 204 without deleting"
    );

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ── POST …/pull — platform → canonical (three-way merge, §11.3) ─────────────

#[utoipa::path(
    post,
    path = "/v1/characters/{id}/connections/{conn_id}/pull",
    request_body = PullRequest,
    responses(
        (status = 200, body = PullResponse, description = "merged or no-op"),
        (status = 409, body = ErrorBody, description = "conflict — mergePreview returned for human resolution"),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn pull(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Path((id, conn_id)): Path<(String, String)>,
    Json(req): Json<PullRequest>,
) -> ApiResult<Json<PullResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;
    let conn_ulid = parse_ulid(&conn_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid connection ULID: {conn_id}")))?;

    // 1. Get the connection.
    let conn = state
        .store
        .get_connection(&conn_ulid, &state.encryption_key)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connection {conn_id} not found")))?;

    // 2. Get HEAD (ours).
    let head = state
        .store
        .get_head_snapshot(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} has no HEAD")))?;
    let ours = head.build.clone();

    let theirs = if let Some(build_val) = req.build {
        serde_json::from_value::<CharacterBuild>(build_val)?
    } else if let Some(payload) = req.payload {
        let credentials =
            serde_json::from_str::<serde_json::Value>(conn.credentials.expose_secret())
                .unwrap_or(serde_json::json!({}));

        let outcome = adapter_worker::adapter_import(
            state.adapters_dir.clone(),
            conn.adapter_id.as_str().into(),
            payload,
            SystemId::new("pf2e"),
            credentials,
        )
        .await?;

        // Complete the partial build with minimal defaults (same as CLI).
        complete_build(outcome.build)
    } else {
        return Err(ApiError::BadRequest(
            "either 'payload' or 'build' must be present".into(),
        ));
    };

    let ancestor = match &conn.last_sync_snapshot_id {
        Some(sid) => state
            .store
            .get_snapshot(sid)
            .await?
            .map(|s| s.build)
            .unwrap_or_else(|| ours.clone()),
        None => ours.clone(),
    };

    // 5. Three-way merge (§11.3).
    let preview = merge(&ancestor, &ours, &theirs);

    // 6. If conflicts and no resolutions → 409 + mergePreview.
    if !preview.conflicts.is_empty() && req.resolutions.is_empty() {
        return Err(ApiError::Conflict {
            merge_preview: preview,
        });
    }

    // 7. If conflicts with resolutions, verify all are covered.
    let resolutions: Vec<(String, Resolution)> = req
        .resolutions
        .iter()
        .map(|r| {
            let res = match &r.resolution {
                serde_json::Value::String(s) if s == "ours" => Resolution::Ours,
                serde_json::Value::String(s) if s == "theirs" => Resolution::Theirs,
                other => Resolution::Custom(other.clone()),
            };
            (r.path.clone(), res)
        })
        .collect();

    if !preview.conflicts.is_empty() {
        let conflict_paths: std::collections::HashSet<&str> =
            preview.conflicts.iter().map(|c| c.path.as_str()).collect();
        let resolved_paths: std::collections::HashSet<&str> =
            resolutions.iter().map(|(p, _)| p.as_str()).collect();
        let unresolved: Vec<_> = conflict_paths.difference(&resolved_paths).collect();
        if !unresolved.is_empty() {
            return Err(ApiError::Conflict {
                merge_preview: preview,
            });
        }
    }

    // 8. Apply the merge to produce the final build.
    //    Diff paths are prefixed with `build.` (diff.rs §11.2), so we wrap
    //    ours in {"build": ...} for apply_merge, then unwrap after.
    let ours_val = serde_json::json!({"build": serde_json::to_value(&ours)?});
    let merged_val = apply_merge(&ours_val, &preview, &resolutions)?;
    let merged_build: CharacterBuild = serde_json::from_value(merged_val["build"].clone())?;
    let merged_hash = content_hash(&merged_build);

    // 9. Idempotency: if the merged content equals HEAD, short-circuit.
    if merged_hash == head.content_hash {
        return Ok(Json(PullResponse {
            status: "no_changes".into(),
            new_snapshot_id: Some(head.id.to_string()),
            content_hash: Some(head.content_hash.to_hex()),
            merge_preview: Some(merge_preview_to_json(&preview)),
        }));
    }

    // 10. Create a merge snapshot (§11.1).
    let snap_id = new_ulid();
    let owner_id = principal.owner_id.clone();
    let now = Utc::now();
    let resolved_count = preview.conflicts.len() as u32;

    let snapshot = Snapshot::new_merge(
        snap_id.clone(),
        cid.clone(),
        Some(head.id.clone()),
        conn.last_sync_snapshot_id
            .clone()
            .unwrap_or_else(|| head.id.clone()),
        merged_build,
        resolved_count,
        now,
        owner_id,
    );

    // Idempotency check (§12).
    if let Some(existing) = state
        .store
        .find_snapshot_by_hash(&cid, &merged_hash, Some(&head.id))
        .await?
    {
        state
            .store
            .update_connection_sync(&conn_ulid, &existing.id)
            .await?;
        return Ok(Json(PullResponse {
            status: "merged".into(),
            new_snapshot_id: Some(existing.id.to_string()),
            content_hash: Some(merged_hash.to_hex()),
            merge_preview: Some(merge_preview_to_json(&preview)),
        }));
    }

    state.store.put_snapshot(&snapshot).await?;
    state
        .store
        .set_head_snapshot(&cid, &snap_id, &merged_hash)
        .await?;
    state
        .store
        .update_connection_sync(&conn_ulid, &snap_id)
        .await?;

    Ok(Json(PullResponse {
        status: "merged".into(),
        new_snapshot_id: Some(snap_id.to_string()),
        content_hash: Some(merged_hash.to_hex()),
        merge_preview: Some(merge_preview_to_json(&preview)),
    }))
}

// ── POST …/push — canonical → platform (§13.3) ──────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/characters/{id}/connections/{conn_id}/push",
    responses(
        (status = 200, body = PushResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn push(
    State(state): State<AppState>,
    Path((id, conn_id)): Path<(String, String)>,
) -> ApiResult<Json<PushResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;
    let conn_ulid = parse_ulid(&conn_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid connection ULID: {conn_id}")))?;

    let conn = state
        .store
        .get_connection(&conn_ulid, &state.encryption_key)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connection {conn_id} not found")))?;

    let head = state
        .store
        .get_head_snapshot(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} has no HEAD")))?;

    let build_json = serde_json::to_value(&head.build)?;

    let credentials = serde_json::from_str::<serde_json::Value>(conn.credentials.expose_secret())
        .unwrap_or(serde_json::json!({}));

    let outcome = adapter_worker::adapter_export(
        state.adapters_dir.clone(),
        conn.adapter_id.as_str().into(),
        build_json,
        SystemId::new("pf2e"),
        credentials,
    )
    .await?;

    Ok(Json(PushResponse {
        payload: outcome.payload,
        snapshot_id: head.id.to_string(),
        content_hash: head.content_hash.to_hex(),
    }))
}

// ── POST …/diff — dry-run pull, no state change (§13.3) ─────────────────────

#[utoipa::path(
    post,
    path = "/v1/characters/{id}/connections/{conn_id}/diff",
    request_body = PullRequest,
    responses(
        (status = 200, body = ConnectionDiffResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn connection_diff(
    State(state): State<AppState>,
    Path((id, conn_id)): Path<(String, String)>,
    Json(req): Json<PullRequest>,
) -> ApiResult<Json<ConnectionDiffResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;
    let conn_ulid = parse_ulid(&conn_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid connection ULID: {conn_id}")))?;

    let conn = state
        .store
        .get_connection(&conn_ulid, &state.encryption_key)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connection {conn_id} not found")))?;

    let head = state
        .store
        .get_head_snapshot(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} has no HEAD")))?;
    let ours = head.build.clone();

    let theirs = if let Some(build_val) = req.build {
        serde_json::from_value::<CharacterBuild>(build_val)?
    } else if let Some(payload) = req.payload {
        let credentials =
            serde_json::from_str::<serde_json::Value>(conn.credentials.expose_secret())
                .unwrap_or(serde_json::json!({}));

        let outcome = adapter_worker::adapter_import(
            state.adapters_dir.clone(),
            conn.adapter_id.as_str().into(),
            payload,
            SystemId::new("pf2e"),
            credentials,
        )
        .await?;

        complete_build(outcome.build)
    } else {
        return Err(ApiError::BadRequest(
            "either 'payload' or 'build' must be present".into(),
        ));
    };

    let ancestor = match &conn.last_sync_snapshot_id {
        Some(sid) => state
            .store
            .get_snapshot(sid)
            .await?
            .map(|s| s.build)
            .unwrap_or_else(|| ours.clone()),
        None => ours.clone(),
    };

    let preview = merge(&ancestor, &ours, &theirs);

    Ok(Json(ConnectionDiffResponse {
        merge_preview: merge_preview_to_json(&preview),
    }))
}

// ── GET …/drift — detect_changes result (§6.2, §13.3) ───────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters/{id}/connections/{conn_id}/drift",
    responses(
        (status = 200, body = DriftResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn drift(
    State(_state): State<AppState>,
    Path((id, conn_id)): Path<(String, String)>,
) -> ApiResult<Json<DriftResponse>> {
    let _cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;
    let _conn_ulid = parse_ulid(&conn_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid connection ULID: {conn_id}")))?;

    // The server cannot reach the platform sandbox directly — drift detection
    // is initiated by the collector/relay, not polled from the server (§6.2).
    // Return `unavailable` with an honest reason rather than a stale "no changes".
    Ok(Json(DriftResponse {
        status: "unavailable".into(),
        reason: Some("sandboxNotRunning".into()),
        last_seen_at: None,
    }))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a `PartialCharacterBuild` (adapter output) into a full
/// `CharacterBuild` by overlaying present fields on a minimal base (§5.1).
/// Mirrors the CLI's `complete_build`.
fn complete_build(partial: adapter_host::PartialCharacterBuild) -> CharacterBuild {
    let mut build = minimal_build();
    if let Some(v) = partial.identity {
        build.identity = v;
    }
    if let Some(v) = partial.ancestry {
        build.ancestry = v;
    }
    if let Some(v) = partial.heritage {
        build.heritage = v;
    }
    if let Some(v) = partial.background {
        build.background = v;
    }
    if let Some(v) = partial.class {
        build.class = v;
    }
    if let Some(v) = partial.level {
        build.level = v;
    }
    if let Some(v) = partial.ability_scores {
        build.ability_scores = v;
    }
    if let Some(v) = partial.skills {
        build.skills = v;
    }
    if let Some(v) = partial.proficiencies {
        build.proficiencies = v;
    }
    if let Some(v) = partial.feats {
        build.feats = v;
    }
    if let Some(v) = partial.class_features {
        build.class_features = v;
    }
    if let Some(v) = partial.equipment {
        build.equipment = v;
    }
    if let Some(v) = partial.currencies {
        build.currencies = v;
    }
    if let Some(v) = partial.spellcasting {
        build.spellcasting = v;
    }
    if let Some(v) = partial.actions {
        build.actions = v;
    }
    if let Some(v) = partial.reactions {
        build.reactions = v;
    }
    if let Some(v) = partial.exploration_activities {
        build.exploration_activities = v;
    }
    if let Some(v) = partial.companions {
        build.companions = v;
    }
    if let Some(v) = partial.familiars {
        build.familiars = v;
    }
    if let Some(v) = partial.drones {
        build.drones = v;
    }
    if let Some(v) = partial.vehicles {
        build.vehicles = v;
    }
    if let Some(v) = partial.notes {
        build.notes = v;
    }
    build
}
