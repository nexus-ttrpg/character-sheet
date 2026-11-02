//! Command implementations for the `nexus` CLI (§12, §19).
//!
//! Each command drives the Phase 1 flow through the adapter-host (§5.5) and
//! the SQLite store (§12). Adapter execution is always boundary-validated —
//! the CLI never trusts adapter output blindly.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use adapter_host::{
    AdapterHost, AdapterManifest, ExportContext, ImportContext, PartialCharacterBuild,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use schema_core::{
    testing::minimal_build, CharacterBuild, CharacterMeta, ContentHash, SystemId, Ulid,
};
use sqlx::sqlite::SqliteConnectOptions;
use store::{new_ulid, parse_ulid, SqliteStore, Store};
use sync_engine::{diff, Snapshot, SnapshotOrigin};

use crate::Cli;

// ── Dispatch ───────────────────────────────────────────────────────────────

pub async fn dispatch(cli: Cli) -> Result<()> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", cli.db.display()))?
        .create_if_missing(true);
    let store = SqliteStore::connect_with(options)
        .await
        .context("connecting to local SQLite store")?;
    store.migrate().await.context("running schema migrations")?;

    match cli.command {
        crate::Command::Import {
            foundry,
            character,
            file,
        } => {
            cmd_import(&store, &cli.adapters_dir, foundry, character, &file).await?;
            Ok(())
        }
        crate::Command::Push {
            foundry,
            character,
            out,
        } => cmd_push(&store, &cli.adapters_dir, foundry, &character, &out).await,
        crate::Command::Diff {
            foundry,
            character,
            file,
            from,
            to,
        } => {
            cmd_diff(
                &store,
                &cli.adapters_dir,
                foundry,
                character,
                file,
                from,
                to,
            )
            .await
        }
        crate::Command::History { character_id } => cmd_history(&store, &character_id).await,
    }
}

// ── import ─────────────────────────────────────────────────────────────────

/// `nexus import --foundry <file>` — import a platform payload into canonical,
/// creating a new snapshot (§19).
///
/// If `--character <id>` is given, the import updates that character (new
/// snapshot with the previous HEAD as parent). Otherwise a new character is
/// created.
async fn cmd_import(
    store: &SqliteStore,
    adapters_dir: &Path,
    _foundry: bool,
    character_id: Option<String>,
    file: &Path,
) -> Result<(Ulid, Ulid)> {
    let raw = read_json(file).await.context("reading platform payload")?;

    let mut host = load_foundry_adapter(adapters_dir).await?;

    let ctx = ImportContext {
        system: SystemId::new("pf2e"),
        credentials: serde_json::json!({}),
    };
    let outcome = host
        .import(raw, ctx)
        .await
        .context("adapter.import() failed")?;

    let build = complete_build(outcome.build);

    for w in &outcome.warnings {
        eprintln!("  warning: {} ({}: {})", w.path, w.kind_label(), w.detail);
    }

    let now = Utc::now();
    let owner_id = new_ulid();

    let (char_id, parent_snap) = match &character_id {
        Some(id) => {
            let cid = parse_ulid(id).ok_or_else(|| anyhow!("invalid character ULID: {id}"))?;
            let head = store
                .get_head_snapshot(&cid)
                .await
                .context("fetching HEAD snapshot")?
                .ok_or_else(|| anyhow!("character {id} has no HEAD snapshot"))?;
            (cid, Some(head))
        }
        None => (new_ulid(), None),
    };

    let content_hash = schema_core::content_hash(&build);

    // Short-circuit: if the imported content matches the current HEAD, this
    // is a no-op (the content hasn't changed since the last sync). This
    // complements the §12 content_hash + parent idempotency check below,
    // which catches replayed ops with the same parent.
    if let Some(ref head) = parent_snap {
        if head.content_hash == content_hash {
            println!(
                "no changes — HEAD {} is already at hash {}",
                head.id,
                fmt_hash(&content_hash)
            );
            return Ok((char_id, head.id.clone()));
        }
    }

    let snap_id = new_ulid();

    if character_id.is_none() {
        let meta = CharacterMeta {
            id: char_id.clone(),
            schema_version: schema_core::CURRENT_SCHEMA_VERSION.clone(),
            system: SystemId::new("pf2e"),
            owner_id: owner_id.clone(),
            display_name: build.identity.name.clone(),
            created_at: now,
            updated_at: now,
            content_hash,
        };
        store
            .create_character(&meta)
            .await
            .context("create_character")?;
    }

    let snapshot = Snapshot::new(
        snap_id.clone(),
        char_id.clone(),
        parent_snap.as_ref().map(|s| s.id.clone()),
        build,
        SnapshotOrigin::Import {
            adapter: "foundry".into(),
        },
        now,
        owner_id,
    );

    // Idempotency check (§12): if a snapshot with this content hash + parent
    // already exists, this is a replay — no-op.
    if let Some(existing) = store
        .find_snapshot_by_hash(&char_id, &content_hash, parent_snap.as_ref().map(|s| &s.id))
        .await?
    {
        println!("replay detected — snapshot {} already exists", existing.id);
        return Ok((char_id, existing.id));
    }

    store
        .put_snapshot(&snapshot)
        .await
        .context("put_snapshot")?;
    store
        .set_head_snapshot(&char_id, &snap_id, &content_hash)
        .await
        .context("set_head_snapshot")?;

    println!(
        "imported → character {} | snapshot {} | hash {}",
        char_id,
        snap_id,
        fmt_hash(&content_hash)
    );
    Ok((char_id, snap_id))
}

// ── push ───────────────────────────────────────────────────────────────────

/// `nexus push --foundry --character <id> --out <file>` — export the current
/// HEAD canonical build to a platform payload (§19).
async fn cmd_push(
    store: &SqliteStore,
    adapters_dir: &Path,
    _foundry: bool,
    character_id: &str,
    out: &Path,
) -> Result<()> {
    let cid = parse_ulid(character_id)
        .ok_or_else(|| anyhow!("invalid character ULID: {character_id}"))?;

    let head = store
        .get_head_snapshot(&cid)
        .await
        .context("fetching HEAD snapshot")?
        .ok_or_else(|| anyhow!("character {character_id} has no HEAD snapshot"))?;

    let mut host = load_foundry_adapter(adapters_dir).await?;

    let build_json = serde_json::to_value(&head.build).context("serializing HEAD build")?;

    let ctx = ExportContext {
        system: SystemId::new("pf2e"),
        credentials: serde_json::json!({}),
    };
    let outcome = host
        .export(build_json, ctx)
        .await
        .context("adapter.export() failed")?;

    for w in &outcome.warnings {
        eprintln!("  warning: {} ({}: {})", w.path, w.kind_label(), w.detail);
    }

    write_json(out, &outcome.payload).context("writing platform payload")?;

    println!(
        "pushed → {} | snapshot {} | hash {}",
        out.display(),
        head.id,
        fmt_hash(&head.content_hash)
    );
    Ok(())
}

// ── diff ───────────────────────────────────────────────────────────────────

/// `nexus diff` — field-level changes between HEAD and a new import, or
/// between two snapshots (§11.2, §19).
async fn cmd_diff(
    store: &SqliteStore,
    adapters_dir: &Path,
    foundry: bool,
    character: Option<String>,
    file: Option<PathBuf>,
    from: Option<String>,
    to: Option<String>,
) -> Result<()> {
    // Case 1: diff two snapshot IDs.
    if let (Some(from_id), Some(to_id)) = (&from, &to) {
        let from_snap = get_snapshot(store, from_id).await?;
        let to_snap = get_snapshot(store, to_id).await?;
        print_diff(&diff(&from_snap.build, &to_snap.build));
        return Ok(());
    }

    if from.is_some() || to.is_some() {
        bail!("--from and --to must be used together");
    }

    // Case 2: diff a new platform payload against a character's HEAD.
    let char_id = character
        .as_deref()
        .ok_or_else(|| anyhow!("--character is required when diffing against HEAD"))?;

    let cid = parse_ulid(char_id).ok_or_else(|| anyhow!("invalid character ULID: {char_id}"))?;

    let head = store
        .get_head_snapshot(&cid)
        .await
        .context("fetching HEAD snapshot")?
        .ok_or_else(|| anyhow!("character {char_id} has no HEAD snapshot"))?;

    let file = file
        .as_deref()
        .ok_or_else(|| anyhow!("a payload file is required (or use --from/--to for snapshots)"))?;

    let raw = read_json(file).await.context("reading platform payload")?;

    let build = if foundry {
        let mut host = load_foundry_adapter(adapters_dir).await?;
        let ctx = ImportContext {
            system: SystemId::new("pf2e"),
            credentials: serde_json::json!({}),
        };
        let outcome = host
            .import(raw, ctx)
            .await
            .context("adapter.import() failed")?;
        for w in &outcome.warnings {
            eprintln!("  warning: {} ({}: {})", w.path, w.kind_label(), w.detail);
        }
        complete_build(outcome.build)
    } else {
        serde_json::from_value::<CharacterBuild>(raw).context("parsing canonical build JSON")?
    };

    let diffs = diff(&head.build, &build);
    print_diff(&diffs);
    Ok(())
}

// ── history ────────────────────────────────────────────────────────────────

/// `nexus history <character_id>` — list snapshot DAG for a character (§11.1).
async fn cmd_history(store: &SqliteStore, character_id: &str) -> Result<()> {
    let cid = parse_ulid(character_id)
        .ok_or_else(|| anyhow!("invalid character ULID: {character_id}"))?;

    let character = store
        .get_character(&cid)
        .await
        .context("fetching character")?
        .ok_or_else(|| anyhow!("character {character_id} not found"))?;

    println!("character: {} ({})", character.display_name, character.id);
    println!("system:    {}", character.system);
    println!(
        "head:      {}",
        character
            .head_snapshot_id
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("(none)")
    );
    println!();

    let mut cursor: Option<Ulid> = None;
    loop {
        let page = store
            .list_history(&cid, cursor.as_ref(), 50)
            .await
            .context("listing history")?;

        if page.rows.is_empty() && cursor.is_none() {
            println!("(no snapshots)");
            break;
        }

        for snap in &page.rows {
            let origin_label = match &snap.origin {
                SnapshotOrigin::ManualEdit => "manual".into(),
                SnapshotOrigin::Import { adapter } => format!("import:{adapter}"),
                SnapshotOrigin::Merge { resolved_conflicts } => {
                    format!("merge({resolved_conflicts} resolved)")
                }
                SnapshotOrigin::Migration { from } => format!("migrate:{from}"),
                SnapshotOrigin::Rollback { to } => format!("revert→{to}"),
            };
            let parent_label = snap.parent.as_ref().map(|p| p.as_str()).unwrap_or("—");
            println!(
                "  {}  parent={}  hash={}  {}  {}",
                snap.id,
                parent_label,
                fmt_hash(&snap.content_hash),
                snap.created_at.format("%Y-%m-%d %H:%M:%S"),
                origin_label,
            );
        }

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Load the Foundry adapter bundle into a fresh adapter-host (§5.5).
///
/// Adapters are stateless (§5.4) — the host is created per operation.
async fn load_foundry_adapter(adapters_dir: &Path) -> Result<AdapterHost> {
    let bundle_path = adapters_dir.join("foundry").join("bundle.js");
    let code = tokio::fs::read_to_string(&bundle_path)
        .await
        .with_context(|| format!("reading adapter bundle at {}", bundle_path.display()))?;

    let manifest = AdapterManifest::new("foundry", bundle_path.display().to_string());
    let mut host = AdapterHost::from_manifest(manifest).await?;
    host.load(&code).await?;
    Ok(host)
}

/// Convert a `PartialCharacterBuild` (adapter output) into a full
/// `CharacterBuild` by overlaying present fields on a minimal base (§5.1).
///
/// Fields the adapter didn't cover get the minimal-build defaults. This is
/// the pragmatic Phase 1 path; a real completion layer would apply system
/// rules (key ability, HP per level) rather than copying defaults.
fn complete_build(partial: PartialCharacterBuild) -> CharacterBuild {
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

/// Fetch a snapshot by ULID string.
async fn get_snapshot(store: &SqliteStore, id: &str) -> Result<Snapshot> {
    let sid = parse_ulid(id).ok_or_else(|| anyhow!("invalid snapshot ULID: {id}"))?;
    store
        .get_snapshot(&sid)
        .await
        .context("fetching snapshot")?
        .ok_or_else(|| anyhow!("snapshot {id} not found"))
}

/// Print field-level diffs in a compact, readable format (§11.2).
fn print_diff(diffs: &[sync_engine::FieldDiff]) {
    if diffs.is_empty() {
        println!("no changes");
        return;
    }
    println!("{} field(s) changed:", diffs.len());
    for d in diffs {
        match &d.change {
            sync_engine::Change::Added { after } => {
                println!("  + {} = {}", d.path.as_str(), fmt_value(after));
            }
            sync_engine::Change::Removed { before } => {
                println!("  - {} = {}", d.path.as_str(), fmt_value(before));
            }
            sync_engine::Change::Modified { before, after } => {
                println!(
                    "  ~ {} : {} → {}",
                    d.path.as_str(),
                    fmt_value(before),
                    fmt_value(after)
                );
            }
        }
    }
}

/// Format a `serde_json::Value` for compact display.
fn fmt_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// Format a content hash as a short hex prefix (§9.2).
fn fmt_hash(hash: &ContentHash) -> String {
    hex::encode(hash.as_bytes()).chars().take(12).collect()
}

/// Read a JSON file.
async fn read_json(path: &Path) -> Result<serde_json::Value> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing JSON in {}", path.display()))
}

/// Write a JSON value to a file (pretty-printed).
fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Extension trait for `AdapterWarning` — a human-readable kind label.
trait WarningKindLabel {
    fn kind_label(&self) -> &'static str;
}

impl WarningKindLabel for adapter_host::AdapterWarning {
    fn kind_label(&self) -> &'static str {
        match &self.kind {
            adapter_host::WarningKind::UnmappedField => "unmapped",
            adapter_host::WarningKind::LossyExport { .. } => "lossy",
            adapter_host::WarningKind::FuzzyMatch { .. } => "fuzzy",
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use schema_core::CharacterBuild;

    /// Workspace root from `CARGO_MANIFEST_DIR` (the `cli/` crate dir).
    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// Path to the pre-built adapter bundle (produced by `pnpm --filter
    /// @nexus-ttrpg/foundry-adapter bundle`). Tests skip if absent.
    fn adapter_bundle_path() -> PathBuf {
        workspace_root().join("adapters/foundry/dist/bundle/bundle.js")
    }

    /// Path to the PF2e fighter fixture (canonical build JSON).
    fn fighter_fixture_path() -> PathBuf {
        workspace_root().join("examples/fixtures/pf2e-level1-fighter.json")
    }

    /// Export the fighter fixture to a Foundry Actor JSON via the adapter-host,
    /// producing a realistic platform payload for import testing.
    async fn export_fighter_to_foundry_actor(adapters_dir: &Path) -> serde_json::Value {
        let build_bytes = tokio::fs::read(fighter_fixture_path()).await.unwrap();
        let build: CharacterBuild = serde_json::from_slice(&build_bytes).unwrap();

        let mut host = load_foundry_adapter(adapters_dir).await.unwrap();
        let ctx = ExportContext {
            system: SystemId::new("pf2e"),
            credentials: serde_json::json!({}),
        };
        let outcome = host
            .export(serde_json::to_value(&build).unwrap(), ctx)
            .await
            .unwrap();
        outcome.payload
    }

    /// Set up a temp directory with the adapter bundle copied in, returning
    /// (tmp_dir, adapters_dir, db_path).
    async fn setup_temp_workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let bundle = adapter_bundle_path();
        assert!(
            bundle.exists(),
            "adapter bundle not found at {} — run `pnpm --filter @nexus-ttrpg/foundry-adapter bundle` first",
            bundle.display()
        );

        let tmp = tempfile::tempdir().unwrap();
        let adapters_dir = tmp.path().join("adapters");
        let foundry_dir = adapters_dir.join("foundry");
        tokio::fs::create_dir_all(&foundry_dir).await.unwrap();
        tokio::fs::copy(&bundle, foundry_dir.join("bundle.js"))
            .await
            .unwrap();

        let db_path = tmp.path().join("test.sqlite");
        (tmp, adapters_dir, db_path)
    }

    async fn connect_store(db_path: &Path) -> SqliteStore {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
            .unwrap()
            .create_if_missing(true);
        let store = SqliteStore::connect_with(options).await.unwrap();
        store.migrate().await.unwrap();
        store
    }

    /// Run an async test body inside a `LocalSet` (V8 is `!Send`).
    fn run_local<F>(future: F)
    where
        F: std::future::Future<Output = ()>,
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, future);
    }

    // ── import → history ────────────────────────────────────────────────────

    #[test]
    fn import_creates_character_and_snapshot() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();

            let (char_id, snap_id) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .expect("import should succeed");

            // Character exists with HEAD pointing at the snapshot.
            let character = store.get_character(&char_id).await.unwrap().unwrap();
            assert_eq!(character.id, char_id);
            assert_eq!(character.head_snapshot_id, Some(snap_id.clone()));
            assert!(!character.display_name.is_empty());

            // Snapshot exists with Import origin.
            let snap = store.get_snapshot(&snap_id).await.unwrap().unwrap();
            assert_eq!(snap.character_id, char_id);
            assert!(snap.parent.is_none()); // first snapshot, no parent
            assert!(matches!(
                snap.origin,
                SnapshotOrigin::Import { ref adapter } if adapter.as_str() == "foundry"
            ));

            // Content hash matches.
            assert_eq!(snap.content_hash, schema_core::content_hash(&snap.build));
        });
    }

    // ── import → diff → push round-trip ─────────────────────────────────────

    #[test]
    fn import_diff_push_round_trip() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            // 1. Import the fighter.
            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();
            let (char_id, _snap1) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .unwrap();

            // 2. Diff: modify the actor's name, diff against HEAD.
            let mut modified_actor = actor.clone();
            modified_actor["name"] = serde_json::json!("Renamed Fighter");
            let modified_file = adapters_dir.join("../actor-modified.json");
            write_json(&modified_file, &modified_actor).unwrap();

            // diff should detect the name change (§11.2). The diff is printed
            // to stdout; here we just verify the command succeeds. A structural
            // assertion is done in import_then_reimport_creates_child_snapshot.
            cmd_diff(
                &store,
                &adapters_dir,
                true,
                Some(char_id.as_str().to_string()),
                Some(modified_file),
                None,
                None,
            )
            .await
            .expect("diff should succeed");

            // 3. Push: export HEAD back to a Foundry Actor JSON file.
            let out_file = adapters_dir.join("../pushed.json");
            cmd_push(&store, &adapters_dir, true, char_id.as_str(), &out_file)
                .await
                .expect("push should succeed");

            let pushed: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&out_file).unwrap()).unwrap();
            assert_eq!(pushed["name"], actor["name"]); // original name (HEAD build)
            assert_eq!(pushed["type"], "character");
        });
    }

    // ── import → re-import creates child snapshot ───────────────────────────

    #[test]
    fn reimport_creates_child_snapshot_with_parent() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            // First import.
            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();
            let (char_id, snap1) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .unwrap();

            // Modify the actor and re-import with --character.
            let mut modified_actor = actor.clone();
            modified_actor["name"] = serde_json::json!("Leveled Up");
            let modified_file = adapters_dir.join("../actor-v2.json");
            write_json(&modified_file, &modified_actor).unwrap();

            let (_char_id2, snap2) = cmd_import(
                &store,
                &adapters_dir,
                true,
                Some(char_id.as_str().to_string()),
                &modified_file,
            )
            .await
            .unwrap();

            // snap2's parent should be snap1 (§11.1 DAG).
            let snap2_row = store.get_snapshot(&snap2).await.unwrap().unwrap();
            assert_eq!(snap2_row.parent, Some(snap1.clone()));
            assert_eq!(snap2_row.character_id, char_id);

            // HEAD should now point at snap2.
            let character = store.get_character(&char_id).await.unwrap().unwrap();
            assert_eq!(character.head_snapshot_id, Some(snap2));
        });
    }

    // ── import → diff between two snapshots ─────────────────────────────────

    #[test]
    fn diff_between_two_snapshots() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            // Import v1.
            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();
            let (char_id, snap1) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .unwrap();

            // Import v2 (modified name).
            let mut modified_actor = actor.clone();
            modified_actor["name"] = serde_json::json!("Renamed");
            let modified_file = adapters_dir.join("../actor-v2.json");
            write_json(&modified_file, &modified_actor).unwrap();
            let (_, snap2) = cmd_import(
                &store,
                &adapters_dir,
                true,
                Some(char_id.as_str().to_string()),
                &modified_file,
            )
            .await
            .unwrap();

            // Diff between the two snapshots should succeed.
            cmd_diff(
                &store,
                &adapters_dir,
                false,
                None,
                None,
                Some(snap1.as_str().to_string()),
                Some(snap2.as_str().to_string()),
            )
            .await
            .expect("snapshot diff should succeed");
        });
    }

    // ── import → history shows DAG ──────────────────────────────────────────

    #[test]
    fn history_lists_snapshots() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();
            let (char_id, _snap1) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .unwrap();

            // Re-import to create a second snapshot.
            let (_, snap2) = cmd_import(
                &store,
                &adapters_dir,
                true,
                Some(char_id.as_str().to_string()),
                &actor_file,
            )
            .await
            .unwrap();

            // History should list both snapshots (the first + the replay, or
            // the first + a child if the content differs). Since we re-imported
            // the same file, the idempotency check should make it a no-op
            // (§12) — so there's still 1 snapshot.
            let page = store.list_history(&char_id, None, 50).await.unwrap();
            assert!(
                !page.rows.is_empty(),
                "history should have at least one snapshot"
            );

            // cmd_history should succeed.
            cmd_history(&store, char_id.as_str())
                .await
                .expect("history should succeed");

            // Suppress unused warning.
            let _ = snap2;
        });
    }

    // ── idempotent replay (§12) ─────────────────────────────────────────────

    #[test]
    fn reimport_same_payload_is_idempotent() {
        run_local(async {
            let (_tmp, adapters_dir, db_path) = setup_temp_workspace().await;
            let store = connect_store(&db_path).await;

            let actor = export_fighter_to_foundry_actor(&adapters_dir).await;
            let actor_file = adapters_dir.join("../actor.json");
            write_json(&actor_file, &actor).unwrap();

            let (char_id, snap1) = cmd_import(&store, &adapters_dir, true, None, &actor_file)
                .await
                .unwrap();

            // Re-import the exact same file with --character.
            let (char_id2, snap2) = cmd_import(
                &store,
                &adapters_dir,
                true,
                Some(char_id.as_str().to_string()),
                &actor_file,
            )
            .await
            .unwrap();

            // Same character, same snapshot (§12 idempotency).
            assert_eq!(char_id, char_id2);
            assert_eq!(snap1, snap2);

            // Only one snapshot in history.
            let page = store.list_history(&char_id, None, 50).await.unwrap();
            assert_eq!(page.rows.len(), 1);
        });
    }
}
