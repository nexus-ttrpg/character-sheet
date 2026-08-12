//! Adapter host — embeds deno_core to run TS adapters in-process, sandboxed,
//! with capability enforcement and boundary validation (§5.5).
//!
//! ## Architecture (§5.5)
//!
//! The host embeds a JavaScript runtime (`deno_core` — V8 plus a custom op
//! layer) inside the Rust process. Adapters are loaded as ES modules and
//! invoked through the `CharacterAdapter` interface (§5.1).
//!
//! **Capabilities are granted, not ambient.** An adapter gets no filesystem
//! access and no global `fetch`. Network access is a host op, scoped by an
//! allowlist in the adapter's manifest (§5.5).
//!
//! **Boundary validation.** Adapter output is deserialized and validated
//! against the canonical schema in Rust before the engine sees it (§5.5, §1).
//! Adapters are untrusted — the TS type checker is a convenience for the
//! adapter author, not a guarantee to the host.
//!
//! **Escape hatch.** An adapter may declare `"runtime": "external"` and ship
//! as a subprocess speaking JSON-RPC over stdio (§5.5).
//!
//! **Threading.** `JsRuntime` is `!Send` (V8 isolates are thread-local), so
//! `InProcessHost` must be used within a `tokio::task::LocalSet`.

pub mod error;
pub mod external;
pub mod host;
pub mod manifest;
pub mod ops;
pub mod outcome;
pub mod validation;

pub use error::HostError;
pub use external::ExternalAdapter;
pub use host::InProcessHost;
pub use manifest::{AdapterManifest, AdapterRuntime, CapabilityGrants, FetchCapabilities};
pub use outcome::{
    AdapterWarning, ChangeSet, ExportContext, ExportOutcome, ImportContext, ImportOutcome,
    PartialCharacterBuild, ValidationReport, WarningKind,
};

/// The unified adapter host — either in-process (V8 sandbox) or external
/// (subprocess JSON-RPC), selected by the manifest (§5.5).
pub enum AdapterHost {
    InProcess(InProcessHost),
    External(ExternalAdapter),
}

impl AdapterHost {
    /// Create a host from a manifest. The manifest's `runtime` field
    /// determines whether the adapter runs in-process or as a subprocess.
    pub async fn from_manifest(manifest: AdapterManifest) -> Result<Self, HostError> {
        match manifest.runtime {
            AdapterRuntime::InProcess => {
                let host = InProcessHost::new(manifest)?;
                Ok(Self::InProcess(host))
            }
            AdapterRuntime::External => {
                let adapter = ExternalAdapter::spawn(manifest).await?;
                Ok(Self::External(adapter))
            }
        }
    }

    /// Load the adapter module (in-process only). For external adapters,
    /// the subprocess is expected to be ready after `spawn`.
    pub async fn load(&mut self, code: &str) -> Result<(), HostError> {
        match self {
            Self::InProcess(h) => h.load(code).await,
            Self::External(_) => Ok(()),
        }
    }

    /// `adapter.import(raw, ctx)` — with boundary validation (§5.5).
    pub async fn import(
        &mut self,
        raw: serde_json::Value,
        ctx: ImportContext,
    ) -> Result<ImportOutcome, HostError> {
        match self {
            Self::InProcess(h) => h.import(raw, ctx).await,
            Self::External(h) => h.import(raw, ctx).await,
        }
    }

    /// `adapter.export(build, ctx)` — with boundary validation (§5.5).
    pub async fn export(
        &mut self,
        build: serde_json::Value,
        ctx: ExportContext,
    ) -> Result<ExportOutcome, HostError> {
        match self {
            Self::InProcess(h) => h.export(build, ctx).await,
            Self::External(h) => h.export(build, ctx).await,
        }
    }

    /// `adapter.detectChanges(previous, current)` (§5.3).
    pub async fn detect_changes(
        &mut self,
        previous: serde_json::Value,
        current: serde_json::Value,
    ) -> Result<ChangeSet, HostError> {
        match self {
            Self::InProcess(h) => h.detect_changes(previous, current).await,
            Self::External(h) => h.detect_changes(previous, current).await,
        }
    }

    /// `adapter.validate(raw)` (§5.1).
    pub async fn validate(
        &mut self,
        raw: serde_json::Value,
    ) -> Result<ValidationReport, HostError> {
        match self {
            Self::InProcess(h) => h.validate(raw).await,
            Self::External(h) => h.validate(raw).await,
        }
    }

    /// Returns the adapter ID from the manifest.
    pub fn adapter_id(&self) -> &str {
        match self {
            Self::InProcess(h) => h.adapter_id(),
            Self::External(h) => h.adapter_id(),
        }
    }
}
