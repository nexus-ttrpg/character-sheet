//! Lazy migration on read (§9.3, Phase 3.2).
//!
//! Proves that a snapshot stored with an older `schema_version` and
//! old-format build JSON is automatically migrated when read back through the
//! store API. The v0.0.1 fixture has `identity.characterName` instead of
//! `identity.name`; without migration it fails to deserialize into the current
//! `CharacterBuild`.

use chrono::Utc;
use schema_core::{
    CharacterMeta, ContentHash, MigrationRegistry, SystemId, CURRENT_SCHEMA_VERSION,
};
use semver::Version;
use serde_json::Value;
use store::{SqliteStore, Store};
use sync_engine::SnapshotOrigin;

const V001_FIGHTER: &str =
    include_str!("../../../examples/fixtures/v0.0.1/pf2e-level1-fighter.json");
const V010_FIGHTER: &str = include_str!("../../../examples/fixtures/pf2e-level1-fighter.json");

#[tokio::test]
async fn lazy_migration_on_read_sqlite() {
    let store = SqliteStore::connect_in_memory()
        .await
        .expect("connect in-memory");
    store.migrate().await.expect("migrate");

    // Create a character to attach the snapshot to.
    let owner_id = store::new_ulid();
    let char_id = store::new_ulid();
    let now = Utc::now();
    let meta = CharacterMeta {
        id: char_id.clone(),
        schema_version: CURRENT_SCHEMA_VERSION.clone(),
        system: SystemId::new("pf2e"),
        owner_id: owner_id.clone(),
        display_name: "Aldric Ashford".into(),
        created_at: now,
        updated_at: now,
        content_hash: ContentHash::from_bytes([0u8; 32]),
    };
    store
        .create_character(&meta)
        .await
        .expect("create character");

    // Insert a v0.0.1 snapshot directly via SQL, bypassing put_snapshot (which
    // always writes CURRENT_SCHEMA_VERSION). This simulates a snapshot written
    // by a v0.0.1 binary that a self-hoster left in their DB.
    let snap_id = store::new_ulid();
    let created_by = store::new_ulid();
    let origin_json = serde_json::to_string(&SnapshotOrigin::ManualEdit).expect("serialize origin");

    // The content_hash of a v0.0.1 build equals that of the v0.1.0 build —
    // the migration is lossless, so the CharacterBuild struct is identical.
    let expected_build: schema_core::CharacterBuild =
        serde_json::from_str(V010_FIGHTER).expect("parse v0.1.0 fixture");
    let content_hash = schema_core::content_hash(&expected_build);

    sqlx::query(
        r#"INSERT INTO snapshots
           (id, character_id, parent_id, merge_parent_id, build,
            content_hash, origin, schema_version, created_at, created_by)
           VALUES (?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(snap_id.as_str())
    .bind(char_id.as_str())
    .bind(V001_FIGHTER)
    .bind(content_hash.to_hex())
    .bind(&origin_json)
    .bind("0.0.1")
    .bind(now)
    .bind(created_by.as_str())
    .execute(store.pool())
    .await
    .expect("insert v0.0.1 snapshot");

    // Read through the store API — map_snapshot should migrate the build.
    let snap = store
        .get_snapshot(&snap_id)
        .await
        .expect("get_snapshot")
        .expect("snapshot exists");

    // The build was migrated: identity.name exists (not characterName).
    assert_eq!(snap.build.identity.name, "Aldric Ashford");

    // The migrated build matches the v0.1.0 fixture exactly.
    assert_eq!(snap.build, expected_build);

    // The content hash of the migrated build matches the v0.1.0 fixture.
    assert_eq!(
        snap.content_hash, content_hash,
        "content hash should match v0.1.0 fixture after migration"
    );
}

#[tokio::test]
async fn current_version_snapshot_needs_no_migration_sqlite() {
    let store = SqliteStore::connect_in_memory()
        .await
        .expect("connect in-memory");
    store.migrate().await.expect("migrate");

    let owner_id = store::new_ulid();
    let char_id = store::new_ulid();
    let now = Utc::now();
    let meta = CharacterMeta {
        id: char_id.clone(),
        schema_version: CURRENT_SCHEMA_VERSION.clone(),
        system: SystemId::new("pf2e"),
        owner_id: owner_id.clone(),
        display_name: "Aldric Ashford".into(),
        created_at: now,
        updated_at: now,
        content_hash: ContentHash::from_bytes([0u8; 32]),
    };
    store
        .create_character(&meta)
        .await
        .expect("create character");

    // Insert a current-version snapshot directly via SQL.
    let snap_id = store::new_ulid();
    let created_by = store::new_ulid();
    let origin_json = serde_json::to_string(&SnapshotOrigin::ManualEdit).expect("serialize origin");

    sqlx::query(
        r#"INSERT INTO snapshots
           (id, character_id, parent_id, merge_parent_id, build,
            content_hash, origin, schema_version, created_at, created_by)
           VALUES (?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(snap_id.as_str())
    .bind(char_id.as_str())
    .bind(V010_FIGHTER)
    .bind(ContentHash::from_bytes([0u8; 32]).to_hex())
    .bind(&origin_json)
    .bind(CURRENT_SCHEMA_VERSION.to_string())
    .bind(now)
    .bind(created_by.as_str())
    .execute(store.pool())
    .await
    .expect("insert v0.1.0 snapshot");

    let snap = store
        .get_snapshot(&snap_id)
        .await
        .expect("get_snapshot")
        .expect("snapshot exists");

    assert_eq!(snap.build.identity.name, "Aldric Ashford");
    let expected: schema_core::CharacterBuild =
        serde_json::from_str(V010_FIGHTER).expect("parse v0.1.0 fixture");
    assert_eq!(snap.build, expected);
}

#[test]
fn v001_fixture_fails_deserialize_without_migration() {
    let result: Result<schema_core::CharacterBuild, _> = serde_json::from_str(V001_FIGHTER);
    assert!(
        result.is_err(),
        "v0.0.1 fixture must not deserialize without migration"
    );
}

#[test]
fn v001_fixture_migrates_through_registry() {
    let value: Value = serde_json::from_str(V001_FIGHTER).expect("parse v0.0.1 JSON");
    let registry = MigrationRegistry::default();
    let migrated = registry
        .migrate_to_current(&Version::new(0, 0, 1), value)
        .expect("migration");
    let build: schema_core::CharacterBuild =
        serde_json::from_value(migrated).expect("deserialize migrated");
    assert_eq!(build.identity.name, "Aldric Ashford");
}
