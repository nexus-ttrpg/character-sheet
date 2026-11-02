// Pathbuilder 2e ↔ canonical mapping (§8, §3.3).
//
// FORMAT FIDELITY CAVEAT — read this before relying on live imports.
// The field names below are a *representative* Pathbuilder JSON shape modelled
// on §8 ("JSON is already close to canonical's shape, so mapping is mostly
// renaming") and §3.3 (ability scores carry components, not final numbers).
// They are NOT verified against a real Pathbuilder 2e export. The contract
// tests validate import/export *symmetry* (round-trip), not real-format
// fidelity. Before importing live Pathbuilder data, remap these field names
// against a real export; the canary job (§3.3) is the mechanism that catches
// real-format drift.
//
// §3.3 headline: Pathbuilder recomputes the final ability number itself, and
// because canonical stores COMPONENTS (boosts/flaws/itemBonus), the round-trip
// is NON-lossy here — unlike Roll20, which stores only the derived number and
// loses the components (§5.2 lossyExport).

import type {
  CharacterBuild,
  AbilityScoreKey,
  AbilityBoostSource,
  ProficiencyRank,
  FeatCategory,
} from "@nexus-ttrpg/schema-types";
import { canonicalPath, type PartialBuild } from "@nexus-ttrpg/adapter-sdk";

// ── Payload types ────────────────────────────────────────────────────────────
//
// A compact, slug-oriented shape: ancestry/heritage/background/class are flat
// slug strings (canonical wraps them in { reference: { slug, source? } }). The
// rulebook `source` is preserved on feats/classFeatures/items where Pathbuilder
// tracks it, and dropped on the four lineage slugs (builders rarely carry the
// rulebook source there) — a documented sub-field loss, not a field-level one.

export interface PathbuilderFeat {
  /** Canonical `reference.slug`. */
  name: string;
  level: number;
  category: FeatCategory;
  /** Canonical `reference.source`, when present. */
  source?: string;
}

export interface PathbuilderClassFeature {
  name: string;
  level: number;
  source?: string;
}

export interface PathbuilderItem {
  name: string;
  quantity: number;
  source?: string;
}

export interface PathbuilderEquipment {
  weapons: PathbuilderItem[];
  armor: PathbuilderItem[];
  inventory: PathbuilderItem[];
}

export interface PathbuilderSkill {
  rank: ProficiencyRank;
  itemBonus: number;
  lore?: string;
}

export interface PathbuilderCurrencies {
  gp?: number;
  sp?: number;
  cp?: number;
  pp?: number;
}

/**
 * The raw Pathbuilder 2e export this adapter maps to/from (§8).
 *
 * `schemaVersion` is always present; `validate()` checks it and throws
 * `AdapterError({ unsupportedSheetVersion })` rather than attempting a
 * best-effort mapping that would silently corrupt a character (§6.4).
 */
export interface PathbuilderCharacter {
  schemaVersion: number;
  name: string;
  level: number;
  ancestry: string;
  heritage: string;
  background: string;
  class: string;
  keyAbility: AbilityScoreKey;
  hpPerLevel: number;
  /** Ability boosts as COMPONENTS (§3.3) — keyed by ability. */
  abilityBoosts: Partial<Record<AbilityScoreKey, AbilityBoostSource[]>>;
  abilityFlaws: Partial<Record<AbilityScoreKey, AbilityBoostSource[]>>;
  abilityItemBonus: Partial<Record<AbilityScoreKey, number>>;
  skills: Record<string, PathbuilderSkill>;
  proficiencies: Record<string, ProficiencyRank>;
  feats: PathbuilderFeat[];
  classFeatures: PathbuilderClassFeature[];
  equipment: PathbuilderEquipment;
  currencies: PathbuilderCurrencies;
}

// ── Supported schema versions (§6.4) ─────────────────────────────────────────

/** Export schema versions the adapter's mapping is known to work against. */
export const SUPPORTED_SCHEMA_VERSIONS: readonly number[] = [1, 2];

// ── Known top-level keys (unknown-content detection, §5.2) ───────────────────

export const PATHBUILDER_TOP_KEYS = new Set<string>([
  "schemaVersion", "name", "level", "ancestry", "heritage", "background",
  "class", "keyAbility", "hpPerLevel", "abilityBoosts", "abilityFlaws",
  "abilityItemBonus", "skills", "proficiencies", "feats", "classFeatures",
  "equipment", "currencies",
]);

// ── fieldCoverage (§5.4) ─────────────────────────────────────────────────────
//
// Declared at the granularity each field actually round-trips. Lineage slugs
// are declared at the `.reference.slug` sub-path (source is dropped there);
// ability scores are declared per-ability because their COMPONENTS round-trip
// non-lossily (§3.3) — the contract tests backstop presence, and the
// pathbuilder-specific tests assert the component-level equality.

export const PATHBUILDER_FIELD_COVERAGE = [
  canonicalPath("build.identity.name"),
  canonicalPath("build.level"),
  canonicalPath("build.ancestry.reference.slug"),
  canonicalPath("build.heritage.reference.slug"),
  canonicalPath("build.background.reference.slug"),
  canonicalPath("build.class.reference.slug"),
  canonicalPath("build.class.keyAbility"),
  canonicalPath("build.class.hpPerLevel"),
  canonicalPath("build.abilityScores.str"),
  canonicalPath("build.abilityScores.dex"),
  canonicalPath("build.abilityScores.con"),
  canonicalPath("build.abilityScores.int"),
  canonicalPath("build.abilityScores.wis"),
  canonicalPath("build.abilityScores.cha"),
  canonicalPath("build.skills"),
  canonicalPath("build.proficiencies"),
  canonicalPath("build.feats"),
  canonicalPath("build.classFeatures"),
  canonicalPath("build.equipment"),
  canonicalPath("build.currencies"),
] as const satisfies readonly string[];

// ── canonical → Pathbuilder ──────────────────────────────────────────────────

/** Map a canonical `CharacterBuild` to a Pathbuilder export payload. */
export function canonicalToPathbuilder(build: CharacterBuild): PathbuilderCharacter {
  const abilityBoosts: Partial<Record<AbilityScoreKey, AbilityBoostSource[]>> = {};
  const abilityFlaws: Partial<Record<AbilityScoreKey, AbilityBoostSource[]>> = {};
  const abilityItemBonus: Partial<Record<AbilityScoreKey, number>> = {};

  for (const [key, entry] of Object.entries(build.abilityScores)) {
    const k = key as AbilityScoreKey;
    abilityBoosts[k] = entry.boosts.map((b) => b.source);
    abilityFlaws[k] = entry.flaws.map((b) => b.source);
    abilityItemBonus[k] = entry.itemBonus;
  }

  const skills: Record<string, PathbuilderSkill> = {};
  for (const [key, s] of Object.entries(build.skills)) {
    skills[key] = {
      rank: s.rank,
      itemBonus: s.itemBonus,
      ...(s.lore ? { lore: s.lore } : {}),
    };
  }

  const exportItems = (
    items: CharacterBuild["equipment"]["weapons"],
  ): PathbuilderItem[] =>
    items.map((it) => ({
      name: it.reference.slug,
      quantity: it.quantity,
      ...(it.reference.source ? { source: it.reference.source } : {}),
    }));

  return {
    schemaVersion: SUPPORTED_SCHEMA_VERSIONS[SUPPORTED_SCHEMA_VERSIONS.length - 1]!,
    name: build.identity.name,
    level: build.level,
    ancestry: build.ancestry.reference.slug,
    heritage: build.heritage.reference.slug,
    background: build.background.reference.slug,
    class: build.class.reference.slug,
    keyAbility: build.class.keyAbility,
    hpPerLevel: build.class.hpPerLevel,
    abilityBoosts,
    abilityFlaws,
    abilityItemBonus,
    skills,
    proficiencies: { ...build.proficiencies },
    feats: build.feats.map((f) => ({
      name: f.reference.slug,
      level: f.level,
      category: f.category,
      ...(f.reference.source ? { source: f.reference.source } : {}),
    })),
    classFeatures: build.classFeatures.map((cf) => ({
      name: cf.reference.slug,
      level: cf.level,
      ...(cf.reference.source ? { source: cf.reference.source } : {}),
    })),
    equipment: {
      weapons: exportItems(build.equipment.weapons),
      armor: exportItems(build.equipment.armor),
      inventory: exportItems(build.equipment.inventory),
    },
    currencies: {
      ...(build.currencies.gp != null ? { gp: build.currencies.gp } : {}),
      ...(build.currencies.sp != null ? { sp: build.currencies.sp } : {}),
      ...(build.currencies.cp != null ? { cp: build.currencies.cp } : {}),
      ...(build.currencies.pp != null ? { pp: build.currencies.pp } : {}),
    },
  };
}

// ── Pathbuilder → canonical ──────────────────────────────────────────────────

/**
 * Map a Pathbuilder export payload to a canonical `PartialBuild`.
 *
 * Assumes a structurally well-formed payload (the adapter's `validate()` gates
 * this). Ability-score components are preserved verbatim — non-lossy (§3.3).
 */
export function pathbuilderToCanonical(raw: PathbuilderCharacter): PartialBuild {
  const build: PartialBuild = {};

  build.identity = { name: raw.name };
  build.level = raw.level;
  build.ancestry = { reference: { slug: raw.ancestry } };
  build.heritage = { reference: { slug: raw.heritage } };
  build.background = { reference: { slug: raw.background } };
  build.class = {
    reference: { slug: raw.class },
    keyAbility: raw.keyAbility,
    hpPerLevel: raw.hpPerLevel,
  };

  // Ability scores: reconstruct from components. A key may appear in any of the
  // three component maps, so union them. Non-lossy (§3.3).
  const abilityScores: CharacterBuild["abilityScores"] = {};
  const abilityKeys = new Set<string>([
    ...Object.keys(raw.abilityBoosts),
    ...Object.keys(raw.abilityFlaws),
    ...Object.keys(raw.abilityItemBonus),
  ]);
  for (const key of abilityKeys) {
    const k = key as AbilityScoreKey;
    const boosts = raw.abilityBoosts[k] ?? [];
    const flaws = raw.abilityFlaws[k] ?? [];
    const itemBonus = raw.abilityItemBonus[k] ?? 0;
    abilityScores[k] = {
      boosts: boosts.map((s) => ({ source: s })),
      flaws: flaws.map((s) => ({ source: s })),
      itemBonus,
    };
  }
  build.abilityScores = abilityScores;

  const skills: CharacterBuild["skills"] = {};
  for (const [key, s] of Object.entries(raw.skills)) {
    skills[key] = {
      rank: s.rank,
      itemBonus: s.itemBonus,
      ...(s.lore ? { lore: s.lore } : {}),
    };
  }
  build.skills = skills;

  build.proficiencies = { ...raw.proficiencies };

  build.feats = raw.feats.map((f) => ({
    reference: { slug: f.name, ...(f.source ? { source: f.source } : {}) },
    category: f.category,
    level: f.level,
  }));

  build.classFeatures = raw.classFeatures.map((cf) => ({
    reference: { slug: cf.name, ...(cf.source ? { source: cf.source } : {}) },
    level: cf.level,
  }));

  const importItems = (items: PathbuilderItem[]): CharacterBuild["equipment"]["weapons"] =>
    items.map((it) => ({
      reference: { slug: it.name, ...(it.source ? { source: it.source } : {}) },
      quantity: it.quantity,
    }));
  build.equipment = {
    weapons: importItems(raw.equipment.weapons),
    armor: importItems(raw.equipment.armor),
    inventory: importItems(raw.equipment.inventory),
  };

  const currencies: CharacterBuild["currencies"] = {};
  if (raw.currencies.gp != null) currencies.gp = raw.currencies.gp;
  if (raw.currencies.sp != null) currencies.sp = raw.currencies.sp;
  if (raw.currencies.cp != null) currencies.cp = raw.currencies.cp;
  if (raw.currencies.pp != null) currencies.pp = raw.currencies.pp;
  build.currencies = currencies;

  return build;
}
