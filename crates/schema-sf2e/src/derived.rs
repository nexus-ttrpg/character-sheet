//! SF2e derived-stat computation (§3.3, §10.2).
//!
//! SF2e shares its derived-stat chassis with PF2e (§3.4); the shared math lives
//! in [`schema_core::chassis`]. SF2e-specific outputs that don't overlap with
//! the chassis ride in `extensions` (§10.2) — notably Resolve Points, which are
//! session-scoped and thus never a build-derived stat (§4).

use schema_core::character::CharacterBuild;
use schema_core::chassis;
use schema_core::plugin::DerivedStats;
use schema_core::ContentIndex;

/// Compute derived stats for an SF2e build (§10.2). Delegates to the shared
/// chassis; the content index supplies SF2e ancestry HP.
pub fn compute(build: &CharacterBuild, content_index: &ContentIndex) -> DerivedStats {
    chassis::derived_stats(build, content_index)
}
