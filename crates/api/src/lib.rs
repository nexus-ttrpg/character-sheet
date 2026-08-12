//! Axum API (§13). REST; OpenAPI generated from the same types via utoipa (§14).
//!
//! ## Routes (§13)
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | POST   | /v1/characters | create_character |
//! | GET    | /v1/characters | list_characters |
//! | GET    | /v1/characters/:id | get_character |
//! | PATCH  | /v1/characters/:id | patch_character (optimistic locking §18) |
//! | DELETE | /v1/characters/:id | delete_character |
//! | GET    | /v1/characters/:id/history | get_history |
//! | POST   | /v1/characters/:id/rollback | rollback (§11.4) |
//! | GET    | /v1/characters/:id/diff | diff_snapshots |
//! | POST   | /v1/characters/:id/connections | create_connection |
//! | GET    | /v1/characters/:id/connections | list_connections |
//! | DELETE | /v1/characters/:id/connections/:connId | delete_connection |
//! | POST   | /v1/characters/:id/connections/:connId/pull | pull (§11.3) |
//! | POST   | /v1/characters/:id/connections/:connId/push | push |
//! | POST   | /v1/characters/:id/connections/:connId/diff | connection_diff |
//! | GET    | /v1/characters/:id/connections/:connId/drift | drift (§6.2) |
//! | POST   | /v1/webhooks | create_webhook |
//! | GET    | /v1/webhooks | list_webhooks |
//! | DELETE | /v1/webhooks/:id | delete_webhook |
//! | GET    | /v1/openapi.json | OpenAPI spec |

pub mod adapter_worker;
pub mod dto;
pub mod error;
pub mod patch;
pub mod routes;
pub mod state;

use axum::routing::{delete, get, patch, post};
use axum::Router;
use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa::OpenApi;

use crate::state::AppState;

// ── OpenAPI spec ────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::characters::create_character,
        routes::characters::list_characters,
        routes::characters::get_character,
        routes::characters::patch_character,
        routes::characters::delete_character,
        routes::characters::get_history,
        routes::characters::rollback,
        routes::characters::diff_snapshots,
        routes::connections::create_connection,
        routes::connections::list_connections,
        routes::connections::delete_connection,
        routes::connections::pull,
        routes::connections::push,
        routes::connections::connection_diff,
        routes::connections::drift,
        routes::webhooks::create_webhook,
        routes::webhooks::list_webhooks,
        routes::webhooks::delete_webhook,
    ),
    components(schemas(
        dto::CreateCharacterRequest,
        dto::CreateCharacterResponse,
        dto::GetCharacterResponse,
        dto::ListCharactersResponse,
        dto::CharacterSummary,
        dto::PatchCharacterRequest,
        dto::PatchCharacterResponse,
        dto::HistoryResponse,
        dto::SnapshotSummary,
        dto::RollbackRequest,
        dto::RollbackResponse,
        dto::DiffResponse,
        dto::CreateConnectionRequest,
        dto::ConnectionResponse,
        dto::ListConnectionsResponse,
        dto::PullRequest,
        dto::ResolutionDto,
        dto::PullResponse,
        dto::PushResponse,
        dto::ConnectionDiffResponse,
        dto::DriftResponse,
        dto::CreateWebhookRequest,
        dto::WebhookResponse,
        dto::ListWebhooksResponse,
        error::ErrorBody,
    )),
    info(
        title = "Nexus Sheet API",
        version = "0.1.0",
        description = "Cross-VTT character synchronization (§13). REST; OpenAPI generated from Rust types via utoipa.",
    )
)]
pub struct ApiDoc;

/// Return the generated OpenAPI spec (§13, §14).
pub fn openapi_spec() -> OpenApiSpec {
    ApiDoc::openapi()
}

// ── Router ──────────────────────────────────────────────────────────────────

/// Build the API router with all routes (§13).
pub fn app(state: AppState) -> Router {
    let api_routes = Router::new()
        // Characters (§13.2)
        .route("/v1/characters", post(routes::characters::create_character))
        .route("/v1/characters", get(routes::characters::list_characters))
        .route(
            "/v1/characters/{id}",
            get(routes::characters::get_character),
        )
        .route(
            "/v1/characters/{id}",
            patch(routes::characters::patch_character),
        )
        .route(
            "/v1/characters/{id}",
            delete(routes::characters::delete_character),
        )
        // History & rollback (§13.4)
        .route(
            "/v1/characters/{id}/history",
            get(routes::characters::get_history),
        )
        .route(
            "/v1/characters/{id}/rollback",
            post(routes::characters::rollback),
        )
        .route(
            "/v1/characters/{id}/diff",
            get(routes::characters::diff_snapshots),
        )
        // Connections (§13.3)
        .route(
            "/v1/characters/{id}/connections",
            post(routes::connections::create_connection),
        )
        .route(
            "/v1/characters/{id}/connections",
            get(routes::connections::list_connections),
        )
        .route(
            "/v1/characters/{id}/connections/{conn_id}",
            delete(routes::connections::delete_connection),
        )
        .route(
            "/v1/characters/{id}/connections/{conn_id}/pull",
            post(routes::connections::pull),
        )
        .route(
            "/v1/characters/{id}/connections/{conn_id}/push",
            post(routes::connections::push),
        )
        .route(
            "/v1/characters/{id}/connections/{conn_id}/diff",
            post(routes::connections::connection_diff),
        )
        .route(
            "/v1/characters/{id}/connections/{conn_id}/drift",
            get(routes::connections::drift),
        )
        // Webhooks (§13.5)
        .route("/v1/webhooks", post(routes::webhooks::create_webhook))
        .route("/v1/webhooks", get(routes::webhooks::list_webhooks))
        .route(
            "/v1/webhooks/{id}",
            delete(routes::webhooks::delete_webhook),
        )
        // OpenAPI
        .route("/v1/openapi.json", get(openapi_handler));

    Router::new()
        .merge(api_routes)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// Serve the OpenAPI spec as JSON.
async fn openapi_handler() -> axum::Json<serde_json::Value> {
    let spec = openapi_spec();
    let json = serde_json::to_value(&spec).unwrap_or_default();
    axum::Json(json)
}
