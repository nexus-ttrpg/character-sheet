// `nexus` CLI — offline-first single static binary (§12, §19).
//
// Drives the Phase 1 flow: import a platform payload into canonical, diff
// against the current HEAD, push canonical back out to a platform.
//
// All adapter execution goes through the adapter-host (§5.5) — the Foundry
// adapter runs in the V8 sandbox, boundary-validated, never trusted blindly.
// The local store is SQLite (§12); no server contact is required.

mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// `nexus` — Nexus Character Sheet CLI (§12, §19).
///
/// Offline-first character sync for PF2e/SF2e. Local SQLite store; adapter
/// bundles alongside the binary. No server required for self-hosting (§12).
#[derive(Parser, Debug)]
#[command(name = "nexus", version, about)]
struct Cli {
    /// Path to the local SQLite database (default: ./nexus.sqlite).
    #[arg(long, global = true, default_value = "nexus.sqlite")]
    db: PathBuf,

    /// Path to the adapter bundles directory (default: ./adapters).
    #[arg(long, global = true, default_value = "adapters")]
    adapters_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Import a platform payload into canonical, creating a new snapshot (§19).
    Import {
        /// Platform to import from.
        #[arg(long)]
        foundry: bool,

        /// Existing character ID (ULID) to update. If omitted, creates a new
        /// character.
        #[arg(long)]
        character: Option<String>,

        /// Path to the platform payload JSON file (e.g. a Foundry Actor export).
        file: PathBuf,
    },

    /// Export the current HEAD canonical build to a platform payload (§19).
    Push {
        /// Platform to export to.
        #[arg(long)]
        foundry: bool,

        /// Character ID (ULID) whose HEAD to export.
        #[arg(long)]
        character: String,

        /// Output path for the platform payload JSON.
        #[arg(long, default_value = "./output.json")]
        out: PathBuf,
    },

    /// Show field-level changes between HEAD and a new import, or between
    /// two snapshots (§11.2, §19).
    Diff {
        /// Diff a new platform payload against a character's current HEAD.
        #[arg(long)]
        foundry: bool,

        /// Character ID (ULID) whose HEAD to diff against.
        #[arg(long)]
        character: Option<String>,

        /// Path to the platform payload JSON to diff against HEAD.
        file: Option<PathBuf>,

        /// Diff between two snapshot IDs instead.
        #[arg(long)]
        from: Option<String>,

        /// Second snapshot ID (requires --from).
        #[arg(long)]
        to: Option<String>,
    },

    /// List snapshot history for a character (§11.1).
    History {
        /// Character ID (ULID).
        character_id: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let cli = Cli::parse();

    // V8 (deno_core) is !Send, so adapter-host calls must run inside a
    // LocalSet on the current thread (§5.5, host.rs).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move { commands::dispatch(cli).await })
}
