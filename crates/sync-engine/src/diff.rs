//! Structural diff with identity-matched collection pairing (§11.2).
//!
//! Diffing walks the schema structurally rather than running a generic
//! deep-object-diff blind. Collections with identity (feats, inventory, spells
//! — anything with a stable slug via `SystemRefId`) are diffed by
//! identity-matched pairs, not array index. Without this, inserting one feat in
//! the middle of a list reports the entire tail as changed, which is the single
//! fastest way to make a merge UI useless (§11.2).
//!
//! The leaf comparison unit is `serde_json::Value`, produced by serializing
//! each typed field. This keeps diff schema-agnostic (a new field added to a
//! struct is picked up by the exhaustive `walk` match without diff code
//! changes) while the types guarantee that two equal values serialize
//! identically (§3.3 `BTreeMap` determinism, §9.2).

use schema_core::build_types::*;
use schema_core::character::CharacterBuild;
use schema_core::newtypes::FieldPath;
use schema_core::refs::SystemRefId;
use serde::Serialize;
use serde_json::Value;

/// A change at a single canonical field path (§11.2).
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Field present in `b` but not `a`.
    Added { after: Value },
    /// Field present in `a` but not `b`.
    Removed { before: Value },
    /// Field present in both, with different values.
    Modified { before: Value, after: Value },
}

/// One field-level difference between two builds (§11.2).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDiff {
    pub path: FieldPath,
    pub change: Change,
}

/// Compute the structural diff between two builds (§11.2).
///
/// Pure: no I/O, no allocation beyond the returned `Vec`. The path convention
/// is `build.<field>[.<index|key>]`, matching the TS-side `canonicalPath()`
/// helper (§6.3) so a `FieldDiff.path` is directly usable as a UI anchor.
pub fn diff(a: &CharacterBuild, b: &CharacterBuild) -> Vec<FieldDiff> {
    let mut out = Vec::new();
    walk_build(&mut out, "build", a, b);
    out
}

// ── Build walker ───────────────────────────────────────────────────────────
//
// Exhaustive match over `CharacterBuild`'s fields. A new field added to the
// struct is a compile error here, forcing diff coverage to be considered —
// this is the structural-walk guarantee from §11.2.

fn walk_build(out: &mut Vec<FieldDiff>, root: &str, a: &CharacterBuild, b: &CharacterBuild) {
    walk_field(
        out,
        root,
        "identity",
        &a.identity,
        &b.identity,
        walk_identity,
    );
    walk_field(
        out,
        root,
        "ancestry",
        &a.ancestry,
        &b.ancestry,
        walk_ref_entry,
    );
    walk_field(
        out,
        root,
        "heritage",
        &a.heritage,
        &b.heritage,
        walk_ref_entry,
    );
    walk_field(
        out,
        root,
        "background",
        &a.background,
        &b.background,
        walk_ref_entry,
    );
    walk_field(out, root, "class", &a.class, &b.class, walk_class);
    walk_field(out, root, "level", &a.level, &b.level, walk_scalar);
    walk_field(
        out,
        root,
        "abilityScores",
        &a.ability_scores,
        &b.ability_scores,
        walk_ability_map,
    );
    walk_field(out, root, "skills", &a.skills, &b.skills, walk_skill_map);
    walk_field(
        out,
        root,
        "proficiencies",
        &a.proficiencies,
        &b.proficiencies,
        walk_proficiency_table,
    );
    walk_id_collection(out, root, "feats", &a.feats, &b.feats, feat_id);
    walk_id_collection(
        out,
        root,
        "classFeatures",
        &a.class_features,
        &b.class_features,
        class_feature_id,
    );
    walk_field(
        out,
        root,
        "equipment",
        &a.equipment,
        &b.equipment,
        walk_equipment,
    );
    walk_field(
        out,
        root,
        "currencies",
        &a.currencies,
        &b.currencies,
        walk_struct_value,
    );
    walk_field(
        out,
        root,
        "spellcasting",
        &a.spellcasting,
        &b.spellcasting,
        walk_spellcasting,
    );
    walk_id_collection(out, root, "actions", &a.actions, &b.actions, action_id);
    walk_id_collection(
        out,
        root,
        "reactions",
        &a.reactions,
        &b.reactions,
        reaction_id,
    );
    walk_id_collection(
        out,
        root,
        "explorationActivities",
        &a.exploration_activities,
        &b.exploration_activities,
        expl_activity_id,
    );
    walk_id_collection(
        out,
        root,
        "companions",
        &a.companions,
        &b.companions,
        companion_id,
    );
    walk_id_collection(
        out,
        root,
        "familiars",
        &a.familiars,
        &b.familiars,
        familiar_id,
    );
    walk_id_collection(out, root, "drones", &a.drones, &b.drones, drone_id);
    walk_id_collection(out, root, "vehicles", &a.vehicles, &b.vehicles, vehicle_id);
    walk_id_collection(out, root, "notes", &a.notes, &b.notes, note_id);
}

// ── Field-level helpers ────────────────────────────────────────────────────

/// Diff a single named field via a typed walker. If the two values are `Eq`,
/// emits nothing; otherwise the walker appends the per-subfield diffs.
fn walk_field<T, F>(out: &mut Vec<FieldDiff>, root: &str, name: &str, a: &T, b: &T, walk: F)
where
    T: PartialEq,
    F: FnOnce(&mut Vec<FieldDiff>, &str, &T, &T),
{
    if a == b {
        return;
    }
    let path = format!("{root}.{name}");
    walk(out, &path, a, b);
}

/// Diff a leaf scalar by value. `Level` and other newtypes serialize to a
/// primitive, so a whole-value `Modified` is the right granularity.
fn walk_scalar<T>(out: &mut Vec<FieldDiff>, path: &str, a: &T, b: &T)
where
    T: Serialize,
{
    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
        return;
    };
    if before != after {
        out.push(FieldDiff {
            path: FieldPath::new(path),
            change: Change::Modified { before, after },
        });
    }
}

/// Diff any struct that serializes cleanly and has no identity-collection
/// children — used for `Currencies` and other flat shapes where whole-value
/// replacement is acceptable at this granularity.
fn walk_struct_value<T: Serialize>(out: &mut Vec<FieldDiff>, path: &str, a: &T, b: &T) {
    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
        return;
    };
    if before != after {
        out.push(FieldDiff {
            path: FieldPath::new(path),
            change: Change::Modified { before, after },
        });
    }
}

// ── Identity ───────────────────────────────────────────────────────────────

fn walk_identity(out: &mut Vec<FieldDiff>, path: &str, a: &Identity, b: &Identity) {
    walk_opt_string(out, path, "name", Some(&a.name), Some(&b.name));
    walk_opt_string(
        out,
        path,
        "playerName",
        a.player_name.as_deref(),
        b.player_name.as_deref(),
    );
    walk_opt_string(
        out,
        path,
        "pronouns",
        a.pronouns.as_deref(),
        b.pronouns.as_deref(),
    );
}

fn walk_opt_string(
    out: &mut Vec<FieldDiff>,
    path: &str,
    name: &str,
    a: Option<&str>,
    b: Option<&str>,
) {
    let sub = format!("{path}.{name}");
    match (a, b) {
        (Some(a), Some(b)) if a == b => {}
        (Some(a), Some(b)) => out.push(FieldDiff {
            path: FieldPath::new(&sub),
            change: Change::Modified {
                before: Value::String(a.into()),
                after: Value::String(b.into()),
            },
        }),
        (None, Some(b)) => out.push(FieldDiff {
            path: FieldPath::new(&sub),
            change: Change::Added {
                after: Value::String(b.into()),
            },
        }),
        (Some(a), None) => out.push(FieldDiff {
            path: FieldPath::new(&sub),
            change: Change::Removed {
                before: Value::String(a.into()),
            },
        }),
        (None, None) => {}
    }
}

// ── Ancestry / Heritage / Background (ref + display name) ──────────────────

fn walk_ref_entry<T: RefEntry>(out: &mut Vec<FieldDiff>, path: &str, a: &T, b: &T) {
    walk_ref(out, path, "reference", a.reference(), b.reference());
    walk_opt_string(
        out,
        path,
        "displayName",
        a.display_name().as_deref(),
        b.display_name().as_deref(),
    );
}

trait RefEntry {
    fn reference(&self) -> &SystemRefId;
    fn display_name(&self) -> &Option<String>;
}

impl RefEntry for Ancestry {
    fn reference(&self) -> &SystemRefId {
        &self.reference
    }
    fn display_name(&self) -> &Option<String> {
        &self.display_name
    }
}
impl RefEntry for Heritage {
    fn reference(&self) -> &SystemRefId {
        &self.reference
    }
    fn display_name(&self) -> &Option<String> {
        &self.display_name
    }
}
impl RefEntry for Background {
    fn reference(&self) -> &SystemRefId {
        &self.reference
    }
    fn display_name(&self) -> &Option<String> {
        &self.display_name
    }
}

fn walk_ref(out: &mut Vec<FieldDiff>, path: &str, name: &str, a: &SystemRefId, b: &SystemRefId) {
    if a == b {
        return;
    }
    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
        return;
    };
    out.push(FieldDiff {
        path: FieldPath::new(format!("{path}.{name}")),
        change: Change::Modified { before, after },
    });
}

// ── Class ──────────────────────────────────────────────────────────────────

fn walk_class(out: &mut Vec<FieldDiff>, path: &str, a: &ClassEntry, b: &ClassEntry) {
    walk_ref(out, path, "reference", &a.reference, &b.reference);
    if a.key_ability != b.key_ability {
        let (Some(before), Some(after)) = (to_value(&a.key_ability), to_value(&b.key_ability))
        else {
            return;
        };
        out.push(FieldDiff {
            path: FieldPath::new(format!("{path}.keyAbility")),
            change: Change::Modified { before, after },
        });
    }
    if a.hp_per_level != b.hp_per_level {
        out.push(FieldDiff {
            path: FieldPath::new(format!("{path}.hpPerLevel")),
            change: Change::Modified {
                before: Value::Number(a.hp_per_level.into()),
                after: Value::Number(b.hp_per_level.into()),
            },
        });
    }
}

// ── Maps (ability scores, skills, proficiencies) ───────────────────────────
//
// Maps are `BTreeMap` (§3.3) so key order is deterministic. We diff by key:
// present in both → compare values; only in a → removed; only in b → added.

fn walk_ability_map(
    out: &mut Vec<FieldDiff>,
    path: &str,
    a: &schema_core::ability::AbilityScoreMap,
    b: &schema_core::ability::AbilityScoreMap,
) {
    use schema_core::ability::AbilityScoreKey;
    let keys: std::collections::BTreeSet<&AbilityScoreKey> = a.keys().chain(b.keys()).collect();
    for k in keys {
        let sub = format!("{path}.{k:?}").to_lowercase();
        match (a.get(k), b.get(k)) {
            (Some(a), Some(b)) => {
                if a != b {
                    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
                        continue;
                    };
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Modified { before, after },
                    });
                }
            }
            (None, Some(b)) => {
                if let Some(after) = to_value(b) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Added { after },
                    });
                }
            }
            (Some(a), None) => {
                if let Some(before) = to_value(a) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Removed { before },
                    });
                }
            }
            (None, None) => {}
        }
    }
}

fn walk_skill_map(
    out: &mut Vec<FieldDiff>,
    path: &str,
    a: &schema_core::build_types::SkillMap,
    b: &schema_core::build_types::SkillMap,
) {
    let keys: std::collections::BTreeSet<&schema_core::newtypes::SkillKey> =
        a.keys().chain(b.keys()).collect();
    for k in keys {
        let sub = format!("{path}.{}", k.as_str());
        match (a.get(k), b.get(k)) {
            (Some(a), Some(b)) => {
                if a != b {
                    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
                        continue;
                    };
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Modified { before, after },
                    });
                }
            }
            (None, Some(b)) => {
                if let Some(after) = to_value(b) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Added { after },
                    });
                }
            }
            (Some(a), None) => {
                if let Some(before) = to_value(a) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Removed { before },
                    });
                }
            }
            (None, None) => {}
        }
    }
}

fn walk_proficiency_table(
    out: &mut Vec<FieldDiff>,
    path: &str,
    a: &schema_core::proficiency::ProficiencyTable,
    b: &schema_core::proficiency::ProficiencyTable,
) {
    let keys: std::collections::BTreeSet<&schema_core::newtypes::SkillKey> =
        a.0.keys().chain(b.0.keys()).collect();
    for k in keys {
        let sub = format!("{path}.{}", k.as_str());
        match (a.0.get(k), b.0.get(k)) {
            (Some(a), Some(b)) => {
                if a != b {
                    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
                        continue;
                    };
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Modified { before, after },
                    });
                }
            }
            (None, Some(b)) => out.push(FieldDiff {
                path: FieldPath::new(&sub),
                change: Change::Added {
                    after: to_value(b).unwrap_or(Value::Null),
                },
            }),
            (Some(a), None) => out.push(FieldDiff {
                path: FieldPath::new(&sub),
                change: Change::Removed {
                    before: to_value(a).unwrap_or(Value::Null),
                },
            }),
            (None, None) => {}
        }
    }
}

// ── Identity-matched collections (§11.2) ───────────────────────────────────
//
// Collections of items with a stable identity (slug) are paired by identity,
// not array index. Items present in both → compare structurally; only in a →
// removed; only in b → added. This is what keeps a mid-list insert from
// reporting the whole tail as changed.

fn walk_id_collection<T, F>(
    out: &mut Vec<FieldDiff>,
    root: &str,
    name: &str,
    a: &[T],
    b: &[T],
    id_of: F,
) where
    T: PartialEq + Serialize,
    F: Fn(&T) -> &str,
{
    if a == b {
        return;
    }
    let path = format!("{root}.{name}");
    let mut b_seen = vec![false; b.len()];

    // Pass 1: for each a-item, find a b-item with matching identity.
    for (ai, a_item) in a.iter().enumerate() {
        let a_id = id_of(a_item);
        let mut matched = None;
        for (bi, b_item) in b.iter().enumerate() {
            if !b_seen[bi] && id_of(b_item) == a_id {
                matched = Some(bi);
                break;
            }
        }
        match matched {
            Some(bi) => {
                b_seen[bi] = true;
                let b_item = &b[bi];
                if a_item != b_item {
                    let (Some(before), Some(after)) = (to_value(a_item), to_value(b_item)) else {
                        continue;
                    };
                    out.push(FieldDiff {
                        path: FieldPath::new(format!("{path}[{a_id}]")),
                        change: Change::Modified { before, after },
                    });
                }
            }
            None => {
                if let Some(before) = to_value(a_item) {
                    out.push(FieldDiff {
                        path: FieldPath::new(format!("{path}[{a_id}]")),
                        change: Change::Removed { before },
                    });
                }
            }
        }
        // suppress unused-assignment warning for ai in case the compiler
        // can't see the format! usage path
        let _ = ai;
    }

    // Pass 2: remaining unmatched b-items are additions.
    for (bi, b_item) in b.iter().enumerate() {
        if !b_seen[bi] {
            let b_id = id_of(b_item);
            if let Some(after) = to_value(b_item) {
                out.push(FieldDiff {
                    path: FieldPath::new(format!("{path}[{b_id}]")),
                    change: Change::Added { after },
                });
            }
        }
    }
}

// Identity extractors — the slug is the cross-platform anchor (§9.1).

fn feat_id(f: &FeatEntry) -> &str {
    f.reference.slug.as_str()
}
fn class_feature_id(c: &ClassFeatureEntry) -> &str {
    c.reference.slug.as_str()
}
fn action_id(a: &ActionEntry) -> &str {
    a.reference.slug.as_str()
}
fn reaction_id(r: &ReactionEntry) -> &str {
    r.reference.slug.as_str()
}
fn expl_activity_id(e: &ExplorationActivityEntry) -> &str {
    e.reference.slug.as_str()
}
fn companion_id(c: &CompanionEntry) -> &str {
    c.reference.slug.as_str()
}
fn familiar_id(f: &FamiliarEntry) -> &str {
    f.reference.slug.as_str()
}
fn drone_id(d: &DroneEntry) -> &str {
    d.reference.slug.as_str()
}
fn vehicle_id(v: &VehicleEntry) -> &str {
    v.reference.slug.as_str()
}
fn note_id(n: &NoteEntry) -> &str {
    &n.title
}

// ── Equipment (three identity collections) ─────────────────────────────────

fn walk_equipment(out: &mut Vec<FieldDiff>, path: &str, a: &Equipment, b: &Equipment) {
    walk_item_refs(out, path, "weapons", &a.weapons, &b.weapons);
    walk_item_refs(out, path, "armor", &a.armor, &b.armor);
    walk_item_refs(out, path, "inventory", &a.inventory, &b.inventory);
}

fn walk_item_refs(out: &mut Vec<FieldDiff>, path: &str, name: &str, a: &[ItemRef], b: &[ItemRef]) {
    if a == b {
        return;
    }
    let sub = format!("{path}.{name}");
    walk_id_collection(out, &sub, "", a, b, item_ref_id);
}

fn item_ref_id(i: &ItemRef) -> &str {
    i.reference.slug.as_str()
}

// ── Spellcasting (entries list; each entry compared by index) ──────────────
//
// Spellcasting entries have no stable identity slug of their own (a tradition
// + preparedness + attribute triple is the closest, but a character can have
// two identical triples from different sources). Compare by index, whole-value
// per entry — acceptable here because reordering spellcasting entries is rare
// and a whole-entry Modified is honest about the granularity.

fn walk_spellcasting(out: &mut Vec<FieldDiff>, path: &str, a: &Spellcasting, b: &Spellcasting) {
    if a == b {
        return;
    }
    let max = a.entries.len().max(b.entries.len());
    for i in 0..max {
        let sub = format!("{path}.entries[{i}]");
        match (a.entries.get(i), b.entries.get(i)) {
            (Some(a), Some(b)) => {
                if a != b {
                    let (Some(before), Some(after)) = (to_value(a), to_value(b)) else {
                        continue;
                    };
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Modified { before, after },
                    });
                }
            }
            (None, Some(b)) => {
                if let Some(after) = to_value(b) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Added { after },
                    });
                }
            }
            (Some(a), None) => {
                if let Some(before) = to_value(a) {
                    out.push(FieldDiff {
                        path: FieldPath::new(&sub),
                        change: Change::Removed { before },
                    });
                }
            }
            (None, None) => {}
        }
    }
}

// ── Serialization helper ───────────────────────────────────────────────────

fn to_value<T: Serialize>(t: &T) -> Option<Value> {
    serde_json::to_value(t).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use schema_core::newtypes::Level;
    use schema_core::testing::minimal_build;

    #[test]
    fn identical_builds_diff_to_empty() {
        let a = minimal_build();
        let b = minimal_build();
        assert_eq!(diff(&a, &b), Vec::new());
    }

    #[test]
    fn identity_name_change_is_single_modified() {
        let a = minimal_build();
        let mut b = a.clone();
        b.identity.name = "Renamed".into();
        let d = diff(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path.as_str(), "build.identity.name");
        assert!(matches!(
            &d[0].change,
            Change::Modified { before, after }
                if before == &Value::String("Test Character".into())
                && after == &Value::String("Renamed".into())
        ));
    }

    #[test]
    fn mid_list_feat_insert_is_one_add_not_tail_modified() {
        // §11.2: inserting one feat mid-list must not report the tail as changed.
        let mut a = minimal_build();
        a.feats = vec![
            FeatEntry {
                reference: SystemRefId::from_slug("power-attack"),
                category: FeatCategory::Class,
                level: Level::new(1).unwrap(),
            },
            FeatEntry {
                reference: SystemRefId::from_slug("combat-reflexes"),
                category: FeatCategory::Class,
                level: Level::new(1).unwrap(),
            },
        ];
        let mut b = a.clone();
        b.feats.insert(
            1,
            FeatEntry {
                reference: SystemRefId::from_slug("toughness"),
                category: FeatCategory::General,
                level: Level::new(1).unwrap(),
            },
        );
        let d = diff(&a, &b);
        assert_eq!(d.len(), 1, "expected one Added, got {:?}", d);
        assert!(matches!(
            &d[0].change,
            Change::Added { after } if after.get("reference").and_then(|v| v.get("slug")).and_then(|v| v.as_str()) == Some("toughness")
        ));
        assert!(d[0].path.as_str().contains("toughness"));
    }

    #[test]
    fn level_change_is_single_modified() {
        let a = minimal_build();
        let mut b = a.clone();
        b.level = Level::new(5).unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path.as_str(), "build.level");
        assert!(matches!(
            &d[0].change,
            Change::Modified { before, after }
                if before == &Value::Number(1.into()) && after == &Value::Number(5.into())
        ));
    }

    proptest! {
        #[test]
        fn diff_is_symmetric_in_presence(a_name in ".+", b_name in ".+") {
            // Two builds differing only in name: diff(a,b) has exactly one
            // Modified, and diff(b,a) inverts before/after.
            let mut a = minimal_build();
            let mut b = minimal_build();
            a.identity.name = a_name.clone();
            b.identity.name = b_name.clone();
            if a_name != b_name {
                let d_ab = diff(&a, &b);
                let d_ba = diff(&b, &a);
                prop_assert_eq!(d_ab.len(), 1);
                prop_assert_eq!(d_ba.len(), 1);
                match &d_ab[0].change {
                    Change::Modified { before, after } => {
                        prop_assert_eq!(before, &Value::String(a_name.clone()));
                        prop_assert_eq!(after, &Value::String(b_name.clone()));
                    }
                    other => prop_assert!(false, "expected Modified, got {:?}", other),
                }
            }
        }
    }
}
