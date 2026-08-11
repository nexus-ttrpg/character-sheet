//! Newtypes over primitives (§3.3).
//!
//! These close off a class of adapter bugs: you cannot pass a character's level
//! where a proficiency rank is expected, and a `ContentHash` cannot be
//! accidentally compared to an arbitrary string. In the TypeScript design these
//! were all `number` and `string` and the mistakes were caught, if at all, by
//! tests.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Level ──────────────────────────────────────────────────────────────────

/// Character level, bounded 1..=20 enforced on construction (§3.3, §3.1).
///
/// Uses `#[serde(try_from = "u8")]` so deserialization enforces the bound:
/// a payload with `"level": 0` or `"level": 21` is rejected at the boundary,
/// not silently accepted. "If you have a `Level`, it's valid by construction."
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(try_from = "u8", into = "u8")]
pub struct Level(u8);

#[derive(Debug, thiserror::Error)]
#[error("level must be between 1 and 20, got {0}")]
pub struct LevelError(pub u8);

impl Level {
    /// Construct a `Level` from a raw `u8`, enforcing 1..=20 (§3.3).
    pub fn new(value: u8) -> Result<Self, LevelError> {
        if (1..=20).contains(&value) {
            Ok(Self(value))
        } else {
            Err(LevelError(value))
        }
    }

    /// The raw level value. Guaranteed to be in 1..=20.
    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Level {
    type Error = LevelError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Level> for u8 {
    fn from(level: Level) -> u8 {
        level.0
    }
}

// ── ContentHash ────────────────────────────────────────────────────────────

/// BLAKE3 content hash of canonicalized `build` (§9.2). 32 bytes, serialized
/// as lowercase hex for readability in JSON and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Wrap a raw 32-byte BLAKE3 digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex representation.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom(format!(
                "content hash must be 32 bytes (64 hex chars), got {} bytes",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }
}

impl JsonSchema for ContentHash {
    fn schema_name() -> String {
        "ContentHash".into()
    }

    fn json_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^[0-9a-f]{64}$",
            "description": "BLAKE3 content hash (64 lowercase hex chars, §9.2)"
        }))
        .expect("valid schema")
    }
}

// ── String-backed newtypes ─────────────────────────────────────────────────
//
// These are transparent over `String` for serde (serialize as a plain string)
// and generate a `{"type":"string"}` JSON Schema. Each has a validation
// constructor where the format matters; otherwise they're constructed freely.

macro_rules! string_newtype {
    (
        $(#[doc = $doc:expr])*
        $name:ident
    ) => {
        $(#[doc = $doc])*
        #[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_newtype!(
    /// A ULID identifier (§3.2). Stored as the 26-char Crockford base32 string;
    /// string ordering matches byte ordering, preserving the natural cursor
    /// property for pagination (§18). Validation/generation of ULIDs is the
    /// store's job (0.6); here it's an opaque validated string.
    Ulid
);

string_newtype!(
    /// System identifier — `"pf2e"` | `"sf2e"`; extensible via the plugin
    /// registry (§3.2, §10).
    SystemId
);

string_newtype!(
    /// Platform identifier — `"foundry"` | `"roll20"` | … (§4, §13.3).
    PlatformId
);

string_newtype!(
    /// Adapter identifier (§5.1, §5.5). Usually matches the platform id but
    /// distinct: one platform could have multiple adapters.
    AdapterId
);

string_newtype!(
    /// An extension namespace — e.g. `"foundry.pf2e"`, `"homebrew.myGuild"`
    /// (§3.5). Dotted convention; no enforcement beyond being a non-empty
    /// string in 0.2.
    Namespace
);

string_newtype!(
    /// A skill key, including lore skills (§3.3). e.g. `"athletics"`,
    /// `"lore:alchemy"`.
    SkillKey
);

string_newtype!(
    /// A spell-slot key (§4). e.g. `"rank1"`, `"rank2"`.
    SlotKey
);

string_newtype!(
    /// A canonical field path for diff/merge (§11.2). e.g.
    /// `"build.skills.athletics.rank"`. The typed `canonicalPath()` helper on
    /// the TS side is the compile-time-checked version (§6.3); on the Rust
    /// side this is a path string carried by `FieldDiff`.
    FieldPath
);
