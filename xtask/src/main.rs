//! xtask — build tooling run as ordinary Rust programs (§15).
//!
//! Subcommands:
//!   codegen   — Rust types → canonical.schema.json → @nexus-sheet/schema-types/zod (§3.6)
//!   openapi   — export OpenAPI spec from api types (§13)
//!   fixtures  — regenerate fixture manifests
//!
//! Phase 0.1: `codegen` is a stub that exits 0 (no types to generate yet).
//! Real implementation lands in 0.4. The CI staleness check
//! (`cargo xtask codegen && git diff --exit-code`) relies on this being a no-op
//! that writes nothing until 0.4 wires schemars.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("codegen") => {
            eprintln!("xtask codegen: nothing to generate yet (Phase 0.4 will wire schemars)");
            ExitCode::SUCCESS
        }
        Some("openapi") => {
            eprintln!("xtask openapi: not implemented (lands with §13 API in Phase 2)");
            ExitCode::SUCCESS
        }
        Some("fixtures") => {
            eprintln!("xtask fixtures: not implemented (lands in Phase 0.7)");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("xtask: unknown subcommand '{other}'");
            eprintln!("usage: cargo xtask <codegen|openapi|fixtures>");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: cargo xtask <codegen|openapi|fixtures>");
            ExitCode::FAILURE
        }
    }
}
