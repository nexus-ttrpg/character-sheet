//! Nexus Sheet API server binary (§13, §14, §17).
//!
//! Reads configuration from environment variables, constructs `AppState`,
//! and serves the Axum router. The API server uses Postgres (§12); for the
//! offline SQLite path, use the `nexus` CLI.
//!
//! ## Environment variables
//!
//! | Variable | Required | Default | Description |
//! |----------|----------|---------|-------------|
//! | `DATABASE_URL` | yes | — | Postgres connection string |
//! | `NEXUS_PORT` | no | `3000` | Port to listen on |
//! | `NEXUS_ADAPTERS_DIR` | no | `./adapters` | Directory holding adapter bundles |
//! | `NEXUS_ENCRYPTION_KEY_FILE` | no¹ | — | Path to a 32-byte key file for credential encryption (§17) |
//! | `NEXUS_ADMIN_API_KEY` | no | — | Bootstrap admin API key (full scope, never hashed) |
//!
//! ¹ If unset, a random key is generated with a warning. This is fine for dev
//! but **not for production** — encrypted credentials become unreadable after
//! restart (§17).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::warn;
use tracing_subscriber::EnvFilter;

use api::auth::KeyScope;
use api::state::AppState;
use store::{EncryptionKey, PgStore, Store};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required (e.g. postgres://user:pass@localhost/nexus)")?;

    let port: u16 = std::env::var("NEXUS_PORT")
        .ok()
        .map(|s| s.parse())
        .transpose()
        .context("NEXUS_PORT must be a valid port number")?
        .unwrap_or(3000);

    let adapters_dir = std::env::var("NEXUS_ADAPTERS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("adapters"));

    let encryption_key = load_encryption_key()?;

    tracing::info!("connecting to Postgres");
    let store = PgStore::connect(&database_url)
        .await
        .context("failed to connect to Postgres")?;
    store.migrate().await.context("failed to run migrations")?;

    let store: Arc<dyn Store> = Arc::new(store);

    let state = AppState::new(store, adapters_dir, encryption_key);

    if let Ok(admin_key) = std::env::var("NEXUS_ADMIN_API_KEY") {
        tracing::info!("seeding admin API key from NEXUS_ADMIN_API_KEY");
        let admin_owner = store::new_ulid();
        let mut registry = state.api_keys.write().await;
        registry.insert_plaintext(
            admin_key,
            admin_owner,
            KeyScope::Admin,
            "bootstrap admin".to_string(),
        );
    }

    let app = api::app(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("failed to bind 0.0.0.0:{port}"))?;
    tracing::info!("listening on 0.0.0.0:{port}");

    axum::serve(listener, app).await.context("server error")?;

    Ok(())
}

/// Load the AES-256 encryption key from `NEXUS_ENCRYPTION_KEY_FILE` or generate
/// a random one with a warning (§17).
fn load_encryption_key() -> Result<EncryptionKey> {
    match std::env::var("NEXUS_ENCRYPTION_KEY_FILE") {
        Ok(path) => {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read encryption key file: {path}"))?;
            if bytes.len() != 32 {
                anyhow::bail!(
                    "encryption key file must be exactly 32 bytes, got {}",
                    bytes.len()
                );
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            tracing::info!("loaded encryption key from {path}");
            Ok(EncryptionKey::from_bytes(key))
        }
        Err(_) => {
            warn!(
                "NEXUS_ENCRYPTION_KEY_FILE not set — generating a random key. \
                 Encrypted credentials will be unreadable after restart. \
                 This is fine for development but NOT for production (§17)."
            );
            Ok(EncryptionKey::generate())
        }
    }
}
