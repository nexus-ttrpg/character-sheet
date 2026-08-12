//! In-process adapter host — embeds `deno_core` (V8 + tiny op layer) to run
//! TS adapters sandboxed, with boundary validation (§5.5).
//!
//! `JsRuntime` is `!Send` (V8 isolates are thread-local), so `InProcessHost`
//! must be used within a `tokio::task::LocalSet`. The host is created per sync
//! operation — adapters are stateless (§5.4), so there's no need to keep a
//! runtime alive between calls.

use std::time::Duration;

use deno_core::v8;
use deno_core::JsRuntime;
use deno_core::PollEventLoopOptions;
use deno_core::RuntimeOptions;
use tokio::time::timeout;

use crate::error::HostError;
use crate::manifest::{AdapterManifest, FetchCapabilities};
use crate::ops;
use crate::outcome::{
    ChangeSet, ExportContext, ExportOutcome, ImportContext, ImportOutcome, ValidationReport,
};
use crate::validation;

/// Default timeout for adapter calls (§21 — contain panicking/hanging adapters).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// In-process adapter host — V8 runtime with a tiny op layer (§5.5).
///
/// Created per sync operation (adapters are stateless, §5.4). Must be used
/// within a `tokio::task::LocalSet` because `JsRuntime` is `!Send`.
pub struct InProcessHost {
    runtime: JsRuntime,
    manifest: AdapterManifest,
    loaded: bool,
}

impl InProcessHost {
    /// Create a new host with the given manifest. The V8 runtime is configured
    /// with the nexus ops extension; capabilities from the manifest are installed
    /// into `OpState` (§5.5 — capabilities are granted, not ambient).
    pub fn new(manifest: AdapterManifest) -> Result<Self, HostError> {
        let runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![ops::nexus_adapter_host::init()],
            ..Default::default()
        });

        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            ops::install_client(&mut state);
            ops::install_capabilities(&mut state, manifest.capabilities.fetch.clone());
        }

        Ok(Self {
            runtime,
            manifest,
            loaded: false,
        })
    }

    /// Load the adapter module from source code. The module must export an
    /// `adapter` object implementing the `CharacterAdapter` interface (§5.1).
    ///
    /// The host appends `globalThis.__nexusAdapter = adapter;` to the module
    /// code so the adapter object is accessible via `execute_script` after
    /// evaluation.
    pub async fn load(&mut self, code: &str) -> Result<(), HostError> {
        let wrapped = format!("{code}\nglobalThis.__nexusAdapter = adapter;\n");

        let specifier = deno_core::resolve_url("nexus:adapter")
            .map_err(|e| HostError::ModuleLoad(format!("invalid specifier: {e}")))?;

        let module_id = self
            .runtime
            .load_main_es_module_from_code(&specifier, wrapped)
            .await?;

        let evaluate_future = self.runtime.mod_evaluate(module_id);
        self.runtime
            .run_event_loop(PollEventLoopOptions::default())
            .await?;
        evaluate_future.await?;

        self.loaded = true;
        Ok(())
    }

    /// Call a method on the adapter and return the result as JSON.
    ///
    /// The script wraps the call in `JSON.stringify` so the V8 value is a
    /// string — this avoids `serde_v8` and keeps the scope handling simple.
    /// `?? null` ensures `undefined` becomes `"null"` rather than `undefined`.
    async fn call_method(
        &mut self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, HostError> {
        if !self.loaded {
            return Err(HostError::ModuleLoad(
                "adapter not loaded — call load() first".into(),
            ));
        }

        let args_json: Vec<String> = args
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
            .collect();

        let script = format!(
            "globalThis.__nexusAdapter.{}({}).then(r => JSON.stringify(r ?? null))",
            method,
            args_json.join(", ")
        );

        let promise = self
            .runtime
            .execute_script(deno_core::ascii_str!("nexus_call"), script)
            .map_err(|e| HostError::Runtime(e.to_string()))?;

        // Resolve the promise while polling the event loop — `resolve()`
        // alone returns a future that never completes unless the event loop
        // is polled concurrently (deno_core 0.410 API).
        // `resolve()` returns `impl Future + use<>` (no borrow of self), so
        // we can pass it to `with_event_loop_promise` on the same runtime.
        let resolve_future = self.runtime.resolve(promise);
        let result = self
            .runtime
            .with_event_loop_promise(resolve_future, PollEventLoopOptions::default())
            .await
            .map_err(|e| HostError::Runtime(e.to_string()))?;

        // Create a V8 scope with the runtime's main context entered.
        // v8 150.4.0 scope API: HandleScope::new → pin! → init → ContextScope.
        let context = self.runtime.main_context();
        let isolate = self.runtime.v8_isolate();
        let handle_scope = v8::HandleScope::new(isolate);
        let handle_scope = std::pin::pin!(handle_scope);
        let mut handle_scope = handle_scope.init();
        let context_local = v8::Local::new(&handle_scope, context);
        let context_scope = v8::ContextScope::new(&mut handle_scope, context_local);

        // Extract the JSON string from the resolved V8 global.
        let local = v8::Local::new(&context_scope, result);
        let json_str = local.to_rust_string_lossy(&context_scope);

        let json: serde_json::Value = serde_json::from_str(&json_str)?;

        Ok(json)
    }

    /// Call `adapter.import(raw, ctx)` — the boundary check fires here:
    /// adapter output is deserialized and validated in Rust before the engine
    /// sees it (§5.5, §1).
    pub async fn import(
        &mut self,
        raw: serde_json::Value,
        ctx: ImportContext,
    ) -> Result<ImportOutcome, HostError> {
        let ctx_json = serde_json::to_value(&ctx)?;
        let result = timeout(
            DEFAULT_TIMEOUT,
            self.call_method("import", &[raw, ctx_json]),
        )
        .await
        .map_err(|_| HostError::Timeout(DEFAULT_TIMEOUT))??;

        validation::validate_import(result)
    }

    /// Call `adapter.export(build, ctx)` — same boundary check.
    pub async fn export(
        &mut self,
        build: serde_json::Value,
        ctx: ExportContext,
    ) -> Result<ExportOutcome, HostError> {
        let ctx_json = serde_json::to_value(&ctx)?;
        let result = timeout(
            DEFAULT_TIMEOUT,
            self.call_method("export", &[build, ctx_json]),
        )
        .await
        .map_err(|_| HostError::Timeout(DEFAULT_TIMEOUT))??;

        validation::validate_export(result)
    }

    /// Call `adapter.detectChanges(previous, current)` (§5.3).
    pub async fn detect_changes(
        &mut self,
        previous: serde_json::Value,
        current: serde_json::Value,
    ) -> Result<ChangeSet, HostError> {
        let result = timeout(
            DEFAULT_TIMEOUT,
            self.call_method("detectChanges", &[previous, current]),
        )
        .await
        .map_err(|_| HostError::Timeout(DEFAULT_TIMEOUT))??;

        validation::validate_changes(result)
    }

    /// Call `adapter.validate(raw)` (§5.1).
    pub async fn validate(
        &mut self,
        raw: serde_json::Value,
    ) -> Result<ValidationReport, HostError> {
        let result = timeout(DEFAULT_TIMEOUT, self.call_method("validate", &[raw]))
            .await
            .map_err(|_| HostError::Timeout(DEFAULT_TIMEOUT))??;

        validation::validate_report(result)
    }

    /// Returns the manifest's adapter ID.
    pub fn adapter_id(&self) -> &str {
        &self.manifest.id
    }

    /// Returns the manifest's fetch capabilities (for testing).
    pub fn fetch_capabilities(&self) -> &FetchCapabilities {
        &self.manifest.capabilities.fetch
    }
}
