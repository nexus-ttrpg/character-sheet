//! Content hashing (§9.2).
//!
//! `meta.content_hash` is a BLAKE3 hash of a canonicalized (stable key order,
//! no whitespace) serialization of `build` only — not `session`, not `meta`.
//! BLAKE3 over SHA-256 because hashing happens on every write and every drift
//! check across potentially thousands of characters, and it's several times
//! faster with no security downside for a non-adversarial integrity check.
//!
//! Determinism is the whole game here, which is why §3.3 mandates `BTreeMap`.
//! With `BTreeMap` everywhere, canonicalization is just "serialize it" —
//! `serde_json::to_vec` produces compact (no whitespace) JSON with sorted map
//! keys.

use crate::character::CharacterBuild;
use crate::newtypes::ContentHash;

/// Compute the BLAKE3 content hash of a `CharacterBuild` (§9.2).
///
/// Hashes `build` only — never `session` or `meta`. The canonical serialization
/// is `serde_json::to_vec(build)`, which is compact and deterministic because
/// every map in the schema is a `BTreeMap` (§3.3).
///
/// Property tests (§16, lands in 0.5) assert `hash(x) == hash(roundtrip(x))`
/// for arbitrary generated characters.
pub fn content_hash(build: &CharacterBuild) -> ContentHash {
    let bytes = serde_json::to_vec(build).expect("CharacterBuild is always serializable");
    let digest = blake3::hash(&bytes);
    ContentHash::from_bytes(digest.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::minimal_build;

    #[test]
    fn hash_is_deterministic() {
        let build = minimal_build();
        let h1 = content_hash(&build);
        let h2 = content_hash(&build);
        assert_eq!(h1, h2, "same build must hash identically");
    }

    #[test]
    fn hash_is_hex() {
        let build = minimal_build();
        let hex = content_hash(&build).to_hex();
        assert_eq!(hex.len(), 64, "BLAKE3 hex is 64 chars");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
