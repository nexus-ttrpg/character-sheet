//! SF2e semantic validation (§3.1, §10.2).
//!
//! This is the second validation layer. `serde` + `deny_unknown_fields` already
//! enforce structural validity by construction (if you have a `CharacterBuild`,
//! it deserialized cleanly). This module enforces cross-field and ruleset rules
//! the type system can't express.
//!
//! SF2e shares its validation chassis with PF2e (§3.4); the shared rules live
//! in [`schema_core::chassis`]. SF2e-specific rules (e.g. drone chassis/mod
//! constraints) will append here as system coverage matures; for now the
//! chassis rules apply.
//!
//! Philosophy (§5.2): unknown/homebrew content is a **warning**, never an error.

use schema_core::character::CharacterBuild;
use schema_core::chassis;
use schema_core::ValidationReport;

/// Validate an SF2e `CharacterBuild` (§10.2). Delegates to the shared chassis;
/// SF2e-specific rules append on top when needed.
pub fn validate(build: &CharacterBuild) -> ValidationReport {
    chassis::validate_chassis(build)
}
