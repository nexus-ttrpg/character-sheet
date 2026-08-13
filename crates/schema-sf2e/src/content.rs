//! SF2e content index — curated slug → rules data map (§9.1).
//!
//! Minimal in 2.4: covers the ancestry referenced by the SF2e fixture
//! (`examples/fixtures/sf2e-level5-mechanic.json`) so `derived_stats` can
//! compute ancestry HP. SF2e reuses the PF2e chassis values (§3.4: "SF2e
//! reuses most of the PF2e chassis"); the full content index is a curation
//! effort that grows over time, paralleling `schema-pf2e`'s minimal 0.3 set.

use schema_core::{ContentEntry, ContentIndex};

/// Build the SF2e content index (§9.1).
///
/// In 2.4 this is a small static set sourced from *Starfinder Player Core*. A
/// future version will load from a versioned data file or embedded compendium,
/// mirroring the planned evolution of the PF2e index.
pub fn build() -> ContentIndex {
    let mut idx = ContentIndex::new(semver::Version::new(0, 1, 0));

    // Human — HP 8, speed 25 ft. SF2e reuses the PF2e ancestry chassis (§3.4);
    // provisional pending full SF2e content curation.
    idx.insert(ContentEntry::ancestry("human", 8, 25));

    idx
}
