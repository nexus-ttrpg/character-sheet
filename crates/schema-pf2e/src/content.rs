//! PF2e content index — curated slug → rules data map (§9.1).
//!
//! Minimal in 0.3: covers the ancestries referenced by `minimal_build()` and
//! the test fixtures so `derived_stats` can compute ancestry HP. The full
//! content index is a large curation effort that grows over time.

use schema_core::{ContentEntry, ContentIndex};

/// Build the PF2e content index (§9.1).
///
/// In 0.3 this is a small static set. A future version will load from a
/// versioned data file or embedded compendium.
pub fn build() -> ContentIndex {
    let mut idx = ContentIndex::new(semver::Version::new(0, 1, 0));

    // Core ancestries (Player Core) — HP and base speed.
    idx.insert(ContentEntry::ancestry("human", 8, 25));
    idx.insert(ContentEntry::ancestry("elf", 6, 30));
    idx.insert(ContentEntry::ancestry("dwarf", 10, 20));
    idx.insert(ContentEntry::ancestry("gnome", 8, 25));
    idx.insert(ContentEntry::ancestry("halfling", 6, 25));
    idx.insert(ContentEntry::ancestry("orc", 10, 25));
    idx.insert(ContentEntry::ancestry("goblin", 6, 25));

    idx
}
