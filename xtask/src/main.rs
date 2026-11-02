//! xtask — build tooling run as ordinary Rust programs (§15).
//!
//! Subcommands:
//!   codegen   — Rust types → canonical.schema.json → @nexus-ttrpg/schema-types/zod (§3.6)
//!   openapi   — export OpenAPI spec from api types (§13)
//!   fixtures  — regenerate fixture manifests
//!
//! The codegen staleness check (`cargo xtask codegen && git diff --exit-code`)
//! is the single control backing the "one source of truth" guarantee (§3.6, §21).

mod codegen;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("codegen") => codegen::run(),
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
