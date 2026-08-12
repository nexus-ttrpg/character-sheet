//! Canonical character model — the source of truth (§3, §3.6).
//!
//! These Rust types are the single hand-written definition from which
//! `canonical.schema.json`, `@nexus-sheet/schema-types`, and
//! `@nexus-sheet/schema-zod` are generated (one-way codegen, §3.6).
//!
//! ## Three-layer validation (§3.1)
//!
//! - `serde` — serialization + structural validation (types, required fields,
//!   unknown-field rejection via `deny_unknown_fields` on typed regions).
//!   Deserialization is total: if you have a `CharacterBuild`, it is
//!   structurally valid by construction.
//! - `schemars` — generates JSON Schema from the same types (feeds OpenAPI §13
//!   and the published schema artifact for third-party validation).
//! - `garde` — semantic validation the type system can't express (level bounds,
//!   ability-score bounds, cross-field rules). Wired here; the full
//!   ruleset lives in `SystemPlugin::validate` (§10.2, implemented in 0.3).
//!
//! ## Load-bearing invariants
//!
//! - `BTreeMap`, not `HashMap`, everywhere (§3.3) — deterministic serialization
//!   order is what makes content hashing correct (§9.2).
//! - Newtypes over primitives (`Level`, `ContentHash`, `Ulid`, …) close off a
//!   class of adapter bugs (§3.3).
//! - `deny_unknown_fields` on typed regions; `ExtensionBag` is the deliberate
//!   escape hatch for unmodeled data (§3.1, §3.5).
//! - `SessionState` has no conversion into `CharacterBuild` (§4) — session state
//!   is never sync input. Enforced by types, not convention.
//! - `content_hash` is BLAKE3 of canonicalized `build` only — not `session`,
//!   not `meta` (§9.2).

pub mod ability;
pub mod build_types;
pub mod character;
pub mod extensions;
pub mod hashing;
pub mod newtypes;
pub mod plugin;
pub mod proficiency;
pub mod refs;
pub mod session;
pub mod testing;

pub use ability::AbilityScoreKey;
pub use build_types::*;
pub use character::{Character, CharacterBuild, CharacterMeta};
pub use extensions::{ExtensionBag, Namespace};
pub use hashing::content_hash;
pub use newtypes::{
    AdapterId, ContentHash, FieldPath, Level, PlatformId, SkillKey, SlotKey, SystemId, Ulid,
};
pub use plugin::{
    proficiency_or_untrained, ContentEntry, ContentIndex, ContentKind, DerivedStats, SystemPlugin,
    SystemPluginRegistry, ValidationError, ValidationReport, ValidationWarning,
};
pub use proficiency::{ProficiencyRank, ProficiencyTable};
pub use refs::{Slug, SystemRefId};
pub use session::SessionState;
