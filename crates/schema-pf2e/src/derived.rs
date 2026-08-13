//! PF2e derived-stat computation (§3.3, §10.2).
//!
//! PF2e shares its derived-stat chassis with SF2e (§3.4); the shared math lives
//! in [`schema_core::chassis`]. This module is the PF2e plugin's delegation
//! point — PF2e currently adds no system-specific derived stats (§10.2 notes
//! system-specific outputs can ride in `extensions`). If PF2e-specific derived
//! values are added later, they append here on top of the chassis result.

use schema_core::character::CharacterBuild;
use schema_core::chassis;
use schema_core::plugin::DerivedStats;
use schema_core::ContentIndex;

/// Compute derived stats for a PF2e build (§10.2). Delegates to the shared
/// chassis; the content index supplies PF2e ancestry HP.
pub fn compute(build: &CharacterBuild, content_index: &ContentIndex) -> DerivedStats {
    chassis::derived_stats(build, content_index)
}
