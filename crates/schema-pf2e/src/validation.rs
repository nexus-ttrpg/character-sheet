//! PF2e semantic validation (§3.1, §10.2).
//!
//! This is the second validation layer. `serde` + `deny_unknown_fields` already
//! enforce structural validity by construction (if you have a `CharacterBuild`,
//! it deserialized cleanly). This module enforces cross-field and ruleset rules
//! the type system can't express.
//!
//! PF2e shares its validation chassis with SF2e (§3.4); the shared rules live
//! in [`schema_core::chassis`]. PF2e currently adds no rules beyond the chassis
//! — when it does (e.g. PF2e-only feat-slot constraints), they append here.
//!
//! Philosophy (§5.2): unknown/homebrew content is a **warning**, never an error.
//! Errors are for hard ruleset violations only.

use schema_core::character::CharacterBuild;
use schema_core::chassis;
use schema_core::ValidationReport;

/// Validate a PF2e `CharacterBuild` (§10.2). Delegates to the shared chassis;
/// PF2e-specific rules append on top when needed.
pub fn validate(build: &CharacterBuild) -> ValidationReport {
    chassis::validate_chassis(build)
}
