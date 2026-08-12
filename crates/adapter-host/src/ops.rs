//! Ops exposed to TS adapters — the deliberately tiny op layer (§5.5, §21).
//!
//! Only two ops exist: `nexus_fetch` (network, scoped by manifest allowlist)
//! and `nexus_log` (logging). No filesystem, no environment, no DB handle.
//! This is the security posture an all-Rust design only gets from a future
//! WASM runtime — here it's day one (§5.5).

use deno_core::op2;
use deno_core::OpState;
use deno_error::JsErrorBox;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use tracing::{debug, warn};

use crate::manifest::FetchCapabilities;

/// A simplified fetch response — body as string + status + headers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Fetch init options — a subset of the standard `fetch` init.
#[derive(Debug, Clone, Deserialize)]
pub struct FetchInit {
    #[serde(default = "default_get")]
    pub method: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub body: Option<String>,
}

fn default_get() -> String {
    "GET".into()
}

/// `nexus_fetch(url, init?)` — HTTP fetch scoped by the adapter's manifest
/// allowlist (§5.5). Denies non-allowlisted hosts; only HTTPS is permitted.
///
/// The reqwest client and capabilities are read from `OpState` before any
/// `.await` point so no borrows are held across the async boundary.
#[op2]
#[serde]
pub async fn op_nexus_fetch(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[serde] init: Option<FetchInit>,
) -> Result<FetchResponse, JsErrorBox> {
    // Extract everything from OpState before any .await point — the RefCell
    // borrow must not be held across an async boundary.
    let (caps, client) = {
        let state = state.borrow();
        let caps = state
            .try_borrow::<FetchCapabilities>()
            .cloned()
            .unwrap_or_default();
        let client = state
            .try_borrow::<Client>()
            .cloned()
            .ok_or_else(|| JsErrorBox::generic("reqwest client not in OpState"))?;
        (caps, client)
    };

    if !caps.is_allowed(&url) {
        return Err(JsErrorBox::generic(format!(
            "capability denied: fetch to '{url}' is not in the adapter's allowlist"
        )));
    }

    let method = init
        .as_ref()
        .map(|i| i.method.clone())
        .unwrap_or_else(|| "GET".into());
    let headers = init
        .as_ref()
        .and_then(|i| i.headers.clone())
        .unwrap_or_default();
    let body = init.as_ref().and_then(|i| i.body.clone());

    let mut req = client.request(method.parse().unwrap_or(reqwest::Method::GET), &url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.body(b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| JsErrorBox::generic(format!("fetch failed: {e}")))?;

    let status = resp.status().as_u16();
    let mut hdrs = HashMap::new();
    for (k, v) in resp.headers() {
        if let Ok(s) = v.to_str() {
            hdrs.insert(k.as_str().to_string(), s.to_string());
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| JsErrorBox::generic(format!("fetch body read failed: {e}")))?;

    Ok(FetchResponse {
        status,
        headers: hdrs,
        body,
    })
}

/// `nexus_log(level, message)` — adapter logging. Forwarded to `tracing`.
#[op2(fast)]
pub fn op_nexus_log(#[string] level: String, #[string] message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "adapter", "{message}"),
        "warn" => warn!(target: "adapter", "{message}"),
        "info" => debug!(target: "adapter", "{message}"),
        _ => debug!(target: "adapter", "{message}"),
    }
}

/// Initialize the reqwest client in `OpState`. Called by the host after
/// creating the runtime, before loading any adapter.
pub fn install_client(state: &mut OpState) {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("reqwest client build");
    state.put(client);
}

/// Initialize fetch capabilities in `OpState`.
pub fn install_capabilities(state: &mut OpState, caps: FetchCapabilities) {
    state.put(caps);
}

deno_core::extension!(nexus_adapter_host, ops = [op_nexus_fetch, op_nexus_log],);
