//! Errors from the adapter host (§5.5).

use thiserror::Error;

/// Errors produced by the adapter host — the boundary between untrusted TS
/// adapter code and the trusted Rust core (§5.5, §1).
#[derive(Debug, Error)]
pub enum HostError {
    /// Adapter output could not be deserialized into the canonical Rust types.
    /// This is the boundary check firing: the adapter returned something that
    /// does not match the schema (§5.5, §1).
    #[error("boundary validation failed: adapter output rejected — {0}")]
    BoundaryValidation(String),

    /// The adapter threw a JS exception or returned a rejected promise.
    #[error("adapter error: {0}")]
    Adapter(String),

    /// The adapter call exceeded the timeout. The runtime is dropped to contain
    /// the misbehaving adapter — one bad adapter degrades one connection, not
    /// the server (§21).
    #[error("adapter call timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// The adapter attempted an operation not granted by its manifest
    /// (§5.5 — capabilities are granted, not ambient).
    #[error("capability denied: {0}")]
    CapabilityDenied(String),

    /// The adapter module could not be loaded (syntax error, missing export,
    /// etc.).
    #[error("module load failed: {0}")]
    ModuleLoad(String),

    /// The external subprocess (JSON-RPC escape hatch, §5.5) failed.
    #[error("external runtime error: {0}")]
    External(String),

    /// A V8/deno_core runtime error not covered by the above.
    #[error("runtime error: {0}")]
    Runtime(String),

    /// I/O error from the external subprocess transport.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error in the host→adapter bridge.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

impl From<deno_core::error::CoreError> for HostError {
    fn from(e: deno_core::error::CoreError) -> Self {
        HostError::Runtime(e.to_string())
    }
}

impl From<deno_core::error::JsError> for HostError {
    fn from(e: deno_core::error::JsError) -> Self {
        HostError::Adapter(e.to_string())
    }
}

impl From<Box<deno_core::error::JsError>> for HostError {
    fn from(e: Box<deno_core::error::JsError>) -> Self {
        HostError::Adapter(e.to_string())
    }
}
