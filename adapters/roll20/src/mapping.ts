// Roll20 PF2e sheet mapping table (§6.3).
//
// Declarative data, not imperative field-by-field code. A Roll20 sheet author
// renaming an attribute is a one-line edit here — the most common maintenance
// task on this adapter, now a change any contributor can make and test with no
// Rust toolchain (§6.3).
//
// `canonicalPath()` makes a typo in a path a compile-time type error, not a
// silently-skipped field (§6.3, §3.6). `fieldCoverage` is derived from the
// table's paths (§6.3 point 2), so it can't drift out of date.

import type { CharacterBuild, AbilityScoreKey } from "@nexus-sheet/schema-types";
import { canonicalPath, type CanonicalPath } from "@nexus-sheet/adapter-sdk";

// ── Roll20 sheet payload types ───────────────────────────────────────────────
//
// The collector (collector/nexus-collector.js) reads Roll20's Attribute objects
// verbatim and POSTs them as this shape. All values are strings — Roll20
// attributes are stored as strings regardless of underlying type.

/** A row in a repeating section, keyed by generated row ID (§6.4). */
export type RepeatingSection = Record<string, Record<string, string>>;

/**
 * The raw Roll20 sheet dump the collector sends (§6.1).
 *
 * `sheet_version` is always included by the collector; `validate()` checks it
 * and returns `UnsupportedSheetVersion` rather than attempting a best-effort
 * mapping that would silently corrupt a sheet (§6.4).
 */
export interface Roll20Sheet {
  sheet_version: string;
  character_id: string;
  /** Flat attributes: `{ character_name: "Bob", level: "5", ... }`. */
  attributes: Record<string, string>;
  /** Repeating sections: `{ repeating_skills: { rowId: { skill_name: "...", ... } } }`. */
  repeating: Record<string, RepeatingSection>;
}

// ── Mapping table types (§6.3) ───────────────────────────────────────────────

/** A row produced by a repeating-section mapping function. */
export interface RepeatingRow {
  /** Content fingerprint for matching on import — name + source, not row id (§6.4). */
  fingerprint: string;
  /** Attribute name→value pairs for this row. */
  attrs: Record<string, string>;
}

/** A single field mapping: canonical path → Roll20 target. */
export type FieldMapping = {
  path: CanonicalPath;
  target:
    | { kind: "attr"; name: string }
    | { kind: "repeating"; section: string; rows: (b: CharacterBuild) => RepeatingRow[] };
};

// ── Ability score attribute names ────────────────────────────────────────────
//
// Roll20 PF2e sheet uses `{ability}_score` for the final value. The canonical
// model stores components (boosts/flaws/itemBonus); Roll20 stores only the
// derived number, so ability scores are a lossy round-trip: the *value*
// survives, the *components* do not (§5.2 lossyExport).

const ABILITY_ATTR: Record<AbilityScoreKey, string> = {
  str: "strength_score",
  dex: "dexterity_score",
  con: "constitution_score",
  int: "intelligence_score",
  wis: "wisdom_score",
  cha: "charisma_score",
};

// ── Repeating-section row builders ───────────────────────────────────────────

function featRows(b: CharacterBuild): RepeatingRow[] {
  return b.feats.map((f) => ({
    fingerprint: `${f.reference.slug}|${f.reference.source ?? ""}`,
    attrs: {
      feat_name: f.reference.slug,
      feat_category: f.category,
      feat_level: String(f.level),
      ...(f.reference.source ? { feat_source: f.reference.source } : {}),
    },
  }));
}

function skillRows(b: CharacterBuild): RepeatingRow[] {
  return Object.entries(b.skills).map(([key, s]) => ({
    fingerprint: key,
    attrs: {
      skill_name: key,
      skill_rank: s.rank,
      skill_item_bonus: String(s.itemBonus),
      ...(s.lore ? { skill_lore: s.lore } : {}),
    },
  }));
}

// ── The mapping table (§6.3) ─────────────────────────────────────────────────

export const ROLL20_PF2E_MAP = [
  { path: canonicalPath("build.identity.name"), target: { kind: "attr", name: "character_name" } },
  { path: canonicalPath("build.level"), target: { kind: "attr", name: "level" } },
  { path: canonicalPath("build.ancestry.reference.slug"), target: { kind: "attr", name: "ancestry" } },
  { path: canonicalPath("build.heritage.reference.slug"), target: { kind: "attr", name: "heritage" } },
  { path: canonicalPath("build.background.reference.slug"), target: { kind: "attr", name: "background" } },
  { path: canonicalPath("build.class.reference.slug"), target: { kind: "attr", name: "class" } },
  { path: canonicalPath("build.class.keyAbility"), target: { kind: "attr", name: "key_ability_score" } },
  { path: canonicalPath("build.class.hpPerLevel"), target: { kind: "attr", name: "class_hp_per_level" } },
  // Ability scores: lossy (value only; components not reconstructable from a flat number).
  { path: canonicalPath("build.abilityScores.str"), target: { kind: "attr", name: ABILITY_ATTR.str } },
  { path: canonicalPath("build.abilityScores.dex"), target: { kind: "attr", name: ABILITY_ATTR.dex } },
  { path: canonicalPath("build.abilityScores.con"), target: { kind: "attr", name: ABILITY_ATTR.con } },
  { path: canonicalPath("build.abilityScores.int"), target: { kind: "attr", name: ABILITY_ATTR.int } },
  { path: canonicalPath("build.abilityScores.wis"), target: { kind: "attr", name: ABILITY_ATTR.wis } },
  { path: canonicalPath("build.abilityScores.cha"), target: { kind: "attr", name: ABILITY_ATTR.cha } },
  // Repeating sections: matched by content fingerprint on import (§6.4).
  { path: canonicalPath("build.feats"), target: { kind: "repeating", section: "repeating_feats", rows: featRows } },
  { path: canonicalPath("build.skills"), target: { kind: "repeating", section: "repeating_skills", rows: skillRows } },
] as const satisfies readonly FieldMapping[];

// ── Derived fieldCoverage (§6.3 point 2) ─────────────────────────────────────
//
// Derived from the table rather than hand-maintained, so it can't silently
// drift out of date. The contract tests (§16) backstop by asserting every
// declared path round-trips on a real fixture.

/** Canonical paths this adapter round-trips, derived from ROLL20_PF2E_MAP. */
export const ROLL20_FIELD_COVERAGE: readonly string[] = ROLL20_PF2E_MAP.map((m) => m.path);

// ── Supported sheet versions (§6.4) ──────────────────────────────────────────

/** Sheet versions the adapter's mapping table is known to work against. */
export const SUPPORTED_SHEET_VERSIONS = ["1.0", "1.1", "2.0"] as const;
