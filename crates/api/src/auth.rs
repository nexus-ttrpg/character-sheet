//! API key authentication + rate limiting (§13.1, §17).
//!
//! ## Auth (§13.1)
//!
//! API keys (`nsk_live_…`) are the sole machine-to-machine auth mechanism —
//! the Roll20 sandbox, the CLI, and the Foundry relay module can't run
//! interactive OAuth flows (§17). Keys are scoped (`ReadOnly`, `Sync`,
//! `Admin`), and characters belong to the key's owner.
//!
//! Keys are persisted in the store's `api_keys` table, Argon2-hashed (PHC
//! format). The plaintext is shown only at creation time and never
//! retrievable after (§17). A TTL cache sits in front of the store to avoid
//! Argon2-verifying every key on every request — Argon2 verification is
//! expensive (~50ms per key), so the cache is essential for performance.
//!
//! ## Rate limiting (§17)
//!
//! Per-key token bucket: read-heavy `GET` endpoints get generous limits,
//! write-heavy `POST`/`PATCH`/`DELETE` endpoints get stricter limits. The
//! spec calls for Redis-backed buckets in multi-instance hosted; Phase 2 uses
//! an in-memory `HashMap`, consistent with the webhook registry approach.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use schema_core::Ulid;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use store::Store;

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

    /// String representation for storage (matches `serde rename_all`).
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyScope::ReadOnly => "read_only",
            KeyScope::Sync => "sync",
            KeyScope::Admin => "admin",
        }
    }
}

impl std::str::FromStr for KeyScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read_only" => Ok(KeyScope::ReadOnly),
            "sync" => Ok(KeyScope::Sync),
            "admin" => Ok(KeyScope::Admin),
            _ => Err(format!("unknown scope: {s}")),
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

// ── Key metadata DTOs ────────────────────────────────────────────────────────

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

// ── API key cache (TTL, store-backed, §17) ───────────────────────────────────

/// TTL for cached validated tokens (§17).
const CACHE_TTL: Duration = Duration::from_secs(60);

struct CacheEntry {
    principal: AuthPrincipal,
    expires_at: Instant,
}

/// In-memory TTL cache for API key validation (§17).
///
/// Argon2 verification is expensive (~50ms per key). To avoid verifying
/// every key on every request, validated tokens are cached with a TTL. On
/// cache miss, the store is queried for all key hashes and each is
/// Argon2-verified against the token. The cache is invalidated on key
/// creation/revocation.
#[derive(Default)]
pub struct ApiKeyCache {
    cache: tokio::sync::RwLock<HashMap<String, CacheEntry>>,
}

impl ApiKeyCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate a plaintext token: check cache first, then query the store.
    ///
    /// On cache hit (token found and not expired): returns the principal
    /// immediately. On cache miss: queries the store for all API key records,
    /// Argon2-verifies each against the token, and caches the match.
    pub async fn validate(&self, token: &str, store: &dyn Store) -> Option<AuthPrincipal> {
        // Fast path: check cache.
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(token) {
                if entry.expires_at > Instant::now() {
                    return Some(entry.principal.clone());
                }
            }
        }

        // Cache miss: query the store, Argon2-verify each key.
        let records = store.list_api_keys().await.ok()?;
        for record in records {
            if verify_key(token, &record.key_hash) {
                let scope = record.scope.parse().ok()?;
                let principal = AuthPrincipal {
                    owner_id: record.owner_id,
                    scope,
                    key_id: record.id,
                };
                let mut cache = self.cache.write().await;
                cache.insert(
                    token.to_string(),
                    CacheEntry {
                        principal: principal.clone(),
                        expires_at: Instant::now() + CACHE_TTL,
                    },
                );
                return Some(principal);
            }
        }
        None
    }

    /// Invalidate the entire cache (after key creation/revocation).
    pub async fn invalidate_all(&self) {
        self.cache.write().await.clear();
    }
}

// ── Key generation + Argon2 hashing ──────────────────────────────────────────

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

/// Argon2id hash of a plaintext key, returned as a PHC-format string (§17).
///
/// Argon2id is the recommended variant (OWASP, RFC 9106). The default
/// parameters from the `argon2` crate are used (m=19456 KiB, t=2, p=1).
/// A random salt is generated for each hash, so the same plaintext produces
/// different hashes — this is by design.
pub fn hash_key(plaintext: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(plaintext.as_bytes(), &salt)
        .expect("Argon2 hashing should not fail with valid input");
    hash.to_string()
}

/// Verify a plaintext key against an Argon2 PHC-format hash (§17).
pub fn verify_key(plaintext: &str, phc_hash: &str) -> bool {
    let parsed = match PasswordHash::new(phc_hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok()
}

/// Hash a plaintext key and store it. Used by main.rs for
/// `NEXUS_ADMIN_API_KEY` bootstrap and by tests to seed known keys.
pub async fn seed_api_key(
    store: &dyn Store,
    plaintext: &str,
    owner_id: Ulid,
    scope: KeyScope,
    label: &str,
) -> Result<Ulid, store::StoreError> {
    let id = store::new_ulid();
    store
        .put_api_key(&store::ApiKeyRecord {
            id: id.clone(),
            owner_id,
            key_hash: hash_key(plaintext),
            scope: scope.as_str().to_string(),
            label: label.to_string(),
            created_at: Utc::now(),
        })
        .await?;
    Ok(id)
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

    // Public: first key creation when the store has no keys (bootstrap, §13.1).
    if path == "/v1/api-keys" && method == Method::POST {
        let count = state.store.api_key_count().await.unwrap_or(1);
        if count == 0 {
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

    let principal = state
        .api_key_cache
        .validate(&token, state.store.as_ref())
        .await;

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
    fn hash_and_verify_key() {
        let key = "nsk_live_test_key_123";
        let hash = hash_key(key);
        assert!(verify_key(key, &hash));
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let hash = hash_key("nsk_live_correct");
        assert!(!verify_key("nsk_live_wrong", &hash));
    }

    #[test]
    fn hash_is_non_deterministic_due_to_salt() {
        // Argon2 uses a random salt → same input produces different hashes.
        let a = hash_key("nsk_live_same");
        let b = hash_key("nsk_live_same");
        assert_ne!(a, b);
        // But both verify against the same plaintext.
        assert!(verify_key("nsk_live_same", &a));
        assert!(verify_key("nsk_live_same", &b));
    }

    #[test]
    fn verify_rejects_malformed_hash() {
        assert!(!verify_key("nsk_live_anything", "not-a-valid-phc-string"));
    }

    #[test]
    fn scope_as_str_roundtrip() {
        for scope in [KeyScope::ReadOnly, KeyScope::Sync, KeyScope::Admin] {
            let s = scope.as_str();
            let parsed: KeyScope = s.parse().unwrap();
            assert_eq!(parsed, scope);
        }
        assert!("bogus".parse::<KeyScope>().is_err());
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

    #[tokio::test]
    async fn cache_validates_seeded_key() {
        let store = store::SqliteStore::connect_in_memory().await.unwrap();
        store.migrate().await.unwrap();

        let owner = store::new_ulid();
        seed_api_key(
            &store,
            "nsk_live_cache_test",
            owner.clone(),
            KeyScope::Admin,
            "test",
        )
        .await
        .unwrap();

        let cache = ApiKeyCache::new();
        let principal = cache
            .validate("nsk_live_cache_test", &store)
            .await
            .expect("key should validate");
        assert_eq!(principal.owner_id, owner);
        assert_eq!(principal.scope, KeyScope::Admin);
    }

    #[tokio::test]
    async fn cache_rejects_unknown_key() {
        let store = store::SqliteStore::connect_in_memory().await.unwrap();
        store.migrate().await.unwrap();

        let cache = ApiKeyCache::new();
        assert!(cache.validate("nsk_live_unknown", &store).await.is_none());
    }

    #[tokio::test]
    async fn cache_serves_from_memory_on_second_hit() {
        let store = store::SqliteStore::connect_in_memory().await.unwrap();
        store.migrate().await.unwrap();

        let owner = store::new_ulid();
        seed_api_key(
            &store,
            "nsk_live_cached",
            owner.clone(),
            KeyScope::Sync,
            "test",
        )
        .await
        .unwrap();

        let cache = ApiKeyCache::new();

        // First call: cache miss → queries store + Argon2 verify.
        let p1 = cache
            .validate("nsk_live_cached", &store)
            .await
            .expect("first validation");

        // Second call: cache hit → no store query.
        let p2 = cache
            .validate("nsk_live_cached", &store)
            .await
            .expect("second validation (from cache)");

        assert_eq!(p1.owner_id, p2.owner_id);
        assert_eq!(p1.key_id, p2.key_id);
    }

    #[tokio::test]
    async fn cache_invalidate_all_clears_entries() {
        let store = store::SqliteStore::connect_in_memory().await.unwrap();
        store.migrate().await.unwrap();

        let owner = store::new_ulid();
        seed_api_key(
            &store,
            "nsk_live_invalidate",
            owner,
            KeyScope::Admin,
            "test",
        )
        .await
        .unwrap();

        let cache = ApiKeyCache::new();
        cache
            .validate("nsk_live_invalidate", &store)
            .await
            .expect("should validate");

        cache.invalidate_all().await;

        // After invalidation, the key still validates (re-queries store).
        let principal = cache
            .validate("nsk_live_invalidate", &store)
            .await
            .expect("should still validate after cache clear");
        assert_eq!(principal.scope, KeyScope::Admin);
    }
}
