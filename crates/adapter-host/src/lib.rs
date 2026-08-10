//! Adapter host — embeds deno_core, runs TS adapters sandboxed, validates
//! adapter output against the canonical schema in Rust before the engine sees
//! it (§5.5, §1). Capabilities are granted, never ambient.
//! Phase 0.1: scaffold only. Lands in 1.1.
