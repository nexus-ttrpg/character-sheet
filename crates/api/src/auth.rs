//! API key authentication + rate limiting (§13.1, §17).
//!
//! ## Auth (§13.1)
//!
//! API keys (`nsk_live_…`) are the sole machine-to-machine auth mechanism —
//! the Roll20 sandbox, the CLI, and the Foundry relay module can't run
//! interactive OAuth flows (§17). Keys are scoped (`ReadOnly`, `Sync`,
//! `Admin`), and characters belong to the key's owner.
//!
//! Phase 2 uses an in-memory key registry (like the webhook registry); when
//! keys move to the store they'll be Argon2-hashed in the database (§17).
//! For now, keys are SHA-256-hashed and compared in constant time via
//! `subtle` (already a dependency for webhook HMAC verification).
//!
//! ## Rate limiting (§17)
//!
//! Per-key token bucket: read-heavy `GET` endpoints get generous limits,
//! write-heavy `POST`/`PATCH`/`DELETE` endpoints get stricter limits. The
//! spec calls for Redis-backed buckets in multi-instance hosted; Phase 2 uses
//! an in-memory `HashMap`, consistent with the webhook registry approach.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use schema_core::Ulid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;

use crate::state::AppState;

// ── Key scope (§13.1) ────────────────────────────────────────────────────────

/// The permission scope of an API key (§13.1).
///
/// - `ReadOnly` — GET endpoints only.
/// - `Sync` — read + pull/push/diff/drift (the relay module + collector scope).
/// - `Admin` — everything, including key management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeyScope {
    ReadOnly,
    Sync,
    Admin,
}

impl KeyScope {
    /// Check whether this scope permits the given HTTP method (§13.1).
    pub fn allows(&self, method: &Method) -> bool {
        match self {
            KeyScope::ReadOnly => method == Method::GET,
            KeyScope::Sync => method == Method::GET || method == Method::POST,
            KeyScope::Admin => true,
        }
    }
}

// ── Auth principal ───────────────────────────────────────────────────────────

/// The authenticated principal, injected into request extensions by the auth
/// middleware. Handlers extract it via `Extension<AuthPrincipal>`.
#[derive(Debug, Clone)]
pub struct AuthPrincipal {
    pub owner_id: Ulid,
    pub scope: KeyScope,
    pub key_id: Ulid,
}

// ── API key registry (in-memory, §13.1) ──────────────────────────────────────

/// A stored API key (hash only — the plaintext is never retained, §17).
#[derive(Debug, Clone)]
struct StoredKey {
    id: Ulid,
    owner_id: Ulid,
    key_hash: [u8; 32],
    scope: KeyScope,
    label: String,
    created_at: DateTime<Utc>,
}

/// Key metadata returned by the list endpoint (no hash, §17).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyInfo {
    pub id: String,
    pub owner_id: String,
    pub scope: KeyScope,
    pub label: String,
    pub created_at: DateTime<Utc>,
}

/// A freshly generated key — the plaintext is shown once (§17).
#[derive(Debug)]
pub struct GeneratedKey {
    pub id: Ulid,
    pub key: String,
    pub scope: KeyScope,
    pub label: String,
}

/// In-memory API key registry (Phase 2; will move to the Store trait later).
///
/// Thread-safe via `RwLock` — reads (validation) are concurrent, writes (key
/// creation/revocation) are exclusive.
#[derive(Debug, Default)]
pub struct ApiKeyRegistry {
    keys: Vec<StoredKey>,
}

impl ApiKeyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a new API key, store its hash, and return the plaintext (§17).
    ///
    /// The plaintext is shown only here — it is never retrievable after.
    pub fn create(&mut self, owner_id: Ulid, scope: KeyScope, label: String) -> GeneratedKey {
        let id = store::new_ulid();
        let key = generate_key();
        let key_hash = hash_key(&key);
        let label_clone = label.clone();

        self.keys.push(StoredKey {
            id: id.clone(),
            owner_id,
            key_hash,
            scope,
            label,
            created_at: Utc::now(),
        });

        GeneratedKey {
            id,
            key,
            scope,
            label: label_clone,
        }
    }

    /// Validate a plaintext key against stored hashes (constant-time, §17).
    ///
    /// Returns the principal if the key is valid.
    pub fn validate(&self, key: &str) -> Option<AuthPrincipal> {
        let hash = hash_key(key);
        self.keys
            .iter()
            .find(|k| k.key_hash.ct_eq(&hash).into())
            .map(|k| AuthPrincipal {
                owner_id: k.owner_id.clone(),
                scope: k.scope,
                key_id: k.id.clone(),
            })
    }

    /// List all keys (metadata only, no hashes — §17).
    pub fn list(&self) -> Vec<KeyInfo> {
        self.keys
            .iter()
            .map(|k| KeyInfo {
                id: k.id.to_string(),
                owner_id: k.owner_id.to_string(),
                scope: k.scope,
                label: k.label.clone(),
                created_at: k.created_at,
            })
            .collect()
    }

    /// Revoke a key by id. Returns true if a key was removed.
    pub fn revoke(&mut self, id: &str) -> bool {
        let before = self.keys.len();
        self.keys.retain(|k| k.id.as_str() != id);
        self.keys.len() < before
    }

    /// Insert a pre-existing plaintext key (e.g. from `NEXUS_ADMIN_API_KEY`).
    /// The key is hashed before storage — the plaintext is never retained (§17).
    pub fn insert_plaintext(
        &mut self,
        key: String,
        owner_id: Ulid,
        scope: KeyScope,
        label: String,
    ) {
        self.keys.push(StoredKey {
            id: store::new_ulid(),
            owner_id,
            key_hash: hash_key(&key),
            scope,
            label,
            created_at: Utc::now(),
        });
    }

    /// Whether no keys exist yet (bootstrap mode, §13.1).
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Pre-seed a known key for tests/dev (bypasses generation).
    pub fn bootstrap(&mut self, key: &str, owner_id: Ulid, scope: KeyScope, label: &str) -> Ulid {
        let id = store::new_ulid();
        self.keys.push(StoredKey {
            id: id.clone(),
            owner_id,
            key_hash: hash_key(key),
            scope,
            label: label.to_string(),
            created_at: Utc::now(),
        });
        id
    }
}

// ── Key generation + hashing ─────────────────────────────────────────────────

/// Generate a random API key: `nsk_live_` + 32 hex chars (§13.1, §17).
///
/// Uses `/dev/urandom` directly (same pattern as webhook secret generation in
/// `routes/webhooks.rs`) to avoid pulling in a `rand` crate dependency.
pub fn generate_key() -> String {
    let bytes = rand_bytes();
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("nsk_live_{hex}")
}

/// Read 16 random bytes from `/dev/urandom` (Linux). Falls back to zeros on
/// failure (which would be a catastrophic system error — `/dev/urandom` not
/// being available means the OS CSPRNG is broken).
fn rand_bytes() -> [u8; 16] {
    use std::io::Read;
    let mut buf = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

/// SHA-256 hash of a key (constant-time comparison via `subtle`, §17).
fn hash_key(key: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

// ── Auth middleware (§13.1) ──────────────────────────────────────────────────

/// Axum middleware: validate the `Authorization: Bearer nsk_live_…` header
/// and inject `AuthPrincipal` into request extensions.
///
/// Public routes (OpenAPI spec, bootstrap key creation) bypass auth.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let method = req.method().clone();

    // Public: OpenAPI spec.
    if path == "/v1/openapi.json" {
        return next.run(req).await;
    }

    // Public: first key creation when registry is empty (bootstrap, §13.1).
    // Check is_empty() then drop the read lock BEFORE calling next.run() —
    // the handler will acquire a write lock, which would deadlock if we held
    // the read lock.
    if path == "/v1/api-keys" && method == Method::POST {
        let is_bootstrap = state.api_keys.read().await.is_empty();
        if is_bootstrap {
            return next.run(req).await;
        }
    }

    // All other routes require a valid Bearer token.
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(token) = extract_bearer_token(auth_header) else {
        return unauthorized("missing or invalid Authorization header");
    };

    let principal = {
        let registry = state.api_keys.read().await;
        registry.validate(&token)
    };

    let Some(principal) = principal else {
        return unauthorized("invalid API key");
    };

    // Scope check: the key's scope must permit the request method (§13.1).
    if !principal.scope.allows(&method) {
        return forbidden(&format!(
            "key scope {:?} does not allow {}",
            principal.scope, method
        ));
    }

    req.extensions_mut().insert(principal);
    next.run(req).await
}

/// Extract the token from `Bearer nsk_live_…`.
fn extract_bearer_token(header: Option<&str>) -> Option<String> {
    let h = header?;
    let rest = h.strip_prefix("Bearer ")?;
    Some(rest.to_string())
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "error": "unauthorized",
            "message": message,
        })),
    )
        .into_response()
}

fn forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({
            "error": "forbidden",
            "message": message,
        })),
    )
        .into_response()
}

// ── Rate limiter (§17) ───────────────────────────────────────────────────────

/// Per-key token bucket configuration.
#[derive(Debug, Clone, Copy)]
struct BucketConfig {
    capacity: u32,
    refill_per_second: f64,
}

/// Read-heavy endpoints: 100 req/min (§17).
const READ_CONFIG: BucketConfig = BucketConfig {
    capacity: 100,
    refill_per_second: 100.0 / 60.0,
};

/// Write-heavy endpoints: 30 req/min (§17).
const WRITE_CONFIG: BucketConfig = BucketConfig {
    capacity: 30,
    refill_per_second: 30.0 / 60.0,
};

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill: std::time::Instant,
}

impl TokenBucket {
    fn new(capacity: u32) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: std::time::Instant::now(),
        }
    }

    fn refill(&mut self, config: BucketConfig) {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * config.refill_per_second).min(config.capacity as f64);
        self.last_refill = now;
    }

    fn try_take(&mut self, config: BucketConfig) -> bool {
        self.refill(config);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// In-memory per-key rate limiter (§17).
///
/// Phase 2: in-memory `HashMap`. Multi-instance hosted would use Redis.
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: tokio::sync::Mutex<HashMap<Ulid, TokenBucket>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a request is allowed. Returns `false` if rate-limited.
    pub async fn check(&self, key_id: &Ulid, is_read: bool) -> bool {
        let config = if is_read { READ_CONFIG } else { WRITE_CONFIG };
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(key_id.clone())
            .or_insert_with(|| TokenBucket::new(config.capacity));
        bucket.try_take(config)
    }
}

// ── Rate limit middleware (§17) ──────────────────────────────────────────────

/// Axum middleware: enforce per-key rate limits.
///
/// Runs after `auth_middleware` (which inserts `AuthPrincipal` into
/// extensions). Public routes (no principal) bypass rate limiting.
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Only rate-limit authenticated requests.
    let Some(principal) = req.extensions().get::<AuthPrincipal>() else {
        return next.run(req).await;
    };

    let is_read = req.method() == Method::GET;
    let allowed = state.rate_limiter.check(&principal.key_id, is_read).await;

    if !allowed {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, "2")],
            axum::Json(serde_json::json!({
                "error": "rate_limited",
                "message": "too many requests, please slow down",
            })),
        )
            .into_response();
    }

    next.run(req).await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_key_format() {
        let key = generate_key();
        assert!(key.starts_with("nsk_live_"));
        assert_eq!(key.len(), "nsk_live_".len() + 32);
    }

    #[test]
    fn generate_key_unique() {
        let keys: std::collections::HashSet<_> = (0..100).map(|_| generate_key()).collect();
        assert_eq!(keys.len(), 100);
    }

    #[test]
    fn hash_key_deterministic() {
        assert_eq!(hash_key("nsk_live_test"), hash_key("nsk_live_test"));
    }

    #[test]
    fn hash_key_differs() {
        assert_ne!(hash_key("nsk_live_a"), hash_key("nsk_live_b"));
    }

    #[test]
    fn registry_create_and_validate() {
        let mut reg = ApiKeyRegistry::new();
        let owner = store::new_ulid();
        let gen = reg.create(owner.clone(), KeyScope::Admin, "test".into());

        let principal = reg.validate(&gen.key).expect("key should validate");
        assert_eq!(principal.owner_id, owner);
        assert_eq!(principal.scope, KeyScope::Admin);
        assert_eq!(principal.key_id, gen.id);
    }

    #[test]
    fn registry_rejects_unknown_key() {
        let reg = ApiKeyRegistry::new();
        assert!(reg.validate("nsk_live_unknown").is_none());
    }

    #[test]
    fn registry_revoke() {
        let mut reg = ApiKeyRegistry::new();
        let owner = store::new_ulid();
        let gen = reg.create(owner, KeyScope::Admin, "test".into());

        assert!(reg.validate(&gen.key).is_some());
        assert!(reg.revoke(gen.id.as_str()));
        assert!(reg.validate(&gen.key).is_none());
    }

    #[test]
    fn registry_list_excludes_hash() {
        let mut reg = ApiKeyRegistry::new();
        let owner = store::new_ulid();
        reg.create(owner, KeyScope::Sync, "relay".into());

        let list = reg.list();
        assert_eq!(list.len(), 1);
        // KeyInfo has no hash field.
        let json = serde_json::to_string(&list[0]).unwrap();
        assert!(!json.contains("hash"));
    }

    #[test]
    fn registry_bootstrap() {
        let mut reg = ApiKeyRegistry::new();
        let owner = store::new_ulid();
        let id = reg.bootstrap(
            "nsk_live_bootstrapped",
            owner.clone(),
            KeyScope::Admin,
            "bootstrap",
        );

        assert!(!reg.is_empty());
        let principal = reg
            .validate("nsk_live_bootstrapped")
            .expect("bootstrap key");
        assert_eq!(principal.key_id, id);
        assert_eq!(principal.owner_id, owner);
    }

    #[test]
    fn scope_allows() {
        assert!(KeyScope::ReadOnly.allows(&Method::GET));
        assert!(!KeyScope::ReadOnly.allows(&Method::POST));

        assert!(KeyScope::Sync.allows(&Method::GET));
        assert!(KeyScope::Sync.allows(&Method::POST));
        assert!(!KeyScope::Sync.allows(&Method::DELETE));

        assert!(KeyScope::Admin.allows(&Method::GET));
        assert!(KeyScope::Admin.allows(&Method::POST));
        assert!(KeyScope::Admin.allows(&Method::DELETE));
    }

    #[test]
    fn extract_bearer_token_valid() {
        assert_eq!(
            extract_bearer_token(Some("Bearer nsk_live_abc")),
            Some("nsk_live_abc".to_string())
        );
    }

    #[test]
    fn extract_bearer_token_invalid() {
        assert!(extract_bearer_token(None).is_none());
        assert!(extract_bearer_token(Some("Basic abc")).is_none());
        assert!(extract_bearer_token(Some("nsk_live_abc")).is_none());
    }

    #[tokio::test]
    async fn rate_limiter_allows_under_limit() {
        let limiter = RateLimiter::new();
        let key = store::new_ulid();

        // READ_CONFIG.capacity = 100; first 100 should pass.
        for _ in 0..100 {
            assert!(limiter.check(&key, true).await, "should allow under limit");
        }
        // 101st should fail.
        assert!(
            !limiter.check(&key, true).await,
            "should rate-limit over capacity"
        );
    }

    #[tokio::test]
    async fn rate_limiter_separate_keys() {
        let limiter = RateLimiter::new();
        let key_a = store::new_ulid();
        let key_b = store::new_ulid();

        // Exhaust key_a's write bucket (capacity 30).
        for _ in 0..30 {
            assert!(limiter.check(&key_a, false).await);
        }
        assert!(!limiter.check(&key_a, false).await);

        // key_b is unaffected.
        assert!(limiter.check(&key_b, false).await);
    }
}
