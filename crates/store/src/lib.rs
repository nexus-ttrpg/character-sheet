//! SQLx store — PostgreSQL (hosted) + SQLite (offline CLI) from one codebase
//! (§12, §14).
//!
//! One `Store` trait, two backends: `PgStore` (feature `postgres`) and
//! `SqliteStore` (feature `sqlite`). The CLI compiles with `sqlite`, the API
//! server with `postgres`; the test suite enables both. Query strings differ
//! slightly between backends (parameter placeholders, JSONB vs TEXT) but the
//! trait surface is identical, so callers are backend-agnostic.
//!
//! ## What the store owns (§12, §17)
//!
//! - The snapshot DAG: immutable `Snapshot` rows keyed by ULID, parent/merge-parent
//!   links, content-hash index for idempotent replay (§12).
//! - Character metadata + a `head_snapshot_id` pointer — current state is
//!   whatever HEAD points at (§11.1).
//! - Platform connections with AES-256-GCM-encrypted credentials (§17). Key
//!   material lives in `secrecy::Secret` wrappers and is never logged.
//! - ULID generation — the store assigns `id` and `created_at` (snapshot.rs:70).

#![forbid(unsafe_code)]

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
compile_error!("store: at least one of 'postgres' or 'sqlite' features must be enabled");

pub mod crypto;

#[cfg(feature = "postgres")]
pub mod pg;
#[cfg(feature = "postgres")]
pub use pg::PgStore;

#[cfg(feature = "sqlite")]
pub mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use schema_core::{
    AdapterId, CharacterBuild, CharacterMeta, ContentHash, PlatformId, SystemId, Ulid,
};
use sync_engine::{Snapshot, SnapshotOrigin};

pub use crypto::EncryptionKey;

// ── ULID generation ─────────────────────────────────────────────────────────

/// Generate a new ULID (§3.2). ULIDs are Crockford base32; string ordering
/// matches byte ordering, preserving the natural cursor property for
/// pagination (§18). This is the store's job per newtypes.rs.
pub fn new_ulid() -> Ulid {
    Ulid::new(ulid::Ulid::new().to_string())
}

/// Parse a ULID string, validating format. Returns `None` on malformed input.
pub fn parse_ulid(s: &str) -> Option<Ulid> {
    ulid::Ulid::from_string(s)
        .ok()
        .map(|u| Ulid::new(u.to_string()))
}

// ── Row types ───────────────────────────────────────────────────────────────

/// Character metadata as stored in the `characters` table — `CharacterMeta`
/// plus the `head_snapshot_id` pointer (§11.1).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterRow {
    pub id: Ulid,
    pub schema_version: semver::Version,
    pub system: SystemId,
    pub owner_id: Ulid,
    pub display_name: String,
    pub content_hash: ContentHash,
    pub head_snapshot_id: Option<Ulid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CharacterRow {
    /// Convert to a `CharacterMeta` (drops `head_snapshot_id`).
    pub fn into_meta(self) -> CharacterMeta {
        CharacterMeta {
            id: self.id,
            schema_version: self.schema_version,
            system: self.system,
            owner_id: self.owner_id,
            display_name: self.display_name,
            created_at: self.created_at,
            updated_at: self.updated_at,
            content_hash: self.content_hash,
        }
    }
}

/// A platform connection record with encrypted credentials (§17). The
/// `credentials` field is a `SecretString` — it is encrypted at rest by the
/// store and never logged via `Debug`.
#[derive(Debug, Clone)]
pub struct ConnectionRecord {
    pub id: Ulid,
    pub character_id: Ulid,
    pub platform: PlatformId,
    pub adapter_id: AdapterId,
    pub credentials: SecretString,
    pub last_sync_snapshot_id: Option<Ulid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A page of cursor-paginated results (§18). `next_cursor` is `Some` when more
/// rows may exist beyond the returned page; pass it as `cursor` on the next
/// call. ULID ordering is the natural stable cursor.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub rows: Vec<T>,
    pub next_cursor: Option<Ulid>,
}

// ── Store trait ─────────────────────────────────────────────────────────────

/// The persistence boundary for the snapshot DAG, characters, and connections
/// (§12).
///
/// One trait, two backends (`PgStore`, `SqliteStore`). Callers are
/// backend-agnostic — the API server uses `PgStore`, the CLI uses
/// `SqliteStore`, and tests exercise both against the same assertions.
///
/// ## Idempotency (§12)
///
/// `find_snapshot_by_hash` backs the content-addressed replay guarantee: a
/// sync op keyed by `content_hash` + parent snapshot is a no-op on replay
/// after a flaky reconnect, not a duplicate.
#[async_trait]
pub trait Store: Send + Sync {
    /// Run schema DDL (idempotent — safe to call on every startup).
    async fn migrate(&self) -> Result<(), StoreError>;

    // ── Character CRUD (§13.2) ──────────────────────────────────────────────

    /// Insert a new character. The `head_snapshot_id` is initially `None`;
    /// `set_head_snapshot` updates it after the first snapshot lands.
    async fn create_character(&self, meta: &CharacterMeta) -> Result<(), StoreError>;

    /// Get character metadata + head pointer. Returns `None` if not found.
    async fn get_character(&self, id: &Ulid) -> Result<Option<CharacterRow>, StoreError>;

    /// List characters for an owner, ULID-ordered cursor pagination (§18).
    /// `cursor` is exclusive — rows with `id > cursor` are returned.
    async fn list_characters(
        &self,
        owner_id: &Ulid,
        cursor: Option<&Ulid>,
        limit: u32,
    ) -> Result<Page<CharacterRow>, StoreError>;

    /// Delete a character and all associated snapshots/connections (cascade).
    async fn delete_character(&self, id: &Ulid) -> Result<(), StoreError>;

    /// Advance the head pointer to a new snapshot and update `content_hash` +
    /// `updated_at` (§11.1, §18 optimistic locking).
    async fn set_head_snapshot(
        &self,
        character_id: &Ulid,
        snapshot_id: &Ulid,
        content_hash: &ContentHash,
    ) -> Result<(), StoreError>;

    // ── Snapshot DAG (§11.1) ────────────────────────────────────────────────

    /// Insert an immutable snapshot. Idempotent: re-inserting the same `id`
    /// is a no-op (§12).
    async fn put_snapshot(&self, snapshot: &Snapshot) -> Result<(), StoreError>;

    /// Retrieve a snapshot by id. Returns `None` if not found.
    async fn get_snapshot(&self, id: &Ulid) -> Result<Option<Snapshot>, StoreError>;

    /// Get the snapshot the character's `head_snapshot_id` points at (§11.1).
    async fn get_head_snapshot(&self, character_id: &Ulid) -> Result<Option<Snapshot>, StoreError>;

    /// List a character's snapshot history, ULID-ordered (chronological).
    async fn list_history(
        &self,
        character_id: &Ulid,
        cursor: Option<&Ulid>,
        limit: u32,
    ) -> Result<Page<Snapshot>, StoreError>;

    /// Idempotency lookup (§12): find a snapshot with the given content hash
    /// and parent. Used to short-circuit replayed sync ops.
    async fn find_snapshot_by_hash(
        &self,
        character_id: &Ulid,
        content_hash: &ContentHash,
        parent: Option<&Ulid>,
    ) -> Result<Option<Snapshot>, StoreError>;

    // ── Connections (§17) ───────────────────────────────────────────────────

    /// Store a connection, encrypting credentials with AES-256-GCM (§17).
    async fn put_connection(
        &self,
        conn: &ConnectionRecord,
        key: &EncryptionKey,
    ) -> Result<(), StoreError>;

    /// Retrieve a connection, decrypting credentials (§17).
    async fn get_connection(
        &self,
        id: &Ulid,
        key: &EncryptionKey,
    ) -> Result<Option<ConnectionRecord>, StoreError>;

    /// List connections for a character (credentials decrypted).
    async fn list_connections(
        &self,
        character_id: &Ulid,
        key: &EncryptionKey,
    ) -> Result<Vec<ConnectionRecord>, StoreError>;

    /// Update a connection's `last_sync_snapshot_id` (§13.3).
    async fn update_connection_sync(
        &self,
        connection_id: &Ulid,
        snapshot_id: &Ulid,
    ) -> Result<(), StoreError>;
}

// ── Error ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("encryption error: {0}")]
    Crypto(#[from] crypto::CryptoError),
    #[error("row not found: {0}")]
    NotFound(String),
    #[error("semver parse error: {0}")]
    Semver(#[from] semver::Error),
    #[error("content hash decode error: {0}")]
    Hash(String),
    #[error("migration error: {0}")]
    Migration(#[from] schema_core::MigrationError),
}

// ── Shared serialization helpers ────────────────────────────────────────────

/// JSON-encode a `CharacterBuild` for storage. Both backends store `build` as
/// JSON text (PG uses JSONB, SQLite uses TEXT) — the canonicalization is
/// deterministic because `BTreeMap` ordering is enforced by `schema-core`
/// (§3.3, §9.2).
pub(crate) fn encode_build(build: &CharacterBuild) -> Result<String, StoreError> {
    Ok(serde_json::to_string(build)?)
}

/// Decode a `CharacterBuild` from a JSON `Value`, migrating it from the given
/// schema version to the current version if needed (§9.3 lazy on-read). The
/// store reads each snapshot's original `schema_version` column and passes it
/// here; if the version is older than `CURRENT_SCHEMA_VERSION`, the
/// `MigrationRegistry` chains registered migrations before deserialization.
pub(crate) fn decode_build_from_value(
    value: serde_json::Value,
    from: &semver::Version,
) -> Result<CharacterBuild, StoreError> {
    let registry = schema_core::MigrationRegistry::default();
    let migrated = registry.migrate_to_current(from, value)?;
    Ok(serde_json::from_value(migrated)?)
}

/// JSON-encode a `SnapshotOrigin` for storage.
pub(crate) fn encode_origin(origin: &SnapshotOrigin) -> Result<String, StoreError> {
    Ok(serde_json::to_string(origin)?)
}

/// Decode a `SnapshotOrigin` from stored JSON.
pub(crate) fn decode_origin(json: &str) -> Result<SnapshotOrigin, StoreError> {
    Ok(serde_json::from_str(json)?)
}

/// Decode a `ContentHash` from its hex string representation.
pub(crate) fn decode_hash(hex_str: &str) -> Result<ContentHash, StoreError> {
    let bytes = hex::decode(hex_str).map_err(|e| StoreError::Hash(e.to_string()))?;
    if bytes.len() != 32 {
        return Err(StoreError::Hash(format!(
            "content hash must be 32 bytes (64 hex chars), got {} bytes",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(ContentHash::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ulid_is_valid_and_unique() {
        let a = new_ulid();
        let b = new_ulid();
        assert_ne!(a, b);
        assert!(parse_ulid(a.as_str()).is_some());
    }

    #[test]
    fn parse_ulid_rejects_garbage() {
        assert!(parse_ulid("not-a-ulid").is_none());
        assert!(parse_ulid("").is_none());
    }

    #[test]
    fn ulid_string_ordering_matches_chronological() {
        // ULIDs encode a millisecond timestamp in the first 10 chars, so
        // string ordering matches creation order (§18).
        let a = new_ulid();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_ulid();
        assert!(
            a.as_str() < b.as_str(),
            "ULID string order should match chronological order"
        );
    }
}
