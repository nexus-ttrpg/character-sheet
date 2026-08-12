//! Round-trip tests for the store (Phase 0.6 gate, §19).
//!
//! SQLite runs unconditionally. Postgres runs when `DATABASE_URL` is set.
//! Both assert the same invariants:
//! - snapshot round-trips with a stable `content_hash`
//! - `get_head_snapshot` returns the current HEAD
//! - cursor pagination walks the history DAG correctly
//! - `find_snapshot_by_hash` backs idempotent replay (§12)
//! - connections encrypt/decrypt credentials (§17)
//! - `put_snapshot` is idempotent (§12)

use chrono::Utc;
use schema_core::{testing::minimal_build, CharacterMeta, ContentHash, SystemId, Ulid};
use secrecy::{ExposeSecret, SecretString};
use store::{ConnectionRecord, EncryptionKey, Page, Store};
use sync_engine::{Snapshot, SnapshotOrigin};

// ── Shared test logic ───────────────────────────────────────────────────────

fn make_character_meta(owner_id: &Ulid) -> CharacterMeta {
    let now = Utc::now();
    CharacterMeta {
        id: store::new_ulid(),
        schema_version: semver::Version::new(0, 1, 0),
        system: SystemId::new("pf2e"),
        owner_id: owner_id.clone(),
        display_name: "Test Character".into(),
        created_at: now,
        updated_at: now,
        content_hash: ContentHash::from_bytes([0u8; 32]),
    }
}

fn make_snapshot(character_id: &Ulid, parent: Option<Ulid>, created_by: &Ulid) -> Snapshot {
    Snapshot::new(
        store::new_ulid(),
        character_id.clone(),
        parent,
        minimal_build(),
        SnapshotOrigin::ManualEdit,
        Utc::now(),
        created_by.clone(),
    )
}

fn make_connection(character_id: &Ulid) -> ConnectionRecord {
    let now = Utc::now();
    ConnectionRecord {
        id: store::new_ulid(),
        character_id: character_id.clone(),
        platform: schema_core::PlatformId::new("foundry"),
        adapter_id: schema_core::AdapterId::new("foundry"),
        credentials: SecretString::new("foundry-token-deadbeef-cafe".into()),
        last_sync_snapshot_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// Run the full suite against any `Store`. This is the Phase 0 gate assertion
/// (§19): "store round-trips a snapshot with a stable hash."
async fn run_suite(store: &dyn Store) {
    let key = EncryptionKey::generate();
    let owner = store::new_ulid();
    let actor = store::new_ulid();

    store.migrate().await.expect("migrate");

    // ── Character CRUD ──────────────────────────────────────────────────────

    let meta = make_character_meta(&owner);
    store
        .create_character(&meta)
        .await
        .expect("create_character");

    let fetched = store
        .get_character(&meta.id)
        .await
        .expect("get_character")
        .expect("character exists");
    assert_eq!(fetched.id, meta.id);
    assert_eq!(fetched.display_name, meta.display_name);
    assert_eq!(fetched.head_snapshot_id, None);

    // ── Snapshot round-trip + stable content_hash ───────────────────────────

    let snap1 = make_snapshot(&meta.id, None, &actor);
    let original_hash = snap1.content_hash;

    store.put_snapshot(&snap1).await.expect("put_snapshot");
    store
        .set_head_snapshot(&meta.id, &snap1.id, &snap1.content_hash)
        .await
        .expect("set_head_snapshot");

    let retrieved = store
        .get_snapshot(&snap1.id)
        .await
        .expect("get_snapshot")
        .expect("snapshot exists");

    // The critical assertion: content_hash is stable across the round-trip.
    assert_eq!(
        retrieved.content_hash, original_hash,
        "content_hash changed across store round-trip"
    );
    assert_eq!(
        retrieved.build, snap1.build,
        "build changed across round-trip"
    );
    assert_eq!(retrieved.origin, snap1.origin);

    // HEAD points at the snapshot we just wrote.
    let head = store
        .get_head_snapshot(&meta.id)
        .await
        .expect("get_head_snapshot")
        .expect("head exists");
    assert_eq!(head.id, snap1.id);

    // ── Idempotent put (§12) ────────────────────────────────────────────────

    // Re-putting the same snapshot id is a no-op.
    store
        .put_snapshot(&snap1)
        .await
        .expect("idempotent put_snapshot");

    // find_snapshot_by_hash backs replay-idempotency (§12).
    let found = store
        .find_snapshot_by_hash(&meta.id, &snap1.content_hash, None)
        .await
        .expect("find_snapshot_by_hash")
        .expect("snapshot found by hash");
    assert_eq!(found.id, snap1.id);

    // ── History DAG + cursor pagination (§18) ───────────────────────────────

    // Create a chain of snapshots: snap1 → snap2 → snap3 → snap4 → snap5.
    let mut snapshots = vec![snap1];
    let mut parent = snapshots[0].id.clone();
    for _ in 0..4 {
        let snap = make_snapshot(&meta.id, Some(parent), &actor);
        store.put_snapshot(&snap).await.expect("put_snapshot chain");
        parent = snap.id.clone();
        snapshots.push(snap);
    }

    // Page through history with limit=2.
    let page1: Page<Snapshot> = store
        .list_history(&meta.id, None, 2)
        .await
        .expect("list_history page1");
    assert_eq!(page1.rows.len(), 2);
    assert!(page1.next_cursor.is_some(), "should have a next page");

    let page2 = store
        .list_history(&meta.id, page1.next_cursor.as_ref(), 2)
        .await
        .expect("list_history page2");
    assert_eq!(page2.rows.len(), 2);
    assert!(page2.next_cursor.is_some(), "should have a next page");

    let page3 = store
        .list_history(&meta.id, page2.next_cursor.as_ref(), 2)
        .await
        .expect("list_history page3");
    assert_eq!(page3.rows.len(), 1, "last page has 1 remaining");
    assert!(page3.next_cursor.is_none(), "no more pages");

    // Total history = 5 snapshots (snap1 + 4 chain).
    let all: Vec<Ulid> = [page1.rows, page2.rows, page3.rows]
        .concat()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(all.len(), 5, "paginated history should have 5 total");
    // ULID ordering = chronological (§18).
    assert!(all.windows(2).all(|w| w[0].as_str() < w[1].as_str()));

    // ── Connections: encrypt/decrypt (§17) ──────────────────────────────────

    let conn = make_connection(&meta.id);
    store
        .put_connection(&conn, &key)
        .await
        .expect("put_connection");

    let fetched_conn = store
        .get_connection(&conn.id, &key)
        .await
        .expect("get_connection")
        .expect("connection exists");
    assert_eq!(fetched_conn.platform, conn.platform);
    assert_eq!(fetched_conn.adapter_id, conn.adapter_id);
    assert_eq!(
        fetched_conn.credentials.expose_secret(),
        conn.credentials.expose_secret(),
        "credentials should decrypt to the same plaintext"
    );

    // list_connections returns the same connection.
    let conns = store
        .list_connections(&meta.id, &key)
        .await
        .expect("list_connections");
    assert_eq!(conns.len(), 1);
    assert_eq!(conns[0].id, conn.id);

    // update_connection_sync sets last_sync_snapshot_id.
    store
        .update_connection_sync(&conn.id, &snapshots[1].id)
        .await
        .expect("update_connection_sync");
    let updated = store
        .get_connection(&conn.id, &key)
        .await
        .expect("get_connection after sync")
        .expect("connection exists");
    assert_eq!(
        updated.last_sync_snapshot_id,
        Some(snapshots[1].id.clone()),
        "last_sync_snapshot_id should be updated"
    );

    // ── Cascade delete ──────────────────────────────────────────────────────

    store
        .delete_character(&meta.id)
        .await
        .expect("delete_character");
    assert!(
        store
            .get_character(&meta.id)
            .await
            .expect("get_character after delete")
            .is_none(),
        "character should be gone"
    );
    assert!(
        store
            .get_snapshot(&snapshots[0].id)
            .await
            .expect("get_snapshot after cascade")
            .is_none(),
        "snapshots should be cascade-deleted"
    );
    assert!(
        store
            .get_connection(&conn.id, &key)
            .await
            .expect("get_connection after cascade")
            .is_none(),
        "connections should be cascade-deleted"
    );
}

// ── SQLite ──────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_round_trip() {
    let store = store::SqliteStore::connect_in_memory()
        .await
        .expect("connect_in_memory");
    run_suite(&store).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_file_backed_round_trip() {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let url = format!("sqlite://{}", tmp.path().display());
    let store = store::SqliteStore::connect(&url)
        .await
        .expect("connect_file");
    run_suite(&store).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_list_characters_pagination() {
    let store = store::SqliteStore::connect_in_memory()
        .await
        .expect("connect");
    store.migrate().await.expect("migrate");
    let owner = store::new_ulid();

    // Create 3 characters.
    for i in 0..3 {
        let mut meta = make_character_meta(&owner);
        meta.display_name = format!("Character {i}");
        store.create_character(&meta).await.expect("create");
    }

    // Page with limit=2 → first page has 2, next_cursor is Some.
    let page1 = store
        .list_characters(&owner, None, 2)
        .await
        .expect("list_characters page1");
    assert_eq!(page1.rows.len(), 2);
    assert!(page1.next_cursor.is_some());

    // Second page → 1 remaining, no next.
    let page2 = store
        .list_characters(&owner, page1.next_cursor.as_ref(), 2)
        .await
        .expect("list_characters page2");
    assert_eq!(page2.rows.len(), 1);
    assert!(page2.next_cursor.is_none());
}

// ── Postgres (only when DATABASE_URL is set) ────────────────────────────────

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_round_trip() {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("skipping postgres_round_trip: DATABASE_URL not set");
            return;
        }
    };
    let store = store::PgStore::connect(&database_url)
        .await
        .expect("connect");

    // Clean slate — drop and recreate tables so the test is deterministic.
    sqlx::query("DROP TABLE IF EXISTS connections, snapshots, characters CASCADE")
        .execute(store.pool())
        .await
        .expect("drop tables");

    run_suite(&store).await;
}
