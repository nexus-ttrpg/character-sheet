//! Adapter worker — bridges the `!Send` adapter-host into Axum's async runtime.
//!
//! `deno_core`'s V8 isolates are `!Send` (§5.5, host.rs), so `InProcessHost`
//! cannot live across `.await` points on Tokio's multi-threaded scheduler.
//! The CLI solves this with a `LocalSet` on a current-thread runtime; the API
//! server needs the same, but invoked from an async handler.
//!
//! The pattern: pass only `Send` data (adapter code as `String`, manifest,
//! payload) into a `spawn_blocking` closure, create the `AdapterHost` *inside*
//! a fresh current-thread runtime + `LocalSet`, run the operation, and return
//! the `Send` outcome.

use std::path::{Path, PathBuf};

use adapter_host::{
    AdapterHost, AdapterManifest, ExportContext, ExportOutcome, HostError, ImportContext,
    ImportOutcome,
};
use schema_core::SystemId;

/// Read an adapter bundle from disk, returning the source code.
async fn read_adapter_bundle(adapters_dir: &Path, adapter_id: &str) -> Result<String, HostError> {
    let path = adapters_dir.join(adapter_id).join("bundle.js");
    let code = tokio::fs::read_to_string(&path)
        .await
        .map_err(HostError::Io)?;
    Ok(code)
}

/// Run `adapter.import(raw, ctx)` on a dedicated thread with a `LocalSet`.
///
/// The adapter code and manifest are read on the async runtime; the `!Send`
/// V8 isolate is created and used entirely inside the blocking closure.
pub async fn adapter_import(
    adapters_dir: PathBuf,
    adapter_id: String,
    raw: serde_json::Value,
    system: SystemId,
    credentials: serde_json::Value,
) -> Result<ImportOutcome, HostError> {
    let code = read_adapter_bundle(&adapters_dir, &adapter_id).await?;
    let manifest = AdapterManifest::new(adapter_id.clone(), "bundle.js");

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::Runtime(e.to_string()))?;
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            let mut host = AdapterHost::from_manifest(manifest).await?;
            host.load(&code).await?;
            let ctx = ImportContext {
                system,
                credentials,
            };
            host.import(raw, ctx).await
        })
    })
    .await
    .map_err(|e| HostError::Runtime(format!("adapter task panicked: {e}")))?
}

/// Run `adapter.export(build, ctx)` on a dedicated thread with a `LocalSet`.
pub async fn adapter_export(
    adapters_dir: PathBuf,
    adapter_id: String,
    build: serde_json::Value,
    system: SystemId,
    credentials: serde_json::Value,
) -> Result<ExportOutcome, HostError> {
    let code = read_adapter_bundle(&adapters_dir, &adapter_id).await?;
    let manifest = AdapterManifest::new(adapter_id.clone(), "bundle.js");

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| HostError::Runtime(e.to_string()))?;
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            let mut host = AdapterHost::from_manifest(manifest).await?;
            host.load(&code).await?;
            let ctx = ExportContext {
                system,
                credentials,
            };
            host.export(build, ctx).await
        })
    })
    .await
    .map_err(|e| HostError::Runtime(format!("adapter task panicked: {e}")))?
}
