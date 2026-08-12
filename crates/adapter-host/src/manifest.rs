//! Adapter manifest — capability grants (§5.5).
//!
//! Capabilities are **granted, not ambient** (§5.5). An adapter gets no
//! filesystem access, no environment, no DB handle. Network access is a host
//! op, scoped by an allowlist declared in the manifest. A malicious or careless
//! adapter cannot read `/etc/passwd`, exfiltrate to an arbitrary host, or
//! reach the database.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

/// How the adapter is run (§5.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdapterRuntime {
    /// Default: run in-process via `deno_core` (V8 sandbox).
    #[default]
    InProcess,
    /// Escape hatch: run as a subprocess speaking JSON-RPC over stdio
    /// (§5.5). For adapters that genuinely need full Node.
    External,
}

/// Network access grants (§5.5). An empty allowlist means no network at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchCapabilities {
    /// Host origins the adapter may fetch from (scheme + host [+ port]).
    /// Examples: `https://pathbuilder2e.com`, `https://api.foundryvtt.com`.
    #[serde(default)]
    pub allow: Vec<String>,
}

impl FetchCapabilities {
    /// Returns true if `url` is allowed by the allowlist. Checks scheme + host
    /// (+ port if specified in the grant).
    pub fn is_allowed(&self, url: &str) -> bool {
        let Ok(parsed) = Url::parse(url) else {
            return false;
        };
        // Only HTTPS is allowed — no cleartext HTTP for credentials/tokens.
        if parsed.scheme() != "https" {
            return false;
        }
        let host = parsed.host_str().unwrap_or("");
        let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
        self.allow.iter().any(|grant| {
            let Ok(g) = Url::parse(grant) else {
                return false;
            };
            if g.scheme() != "https" {
                return false;
            }
            let host_match = g.host_str() == Some(host);
            let port_match =
                g.port().is_none() || g.port().map(|p| p.to_string()).as_deref() == Some(&port);
            host_match && port_match
        })
    }

    /// Returns true if no fetch grants are configured.
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }
}

/// Capabilities granted to an adapter (§5.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityGrants {
    /// Network fetch allowlist. Empty = no network (§5.5).
    #[serde(default)]
    pub fetch: FetchCapabilities,
}

/// The adapter manifest — declares the adapter's entry point, runtime mode,
/// and capability grants (§5.5).
///
/// ```json
/// {
///   "id": "pathbuilder",
///   "entry": "./dist/index.js",
///   "runtime": "inProcess",
///   "capabilities": { "fetch": { "allow": ["https://pathbuilder2e.com"] } }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterManifest {
    /// Adapter identifier — must match the adapter's `id` export (§5.1).
    pub id: String,

    /// Entry point — file path for in-process, executable path for external.
    pub entry: String,

    /// How the adapter is run (§5.5).
    #[serde(default)]
    pub runtime: AdapterRuntime,

    /// Capability grants (§5.5).
    #[serde(default)]
    pub capabilities: CapabilityGrants,
}

impl AdapterManifest {
    /// Create a minimal in-process manifest with no capabilities.
    pub fn new(id: impl Into<String>, entry: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            entry: entry.into(),
            runtime: AdapterRuntime::InProcess,
            capabilities: CapabilityGrants::default(),
        }
    }

    /// Returns the set of allowed fetch origins for quick lookup.
    pub fn allowed_hosts(&self) -> HashSet<&str> {
        self.capabilities
            .fetch
            .allow
            .iter()
            .map(|s| s.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_allowlist_matches_host() {
        let caps = FetchCapabilities {
            allow: vec!["https://pathbuilder2e.com".into()],
        };
        assert!(caps.is_allowed("https://pathbuilder2e.com/api/character/123"));
        assert!(caps.is_allowed("https://pathbuilder2e.com/v2/export"));
        assert!(!caps.is_allowed("https://evil.com/exfiltrate"));
        assert!(!caps.is_allowed("http://pathbuilder2e.com"));
    }

    #[test]
    fn fetch_allowlist_with_port() {
        let caps = FetchCapabilities {
            allow: vec!["https://api.foundryvtt.com:8443".into()],
        };
        assert!(caps.is_allowed("https://api.foundryvtt.com:8443/v1"));
        assert!(!caps.is_allowed("https://api.foundryvtt.com/v1"));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let caps = FetchCapabilities::default();
        assert!(!caps.is_allowed("https://anywhere.com"));
        assert!(caps.is_empty());
    }

    #[test]
    fn manifest_round_trip() {
        let manifest = AdapterManifest {
            id: "foundry".into(),
            entry: "./dist/index.js".into(),
            runtime: AdapterRuntime::InProcess,
            capabilities: CapabilityGrants {
                fetch: FetchCapabilities {
                    allow: vec!["https://api.foundryvtt.com".into()],
                },
            },
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AdapterManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, back);
    }

    #[test]
    fn external_runtime_deserializes() {
        let json = r#"{"id":"heavy","entry":"node ./dist/index.js","runtime":"external","capabilities":{"fetch":{"allow":[]}}}"#;
        let manifest: AdapterManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.runtime, AdapterRuntime::External);
    }
}
