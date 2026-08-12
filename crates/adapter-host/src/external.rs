//! External adapter runtime — subprocess JSON-RPC over stdio (§5.5).
//!
//! An adapter may declare `"runtime": "external"` and ship as a subprocess
//! implementing the identical `CharacterAdapter` interface via JSON-RPC. This
//! is the escape hatch for adapters that genuinely need full Node — it loses
//! in-process performance and forces the operator to have Node installed, but
//! means a platform requiring a Node-only library is never architecturally
//! blocked (§5.5, §21).
//!
//! The protocol is deliberately minimal:
//! - Host sends `{"id":N,"method":"import","params":[raw,ctx]}` on stdin.
//! - Adapter responds `{"id":N,"result":...}` or `{"id":N,"error":...}` on stdout.
//! - `notify` messages (no id) are used for logging.

use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::time::timeout;

use crate::error::HostError;
use crate::manifest::AdapterManifest;
use crate::outcome::{
    ChangeSet, ExportContext, ExportOutcome, ImportContext, ImportOutcome, ValidationReport,
};
use crate::validation;

/// Default timeout for external adapter calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// JSON-RPC request envelope.
#[derive(Debug, Serialize)]
struct Request<'a> {
    id: u64,
    method: &'a str,
    params: &'a [serde_json::Value],
}

/// JSON-RPC response envelope.
#[derive(Debug, Deserialize)]
struct Response {
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

/// JSON-RPC error.
#[derive(Debug, Deserialize)]
struct RpcError {
    code: i32,
    message: String,
}

/// External adapter runtime — subprocess speaking JSON-RPC over stdio (§5.5).
pub struct ExternalAdapter {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    manifest: AdapterManifest,
}

impl ExternalAdapter {
    /// Spawn the external adapter subprocess. The manifest's `entry` field is
    /// used as the command; it's split on whitespace into command + args.
    pub async fn spawn(manifest: AdapterManifest) -> Result<Self, HostError> {
        let mut parts = manifest.entry.split_whitespace();
        let cmd = parts
            .next()
            .ok_or_else(|| HostError::External("manifest entry is empty".into()))?;
        let args: Vec<&str> = parts.collect();

        let mut child = tokio::process::Command::new(cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| HostError::External(format!("failed to spawn adapter: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HostError::External("adapter stdin not captured".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HostError::External("adapter stdout not captured".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 0,
            manifest,
        })
    }

    /// Send a JSON-RPC request and wait for the response.
    async fn call(
        &mut self,
        method: &str,
        params: &[serde_json::Value],
    ) -> Result<serde_json::Value, HostError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = Request { id, method, params };
        let line = serde_json::to_string(&request)?;
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| HostError::External(format!("write to adapter stdin: {e}")))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|e| HostError::External(format!("write newline: {e}")))?;
        self.stdin.flush().await?;

        // Read response line.
        let mut buf = String::new();
        let read = timeout(DEFAULT_TIMEOUT, self.stdout.read_line(&mut buf))
            .await
            .map_err(|_| HostError::Timeout(DEFAULT_TIMEOUT))?
            .map_err(|e| HostError::External(format!("read from adapter stdout: {e}")))?;

        if read == 0 {
            return Err(HostError::External(
                "adapter process closed stdout unexpectedly".into(),
            ));
        }

        let response: Response = serde_json::from_str(&buf)?;

        if let Some(err) = response.error {
            return Err(HostError::Adapter(format!(
                "[code {}] {}",
                err.code, err.message
            )));
        }

        response
            .result
            .ok_or_else(|| HostError::External("response had neither result nor error".into()))
    }

    /// `adapter.import(raw, ctx)` via JSON-RPC.
    pub async fn import(
        &mut self,
        raw: serde_json::Value,
        ctx: ImportContext,
    ) -> Result<ImportOutcome, HostError> {
        let ctx_json = serde_json::to_value(&ctx)?;
        let result = self.call("import", &[raw, ctx_json]).await?;
        validation::validate_import(result)
    }

    /// `adapter.export(build, ctx)` via JSON-RPC.
    pub async fn export(
        &mut self,
        build: serde_json::Value,
        ctx: ExportContext,
    ) -> Result<ExportOutcome, HostError> {
        let ctx_json = serde_json::to_value(&ctx)?;
        let result = self.call("export", &[build, ctx_json]).await?;
        validation::validate_export(result)
    }

    /// `adapter.detectChanges(previous, current)` via JSON-RPC.
    pub async fn detect_changes(
        &mut self,
        previous: serde_json::Value,
        current: serde_json::Value,
    ) -> Result<ChangeSet, HostError> {
        let result = self.call("detectChanges", &[previous, current]).await?;
        validation::validate_changes(result)
    }

    /// `adapter.validate(raw)` via JSON-RPC.
    pub async fn validate(
        &mut self,
        raw: serde_json::Value,
    ) -> Result<ValidationReport, HostError> {
        let result = self.call("validate", &[raw]).await?;
        validation::validate_report(result)
    }

    /// Returns the manifest's adapter ID.
    pub fn adapter_id(&self) -> &str {
        &self.manifest.id
    }

    /// Returns whether the subprocess is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ExternalAdapter {
    fn drop(&mut self) {
        // Kill the subprocess on drop — kill_on_drop also handles this, but
        // we're explicit for clarity (§21 — contain misbehaving adapters).
        let _ = self.child.start_kill();
    }
}
