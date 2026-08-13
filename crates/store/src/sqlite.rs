//! SQLite backend for the offline CLI (§12).
//!
//! `build` and `origin` are stored as JSON TEXT. ULIDs are stored as TEXT
//! strings — Crockford base32 ordering matches string ordering, so cursor
//! pagination works natively (§18). Credentials are AES-256-GCM encrypted
//! before storage (§17).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::ExposeSecret;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqliteRow};
use sqlx::Row as _;

use schema_core::{
    AdapterId, CharacterMeta, ContentHash, PlatformId, SystemId, Ulid, CURRENT_SCHEMA_VERSION,
};
use sync_engine::Snapshot;

use crate::crypto::EncryptionKey;
use crate::{
    decode_build_from_value, decode_hash, decode_origin, encode_build, encode_origin, CharacterRow,
    ConnectionRecord, Page, Store, StoreError,
};

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Connect with full control over options. Foreign keys are enabled so
    /// `ON DELETE CASCADE` works (§12).
    pub async fn connect_with(options: SqliteConnectOptions) -> Result<Self, StoreError> {
        let pool = SqlitePool::connect_with(options.foreign_keys(true)).await?;
        Ok(Self { pool })
    }

    /// Connect to a database by URL (e.g. `sqlite:///path/to/db.sqlite`).
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        Self::connect_with(SqliteConnectOptions::from_str(url)?).await
    }

    /// In-memory database for tests. Uses `cache=shared` so all pool
    /// connections see the same data.
    pub async fn connect_in_memory() -> Result<Self, StoreError> {
        Self::connect_with(
            SqliteConnectOptions::from_str("sqlite::memory:")?.create_if_missing(true),
        )
        .await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

// ── Row mapping ─────────────────────────────────────────────────────────────

fn map_character_row(row: SqliteRow) -> Result<CharacterRow, StoreError> {
    let id: String = row.try_get("id")?;
    let schema_version: String = row.try_get("schema_version")?;
    let system: String = row.try_get("system")?;
    let owner_id: String = row.try_get("owner_id")?;
    let display_name: String = row.try_get("display_name")?;
    let content_hash: String = row.try_get("content_hash")?;
    let head_snapshot_id: Option<String> = row.try_get("head_snapshot_id")?;
    let created_at = row.try_get("created_at")?;
    let updated_at = row.try_get("updated_at")?;
    Ok(CharacterRow {
        id: Ulid::new(id),
        schema_version: schema_version.parse()?,
        system: SystemId::new(system),
        owner_id: Ulid::new(owner_id),
        display_name,
        content_hash: decode_hash(&content_hash)?,
        head_snapshot_id: head_snapshot_id.map(Ulid::new),
        created_at,
        updated_at,
    })
}

fn map_snapshot(row: SqliteRow) -> Result<Snapshot, StoreError> {
    let id: String = row.try_get("id")?;
    let character_id: String = row.try_get("character_id")?;
    let parent_id: Option<String> = row.try_get("parent_id")?;
    let merge_parent_id: Option<String> = row.try_get("merge_parent_id")?;
    let build_json: String = row.try_get("build")?;
    let content_hash: String = row.try_get("content_hash")?;
    let origin_json: String = row.try_get("origin")?;
    let schema_version: String = row.try_get("schema_version")?;
    let created_at = row.try_get("created_at")?;
    let created_by: String = row.try_get("created_by")?;
    let build_value: serde_json::Value = serde_json::from_str(&build_json)?;
    Ok(Snapshot {
        id: Ulid::new(id),
        character_id: Ulid::new(character_id),
        parent: parent_id.map(Ulid::new),
        merge_parent: merge_parent_id.map(Ulid::new),
        build: decode_build_from_value(build_value, &schema_version.parse()?)?,
        content_hash: decode_hash(&content_hash)?,
        origin: decode_origin(&origin_json)?,
        created_at,
        created_by: Ulid::new(created_by),
    })
}

fn map_connection(row: SqliteRow, key: &EncryptionKey) -> Result<ConnectionRecord, StoreError> {
    let id: String = row.try_get("id")?;
    let character_id: String = row.try_get("character_id")?;
    let platform: String = row.try_get("platform")?;
    let adapter_id: String = row.try_get("adapter_id")?;
    let ciphertext: Vec<u8> = row.try_get("encrypted_credentials")?;
    let nonce_bytes: Vec<u8> = row.try_get("nonce")?;
    let last_sync_snapshot_id: Option<String> = row.try_get("last_sync_snapshot_id")?;
    let created_at = row.try_get("created_at")?;
    let updated_at = row.try_get("updated_at")?;
    let nonce: &[u8; 12] = nonce_bytes
        .as_slice()
        .try_into()
        .map_err(|_| StoreError::Hash("nonce must be 12 bytes".into()))?;
    let credentials = crate::crypto::decrypt(&ciphertext, nonce, key)?;
    Ok(ConnectionRecord {
        id: Ulid::new(id),
        character_id: Ulid::new(character_id),
        platform: PlatformId::new(platform),
        adapter_id: AdapterId::new(adapter_id),
        credentials,
        last_sync_snapshot_id: last_sync_snapshot_id.map(Ulid::new),
        created_at,
        updated_at,
    })
}

/// SQLite doesn't support `ADD COLUMN IF NOT EXISTS`. Try the ALTER and
/// ignore the "duplicate column name" error for idempotency (§9.3).
async fn add_column_if_not_exists(
    pool: &SqlitePool,
    table: &str,
    column_def: &str,
) -> Result<(), StoreError> {
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column_def}");
    match sqlx::query(&sql).execute(pool).await {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.message().contains("duplicate column name") => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ── Store impl ──────────────────────────────────────────────────────────────

#[async_trait]
impl Store for SqliteStore {
    async fn migrate(&self) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS characters (
                id               TEXT PRIMARY KEY,
                schema_version   TEXT NOT NULL,
                system           TEXT NOT NULL,
                owner_id         TEXT NOT NULL,
                display_name     TEXT NOT NULL,
                content_hash     TEXT NOT NULL,
                head_snapshot_id TEXT,
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_characters_owner
                ON characters(owner_id, id);

            CREATE TABLE IF NOT EXISTS snapshots (
                id               TEXT PRIMARY KEY,
                character_id     TEXT NOT NULL
                    REFERENCES characters(id) ON DELETE CASCADE,
                parent_id        TEXT,
                merge_parent_id  TEXT,
                build            TEXT NOT NULL,
                content_hash     TEXT NOT NULL,
                origin           TEXT NOT NULL,
                schema_version   TEXT NOT NULL DEFAULT '0.1.0',
                created_at       TEXT NOT NULL,
                created_by       TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_character
                ON snapshots(character_id, id);
            CREATE INDEX IF NOT EXISTS idx_snapshots_hash
                ON snapshots(character_id, content_hash, parent_id);

            CREATE TABLE IF NOT EXISTS connections (
                id                      TEXT PRIMARY KEY,
                character_id            TEXT NOT NULL
                    REFERENCES characters(id) ON DELETE CASCADE,
                platform                TEXT NOT NULL,
                adapter_id              TEXT NOT NULL,
                encrypted_credentials   BLOB NOT NULL,
                nonce                   BLOB NOT NULL,
                last_sync_snapshot_id   TEXT,
                created_at              TEXT NOT NULL,
                updated_at              TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_connections_character
                ON connections(character_id);
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Add schema_version to pre-existing snapshots tables (§9.3). New
        // tables get the column from CREATE TABLE above; this ALTER covers
        // databases created before the column existed.
        add_column_if_not_exists(
            &self.pool,
            "snapshots",
            "schema_version TEXT NOT NULL DEFAULT '0.1.0'",
        )
        .await?;

        Ok(())
    }

    // ── Character CRUD ──────────────────────────────────────────────────────

    async fn create_character(&self, meta: &CharacterMeta) -> Result<(), StoreError> {
        sqlx::query(
            r#"INSERT INTO characters
               (id, schema_version, system, owner_id, display_name, content_hash,
                head_snapshot_id, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)"#,
        )
        .bind(meta.id.as_str())
        .bind(meta.schema_version.to_string())
        .bind(meta.system.as_str())
        .bind(meta.owner_id.as_str())
        .bind(&meta.display_name)
        .bind(meta.content_hash.to_hex())
        .bind(meta.created_at)
        .bind(meta.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_character(&self, id: &Ulid) -> Result<Option<CharacterRow>, StoreError> {
        let row = sqlx::query(
            r#"SELECT id, schema_version, system, owner_id, display_name,
                      content_hash, head_snapshot_id, created_at, updated_at
               FROM characters WHERE id = ?"#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(map_character_row).transpose()
    }

    async fn list_characters(
        &self,
        owner_id: &Ulid,
        cursor: Option<&Ulid>,
        limit: u32,
    ) -> Result<Page<CharacterRow>, StoreError> {
        let limit = limit.min(100) as i64;
        let fetch_limit = limit + 1;

        let rows: Vec<SqliteRow> = match cursor {
            Some(c) => {
                sqlx::query(
                    r#"SELECT id, schema_version, system, owner_id, display_name,
                          content_hash, head_snapshot_id, created_at, updated_at
                   FROM characters
                   WHERE owner_id = ? AND id > ?
                   ORDER BY id ASC LIMIT ?"#,
                )
                .bind(owner_id.as_str())
                .bind(c.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    r#"SELECT id, schema_version, system, owner_id, display_name,
                          content_hash, head_snapshot_id, created_at, updated_at
                   FROM characters
                   WHERE owner_id = ?
                   ORDER BY id ASC LIMIT ?"#,
                )
                .bind(owner_id.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
        };

        let next_cursor = if (rows.len() as i64) > limit {
            Some(Ulid::new(
                rows[(limit - 1) as usize].try_get::<String, _>("id")?,
            ))
        } else {
            None
        };

        let character_rows: Vec<CharacterRow> = rows
            .into_iter()
            .take(limit as usize)
            .map(map_character_row)
            .collect::<Result<_, _>>()?;

        Ok(Page {
            rows: character_rows,
            next_cursor,
        })
    }

    async fn delete_character(&self, id: &Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM characters WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_head_snapshot(
        &self,
        character_id: &Ulid,
        snapshot_id: &Ulid,
        content_hash: &ContentHash,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"UPDATE characters
               SET head_snapshot_id = ?, content_hash = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(snapshot_id.as_str())
        .bind(content_hash.to_hex())
        .bind(Utc::now())
        .bind(character_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound(format!("character {character_id}")));
        }
        Ok(())
    }

    // ── Snapshot DAG ────────────────────────────────────────────────────────

    async fn put_snapshot(&self, snapshot: &Snapshot) -> Result<(), StoreError> {
        let build_json = encode_build(&snapshot.build)?;
        let origin_json = encode_origin(&snapshot.origin)?;
        // INSERT OR IGNORE — idempotent: re-inserting the same id is a no-op (§12).
        sqlx::query(
            r#"INSERT OR IGNORE INTO snapshots
               (id, character_id, parent_id, merge_parent_id, build,
                content_hash, origin, schema_version, created_at, created_by)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(snapshot.id.as_str())
        .bind(snapshot.character_id.as_str())
        .bind(snapshot.parent.as_ref().map(|u| u.as_str()))
        .bind(snapshot.merge_parent.as_ref().map(|u| u.as_str()))
        .bind(&build_json)
        .bind(snapshot.content_hash.to_hex())
        .bind(&origin_json)
        .bind(CURRENT_SCHEMA_VERSION.to_string())
        .bind(snapshot.created_at)
        .bind(snapshot.created_by.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_snapshot(&self, id: &Ulid) -> Result<Option<Snapshot>, StoreError> {
        let row = sqlx::query(
            r#"SELECT id, character_id, parent_id, merge_parent_id, build,
                      content_hash, origin, schema_version, created_at, created_by
               FROM snapshots WHERE id = ?"#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(map_snapshot).transpose()
    }

    async fn get_head_snapshot(&self, character_id: &Ulid) -> Result<Option<Snapshot>, StoreError> {
        let row = sqlx::query(
            r#"SELECT s.id, s.character_id, s.parent_id, s.merge_parent_id,
                      s.build, s.content_hash, s.origin, s.schema_version, s.created_at, s.created_by
               FROM snapshots s
               JOIN characters c ON c.head_snapshot_id = s.id
               WHERE c.id = ?"#,
        )
        .bind(character_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(map_snapshot).transpose()
    }

    async fn list_history(
        &self,
        character_id: &Ulid,
        cursor: Option<&Ulid>,
        limit: u32,
    ) -> Result<Page<Snapshot>, StoreError> {
        let limit = limit.min(100) as i64;
        let fetch_limit = limit + 1;

        let rows: Vec<SqliteRow> = match cursor {
            Some(c) => {
                sqlx::query(
                    r#"SELECT id, character_id, parent_id, merge_parent_id, build,
                          content_hash, origin, schema_version, created_at, created_by
                   FROM snapshots
                   WHERE character_id = ? AND id > ?
                   ORDER BY id ASC LIMIT ?"#,
                )
                .bind(character_id.as_str())
                .bind(c.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    r#"SELECT id, character_id, parent_id, merge_parent_id, build,
                          content_hash, origin, schema_version, created_at, created_by
                   FROM snapshots
                   WHERE character_id = ?
                   ORDER BY id ASC LIMIT ?"#,
                )
                .bind(character_id.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await?
            }
        };

        let next_cursor = if (rows.len() as i64) > limit {
            Some(Ulid::new(
                rows[(limit - 1) as usize].try_get::<String, _>("id")?,
            ))
        } else {
            None
        };

        let snapshots: Vec<Snapshot> = rows
            .into_iter()
            .take(limit as usize)
            .map(map_snapshot)
            .collect::<Result<_, _>>()?;

        Ok(Page {
            rows: snapshots,
            next_cursor,
        })
    }

    async fn find_snapshot_by_hash(
        &self,
        character_id: &Ulid,
        content_hash: &ContentHash,
        parent: Option<&Ulid>,
    ) -> Result<Option<Snapshot>, StoreError> {
        let row = match parent {
            Some(p) => {
                sqlx::query(
                    r#"SELECT id, character_id, parent_id, merge_parent_id, build,
                          content_hash, origin, schema_version, created_at, created_by
                   FROM snapshots
                   WHERE character_id = ? AND content_hash = ? AND parent_id = ?
                   LIMIT 1"#,
                )
                .bind(character_id.as_str())
                .bind(content_hash.to_hex())
                .bind(p.as_str())
                .fetch_optional(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    r#"SELECT id, character_id, parent_id, merge_parent_id, build,
                          content_hash, origin, schema_version, created_at, created_by
                   FROM snapshots
                   WHERE character_id = ? AND content_hash = ? AND parent_id IS NULL
                   LIMIT 1"#,
                )
                .bind(character_id.as_str())
                .bind(content_hash.to_hex())
                .fetch_optional(&self.pool)
                .await?
            }
        };
        row.map(map_snapshot).transpose()
    }

    // ── Connections ─────────────────────────────────────────────────────────

    async fn put_connection(
        &self,
        conn: &ConnectionRecord,
        key: &EncryptionKey,
    ) -> Result<(), StoreError> {
        let (ciphertext, nonce) = crate::crypto::encrypt(conn.credentials.expose_secret(), key)?;
        sqlx::query(
            r#"INSERT INTO connections
               (id, character_id, platform, adapter_id,
                encrypted_credentials, nonce, last_sync_snapshot_id,
                created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   platform = excluded.platform,
                   adapter_id = excluded.adapter_id,
                   encrypted_credentials = excluded.encrypted_credentials,
                   nonce = excluded.nonce,
                   updated_at = excluded.updated_at"#,
        )
        .bind(conn.id.as_str())
        .bind(conn.character_id.as_str())
        .bind(conn.platform.as_str())
        .bind(conn.adapter_id.as_str())
        .bind(&ciphertext)
        .bind(&nonce[..])
        .bind(conn.last_sync_snapshot_id.as_ref().map(|u| u.as_str()))
        .bind(conn.created_at)
        .bind(conn.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_connection(
        &self,
        id: &Ulid,
        key: &EncryptionKey,
    ) -> Result<Option<ConnectionRecord>, StoreError> {
        let row = sqlx::query(
            r#"SELECT id, character_id, platform, adapter_id,
                      encrypted_credentials, nonce, last_sync_snapshot_id,
                      created_at, updated_at
               FROM connections WHERE id = ?"#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| map_connection(r, key)).transpose()
    }

    async fn list_connections(
        &self,
        character_id: &Ulid,
        key: &EncryptionKey,
    ) -> Result<Vec<ConnectionRecord>, StoreError> {
        let rows = sqlx::query(
            r#"SELECT id, character_id, platform, adapter_id,
                      encrypted_credentials, nonce, last_sync_snapshot_id,
                      created_at, updated_at
               FROM connections WHERE character_id = ?
               ORDER BY id ASC"#,
        )
        .bind(character_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|r| map_connection(r, key)).collect()
    }

    async fn update_connection_sync(
        &self,
        connection_id: &Ulid,
        snapshot_id: &Ulid,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"UPDATE connections
               SET last_sync_snapshot_id = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(snapshot_id.as_str())
        .bind(Utc::now())
        .bind(connection_id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound(format!("connection {connection_id}")));
        }
        Ok(())
    }
}
