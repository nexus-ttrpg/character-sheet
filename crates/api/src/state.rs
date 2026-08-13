//! Shared application state (§13).
//!
//! `AppState` is the `Clone`-able handle Axum handlers receive via
//! `State<AppState>`. It holds an `Arc<dyn Store>` (backend-agnostic), the
//! adapter bundle directory, the at-rest encryption key for connection
//! credentials (§17), and an in-memory webhook registry.

use std::path::PathBuf;
use std::sync::Arc;

use store::{EncryptionKey, Store};

use crate::auth::{ApiKeyRegistry, RateLimiter};

/// Shared state injected into every Axum handler via `State<AppState>`.
///
/// `Clone` is cheap — everything is behind an `Arc`.
#[derive(Clone)]
pub struct AppState {
    /// The persistence boundary — `PgStore` in production, `SqliteStore` in
    /// tests. The handler code is identical either way (§12).
    pub store: Arc<dyn Store>,

    /// Directory holding adapter bundles (`<adapters_dir>/<adapter_id>/bundle.js`).
    pub adapters_dir: PathBuf,

    /// AES-256-GCM key for encrypting/decrypting connection credentials (§17).
    pub encryption_key: Arc<EncryptionKey>,

    /// Webhook registry (§13.5). In-memory for Phase 2.1; a production
    /// deployment would persist these and sign payloads from a shared secret.
    pub webhooks: Arc<tokio::sync::RwLock<WebhookRegistry>>,

    /// API key registry (§13.1). In-memory for Phase 2; will move to the
    /// store with Argon2 hashing when keys are persisted (§17).
    pub api_keys: Arc<tokio::sync::RwLock<ApiKeyRegistry>>,

    /// Per-key rate limiter (§17). In-memory token bucket; multi-instance
    /// hosted would use Redis.
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(
        store: Arc<dyn Store>,
        adapters_dir: PathBuf,
        encryption_key: EncryptionKey,
    ) -> Self {
        Self {
            store,
            adapters_dir,
            encryption_key: Arc::new(encryption_key),
            webhooks: Arc::new(tokio::sync::RwLock::new(WebhookRegistry::new())),
            api_keys: Arc::new(tokio::sync::RwLock::new(ApiKeyRegistry::new())),
            rate_limiter: Arc::new(RateLimiter::new()),
        }
    }

    /// Like `new`, but with a pre-seeded API key registry (for tests/dev).
    pub fn with_api_keys(
        store: Arc<dyn Store>,
        adapters_dir: PathBuf,
        encryption_key: EncryptionKey,
        api_keys: ApiKeyRegistry,
    ) -> Self {
        Self {
            store,
            adapters_dir,
            encryption_key: Arc::new(encryption_key),
            webhooks: Arc::new(tokio::sync::RwLock::new(WebhookRegistry::new())),
            api_keys: Arc::new(tokio::sync::RwLock::new(api_keys)),
            rate_limiter: Arc::new(RateLimiter::new()),
        }
    }
}

// ── Webhook registry (§13.5) ────────────────────────────────────────────────

/// A registered webhook endpoint (§13.5).
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub secret: String,
}

/// In-memory webhook registry. Phase 2.1 keeps it simple; production would
/// persist to the store and use a shared HMAC secret per webhook.
#[derive(Debug, Default)]
pub struct WebhookRegistry {
    endpoints: Vec<WebhookEndpoint>,
}

impl WebhookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, endpoint: WebhookEndpoint) {
        self.endpoints.push(endpoint);
    }

    pub fn list(&self) -> &[WebhookEndpoint] {
        &self.endpoints
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.endpoints.len();
        self.endpoints.retain(|e| e.id != id);
        self.endpoints.len() < before
    }

    pub fn subscribers_for(&self, event: &str) -> Vec<&WebhookEndpoint> {
        self.endpoints
            .iter()
            .filter(|e| e.events.iter().any(|ev| ev == event))
            .collect()
    }
}
