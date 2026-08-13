//! Character CRUD, history, rollback, diff (§13.2, §13.4).

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use schema_core::{content_hash, testing::minimal_build, CharacterBuild, CharacterMeta, SystemId};
use store::{new_ulid, parse_ulid};
use sync_engine::{diff, Snapshot, SnapshotOrigin};

use crate::auth::AuthPrincipal;
use crate::dto::*;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::patch::{field_diffs_to_json, set_at_path};
use crate::state::AppState;

// ── POST /v1/characters ─────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/characters",
    request_body = CreateCharacterRequest,
    responses(
        (status = 200, body = CreateCharacterResponse),
        (status = 400, body = ErrorBody),
    )
)]
pub async fn create_character(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Json(req): Json<CreateCharacterRequest>,
) -> ApiResult<Json<CreateCharacterResponse>> {
    let build: CharacterBuild = serde_json::from_value(req.build)?;
    let system = SystemId::new(&req.system);
    let display_name = req
        .display_name
        .unwrap_or_else(|| build.identity.name.clone());
    let content_hash = content_hash(&build);

    let char_id = new_ulid();
    let snap_id = new_ulid();
    let owner_id = principal.owner_id.clone();
    let now = Utc::now();

    let meta = CharacterMeta {
        id: char_id.clone(),
        schema_version: semver::Version::new(0, 1, 0),
        system,
        owner_id: owner_id.clone(),
        display_name,
        created_at: now,
        updated_at: now,
        content_hash,
    };

    state.store.create_character(&meta).await?;

    let snapshot = Snapshot::new(
        snap_id.clone(),
        char_id.clone(),
        None,
        build,
        SnapshotOrigin::ManualEdit,
        now,
        owner_id,
    );

    state.store.put_snapshot(&snapshot).await?;
    state
        .store
        .set_head_snapshot(&char_id, &snap_id, &content_hash)
        .await?;

    Ok(Json(CreateCharacterResponse {
        character_id: char_id.to_string(),
        snapshot_id: snap_id.to_string(),
        content_hash: content_hash.to_hex(),
    }))
}

// ── GET /v1/characters ──────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters",
    params(ListCharactersQuery),
    responses(
        (status = 200, body = ListCharactersResponse),
    )
)]
pub async fn list_characters(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Query(q): Query<ListCharactersQuery>,
) -> ApiResult<Json<ListCharactersResponse>> {
    let owner = principal.owner_id.clone();
    let limit = q.limit.min(200);

    let cursor = match &q.cursor {
        Some(c) => {
            Some(parse_ulid(c).ok_or_else(|| ApiError::BadRequest("invalid cursor ULID".into()))?)
        }
        None => None,
    };

    let page = state
        .store
        .list_characters(&owner, cursor.as_ref(), limit)
        .await?;

    let characters = page
        .rows
        .iter()
        .map(|r| CharacterSummary {
            id: r.id.to_string(),
            system: r.system.as_str().into(),
            display_name: r.display_name.clone(),
            content_hash: r.content_hash.to_hex(),
            head_snapshot_id: r.head_snapshot_id.as_ref().map(|u| u.to_string()),
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(ListCharactersResponse {
        characters,
        next_cursor: page.next_cursor.map(|u| u.to_string()),
    }))
}

// ── GET /v1/characters/:id ──────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters/{id}",
    responses(
        (status = 200, body = GetCharacterResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn get_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<GetCharacterResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let row = state
        .store
        .get_character(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} not found")))?;

    let head = state.store.get_head_snapshot(&cid).await?;

    let build = match head {
        Some(snap) => serde_json::to_value(&snap.build)?,
        None => serde_json::to_value(minimal_build())?,
    };

    Ok(Json(GetCharacterResponse {
        id: row.id.to_string(),
        system: row.system.as_str().into(),
        display_name: row.display_name,
        content_hash: row.content_hash.to_hex(),
        head_snapshot_id: row.head_snapshot_id.map(|u| u.to_string()),
        build,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

// ── PATCH /v1/characters/:id ────────────────────────────────────────────────

#[utoipa::path(
    patch,
    path = "/v1/characters/{id}",
    request_body = PatchCharacterRequest,
    responses(
        (status = 200, body = PatchCharacterResponse),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody, description = "content hash mismatch (§18)"),
    )
)]
pub async fn patch_character(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Json(req): Json<PatchCharacterRequest>,
) -> ApiResult<Json<PatchCharacterResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let head = state
        .store
        .get_head_snapshot(&cid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("character {id} has no HEAD")))?;

    // Optimistic locking (§18): the client's expected hash must match HEAD.
    let head_hash_hex = head.content_hash.to_hex();
    if req.expected_content_hash != head_hash_hex {
        return Err(ApiError::Stale {
            expected: head_hash_hex,
            got: req.expected_content_hash,
        });
    }

    // Apply the patch to the build as JSON, then re-deserialize to validate.
    let mut build_val = serde_json::to_value(&head.build)?;
    set_at_path(&mut build_val, &req.path, req.value)?;
    let new_build: CharacterBuild = serde_json::from_value(build_val)?;
    let new_hash = content_hash(&new_build);

    // If the patch didn't actually change anything, short-circuit.
    if new_hash == head.content_hash {
        return Ok(Json(PatchCharacterResponse {
            character_id: cid.to_string(),
            new_snapshot_id: head.id.to_string(),
            content_hash: head.content_hash.to_hex(),
            diff: vec![],
        }));
    }

    let snap_id = new_ulid();
    let owner_id = principal.owner_id.clone();
    let now = Utc::now();

    let snapshot = Snapshot::new(
        snap_id.clone(),
        cid.clone(),
        Some(head.id.clone()),
        new_build.clone(),
        SnapshotOrigin::ManualEdit,
        now,
        owner_id,
    );

    // Idempotency check (§12).
    if let Some(existing) = state
        .store
        .find_snapshot_by_hash(&cid, &new_hash, Some(&head.id))
        .await?
    {
        let diffs = diff(&head.build, &existing.build);
        return Ok(Json(PatchCharacterResponse {
            character_id: cid.to_string(),
            new_snapshot_id: existing.id.to_string(),
            content_hash: new_hash.to_hex(),
            diff: field_diffs_to_json(&diffs),
        }));
    }

    state.store.put_snapshot(&snapshot).await?;
    state
        .store
        .set_head_snapshot(&cid, &snap_id, &new_hash)
        .await?;

    let diffs = diff(&head.build, &new_build);

    Ok(Json(PatchCharacterResponse {
        character_id: cid.to_string(),
        new_snapshot_id: snap_id.to_string(),
        content_hash: new_hash.to_hex(),
        diff: field_diffs_to_json(&diffs),
    }))
}

// ── DELETE /v1/characters/:id ───────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/characters/{id}",
    responses(
        (status = 204),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    state.store.delete_character(&cid).await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ── GET /v1/characters/:id/history ──────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters/{id}/history",
    responses(
        (status = 200, body = HistoryResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn get_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<HistoryResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let page = state.store.list_history(&cid, None, 200).await?;

    let snapshots = page
        .rows
        .iter()
        .map(|s| SnapshotSummary {
            id: s.id.to_string(),
            parent: s.parent.as_ref().map(|u| u.to_string()),
            merge_parent: s.merge_parent.as_ref().map(|u| u.to_string()),
            content_hash: s.content_hash.to_hex(),
            origin: serde_json::to_value(&s.origin).unwrap_or_default(),
            created_at: s.created_at,
            created_by: s.created_by.to_string(),
        })
        .collect();

    Ok(Json(HistoryResponse {
        snapshots,
        next_cursor: page.next_cursor.map(|u| u.to_string()),
    }))
}

// ── POST /v1/characters/:id/rollback ────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/characters/{id}/rollback",
    request_body = RollbackRequest,
    responses(
        (status = 200, body = RollbackResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn rollback(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Json(req): Json<RollbackRequest>,
) -> ApiResult<Json<RollbackResponse>> {
    let cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let target_snap_id = parse_ulid(&req.to_snapshot_id).ok_or_else(|| {
        ApiError::BadRequest(format!("invalid snapshot ULID: {}", req.to_snapshot_id))
    })?;

    let target = state
        .store
        .get_snapshot(&target_snap_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("snapshot {} not found", req.to_snapshot_id)))?;

    let head = state.store.get_head_snapshot(&cid).await?;

    let content_hash = target.content_hash;
    let snap_id = new_ulid();
    let owner_id = principal.owner_id.clone();
    let now = Utc::now();

    // Rollback is `revert`, not `reset --hard` (§11.4): a new snapshot carrying
    // the old content, with a Rollback origin pointing at the target.
    let snapshot = Snapshot::new_rollback(
        snap_id.clone(),
        cid.clone(),
        head.as_ref().map(|h| h.id.clone()),
        target.build.clone(),
        target_snap_id.clone(),
        now,
        owner_id,
    );

    state.store.put_snapshot(&snapshot).await?;
    state
        .store
        .set_head_snapshot(&cid, &snap_id, &content_hash)
        .await?;

    Ok(Json(RollbackResponse {
        character_id: cid.to_string(),
        new_snapshot_id: snap_id.to_string(),
        content_hash: content_hash.to_hex(),
    }))
}

// ── GET /v1/characters/:id/diff?from=…&to=… ─────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/characters/{id}/diff",
    params(
        ("from" = String, Query, description = "Snapshot ULID"),
        ("to" = String, Query, description = "Snapshot ULID"),
    ),
    responses(
        (status = 200, body = DiffResponse),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn diff_snapshots(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Json<DiffResponse>> {
    let _cid = parse_ulid(&id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid character ULID: {id}")))?;

    let from_id = params
        .get("from")
        .ok_or_else(|| ApiError::BadRequest("missing 'from' parameter".into()))?;
    let to_id = params
        .get("to")
        .ok_or_else(|| ApiError::BadRequest("missing 'to' parameter".into()))?;

    let from_ulid = parse_ulid(from_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid snapshot ULID: {from_id}")))?;
    let to_ulid = parse_ulid(to_id)
        .ok_or_else(|| ApiError::BadRequest(format!("invalid snapshot ULID: {to_id}")))?;

    let from_snap = state
        .store
        .get_snapshot(&from_ulid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("snapshot {from_id} not found")))?;
    let to_snap = state
        .store
        .get_snapshot(&to_ulid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("snapshot {to_id} not found")))?;

    let diffs = diff(&from_snap.build, &to_snap.build);

    Ok(Json(DiffResponse {
        diffs: field_diffs_to_json(&diffs),
    }))
}
