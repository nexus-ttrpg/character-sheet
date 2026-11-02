// @nexus-ttrpg/roll20-adapter — the structurally harder second adapter (§6, §21).
//
// Roll20 sheets store data as flat Attribute rows keyed by name
// (`strength_score`, `repeating_skills_-abc123_skill_name`), plus repeating
// sections addressed by generated row IDs — very different from canonical's
// nested graph. The mapping is declarative data (ROLL20_PF2E_MAP, §6.3), not
// imperative field-by-field code.
//
// The collector (collector/nexus-collector.js) runs inside Roll20's sandbox and
// is deliberately logic-free: it reads attrs verbatim, POSTs them signed, and
// writes attrs verbatim on apply. All mapping logic lives here, server-side,
// so a mapping fix ships instantly without every GM re-pasting the collector
// (§6.1).

import type {
  CharacterAdapter,
  ImportOutcome,
  ExportOutcome,
  ChangeSet,
  ValidationReport,
  ImportContext,
  ExportContext,
  AdapterWarning,
} from "@nexus-ttrpg/adapter-sdk";
import { AdapterError } from "@nexus-ttrpg/adapter-sdk";
import type {
  CharacterBuild,
  AbilityScoreKey,
  AbilityScoreEntry,
  ProficiencyRank,
} from "@nexus-ttrpg/schema-types";
import {
  ROLL20_PF2E_MAP,
  ROLL20_FIELD_COVERAGE,
  SUPPORTED_SHEET_VERSIONS,
  type Roll20Sheet,
  type RepeatingRow,
} from "./mapping.js";

// ── Helpers ──────────────────────────────────────────────────────────────────

const ABILITY_KEYS: readonly AbilityScoreKey[] = ["str", "dex", "con", "int", "wis", "cha"];

/**
 * Derive the final ability score value from its components (§3.3).
 * PF2e: start at 10, each boost +2, each flaw -2, plus item bonus.
 * (Same derivation as foundry-mapping; duplicated here to avoid coupling the
 * two adapters — Roll20 and Foundry are independently disposable, §6.1.)
 */
function derivedAbilityValue(entry: AbilityScoreEntry): number {
  return 10 + 2 * entry.boosts.length - 2 * entry.flaws.length + entry.itemBonus;
}

/** Parse a string attr as a number, defaulting if absent/invalid. */
function parseNum(s: string | undefined, fallback: number): number {
  if (s == null || s === "") return fallback;
  const n = Number(s);
  return Number.isNaN(n) ? fallback : n;
}

// ── Export: canonical → Roll20 sheet (§6.3) ──────────────────────────────────

function exportFlatAttrs(build: CharacterBuild): Record<string, string> {
  const attrs: Record<string, string> = {};

  for (const mapping of ROLL20_PF2E_MAP) {
    if (mapping.target.kind !== "attr") continue;
    const value = getField(build, mapping.path);
    if (value == null) continue;

    // Ability scores: derive the final number (lossy — components dropped).
    if (typeof value === "object" && "boosts" in value) {
      attrs[mapping.target.name] = String(derivedAbilityValue(value as AbilityScoreEntry));
      continue;
    }
    attrs[mapping.target.name] = String(value);
  }
  return attrs;
}

function exportRepeating(build: CharacterBuild): Record<string, Record<string, Record<string, string>>> {
  const sections: Record<string, Record<string, Record<string, string>>> = {};

  for (const mapping of ROLL20_PF2E_MAP) {
    if (mapping.target.kind !== "repeating") continue;
    const rows = mapping.target.rows(build);
    const sectionData: Record<string, Record<string, string>> = {};
    for (let i = 0; i < rows.length; i++) {
      // Use the fingerprint as a stable row key — Roll20 regenerates row IDs on
      // some sheet operations, so we key by content, not by Roll20's row id (§6.4).
      sectionData[rows[i]!.fingerprint] = rows[i]!.attrs;
    }
    sections[mapping.target.section] = sectionData;
  }
  return sections;
}

// ── Import: Roll20 sheet → canonical PartialBuild (§6.3) ─────────────────────

function importFlatAttrs(
  attrs: Record<string, string>,
  warnings: AdapterWarning[],
): Partial<CharacterBuild> {
  const build: Partial<CharacterBuild> = {};
  const abilityScores: Partial<Record<AbilityScoreKey, AbilityScoreEntry>> = {};

  for (const mapping of ROLL20_PF2E_MAP) {
    if (mapping.target.kind !== "attr") continue;
    const raw = attrs[mapping.target.name];
    if (raw == null) continue;

    switch (mapping.path) {
      case "build.identity.name":
        build.identity = { name: raw };
        break;
      case "build.level":
        build.level = parseNum(raw, 1);
        break;
      case "build.ancestry.reference.slug":
        build.ancestry = { reference: { slug: raw } };
        break;
      case "build.heritage.reference.slug":
        build.heritage = { reference: { slug: raw } };
        break;
      case "build.background.reference.slug":
        build.background = { reference: { slug: raw } };
        break;
      case "build.class.reference.slug":
        build.class = { reference: { slug: raw }, keyAbility: "str", hpPerLevel: 8 };
        break;
      case "build.class.keyAbility":
        if (build.class) {
          build.class.keyAbility = raw as AbilityScoreKey;
        } else {
          build.class = { reference: { slug: "" }, keyAbility: raw as AbilityScoreKey, hpPerLevel: 8 };
        }
        break;
      case "build.class.hpPerLevel":
        if (build.class) {
          build.class.hpPerLevel = parseNum(raw, 8);
        } else {
          build.class = { reference: { slug: "" }, keyAbility: "str", hpPerLevel: parseNum(raw, 8) };
        }
        break;
      // Ability scores: reconstruct from the flat number. The *value* round-trips
      // (derivedAbilityValue gives back the same number) but the *components*
      // (boosts/flaws) are lost — stored as itemBonus = value - 10 (§5.2 lossy).
      case "build.abilityScores.str":
      case "build.abilityScores.dex":
      case "build.abilityScores.con":
      case "build.abilityScores.int":
      case "build.abilityScores.wis":
      case "build.abilityScores.cha": {
        const key = mapping.path.split(".").pop() as AbilityScoreKey;
        const score = parseNum(raw, 10);
        abilityScores[key] = {
          boosts: [],
          flaws: [],
          itemBonus: score - 10,
        };
        warnings.push({
          path: `build.abilityScores.${key}`,
          kind: { type: "lossyExport", reason: "Roll20 stores the derived score only; boosts/flaws not reconstructable" },
          detail: `${key} score ${score} imported as itemBonus=${score - 10} (components lost)`,
        });
        break;
      }
    }
  }

  if (Object.keys(abilityScores).length > 0) {
    build.abilityScores = abilityScores as CharacterBuild["abilityScores"];
  }
  return build;
}

function importRepeating(
  sections: Record<string, Record<string, Record<string, string>>>,
): Pick<CharacterBuild, "feats" | "skills"> {
  const feats: CharacterBuild["feats"] = [];
  const skills: CharacterBuild["skills"] = {};

  for (const mapping of ROLL20_PF2E_MAP) {
    if (mapping.target.kind !== "repeating") continue;
    const section = sections[mapping.target.section];
    if (!section) continue;

    if (mapping.target.section === "repeating_feats") {
      for (const row of Object.values(section)) {
        const slug = row.feat_name ?? "";
        if (!slug) continue;
        feats.push({
          reference: {
            slug,
            ...(row.feat_source ? { source: row.feat_source } : {}),
          },
          category: (row.feat_category ?? "general") as CharacterBuild["feats"][number]["category"],
          level: parseNum(row.feat_level, 1),
        });
      }
    }

    if (mapping.target.section === "repeating_skills") {
      for (const row of Object.values(section)) {
        const key = row.skill_name ?? "";
        if (!key) continue;
        skills[key] = {
          rank: (row.skill_rank ?? "untrained") as ProficiencyRank,
          itemBonus: parseNum(row.skill_item_bonus, 0),
          ...(row.skill_lore ? { lore: row.skill_lore } : {}),
        };
      }
    }
  }

  return { feats, skills };
}

// ── The adapter ──────────────────────────────────────────────────────────────

export const roll20Adapter: CharacterAdapter<Roll20Sheet> = {
  id: "roll20",

  capabilities: {
    supportsImport: true,
    supportsExport: true,
    supportsPartialExport: false,
    // Roll20's sandbox is unreachable from outside and frequently asleep (§6.2).
    // Session state (HP, conditions) is scene-scoped and never synced (§4).
    supportsSessionRead: false,
    fieldCoverage: ROLL20_FIELD_COVERAGE,
  },

  // ── import: Roll20 sheet → canonical (§6.3) ────────────────────────────────
  //
  // Never throws on unknown content — unmapped attrs are silently ignored,
  // lossy imports produce warnings (§5.2). Repeating rows matched by content
  // fingerprint, not row id (§6.4).

  async import(raw: Roll20Sheet, _ctx: ImportContext): Promise<ImportOutcome> {
    const warnings: AdapterWarning[] = [];
    const unmapped: string[] = [];

    const flat = importFlatAttrs(raw.attributes, warnings);
    const repeating = importRepeating(raw.repeating);

    const build = { ...flat, ...repeating } as ImportOutcome["build"];

    return { build, warnings, unmapped };
  },

  // ── export: canonical → Roll20 sheet (§6.3) ────────────────────────────────

  async export(
    build: CharacterBuild,
    _ctx: ExportContext,
  ): Promise<ExportOutcome<Roll20Sheet>> {
    const warnings: AdapterWarning[] = [];
    const skipped: string[] = [];

    const attrs = exportFlatAttrs(build);
    const repeating = exportRepeating(build);

    // Ability scores are lossy on this adapter (§5.2).
    for (const key of ABILITY_KEYS) {
      if (build.abilityScores[key]) {
        warnings.push({
          path: `build.abilityScores.${key}`,
          kind: { type: "lossyExport", reason: "Roll20 receives the derived score only; boosts/flaws dropped" },
          detail: `${key} exported as flat number`,
        });
      }
    }

    const payload: Roll20Sheet = {
      sheet_version: SUPPORTED_SHEET_VERSIONS[SUPPORTED_SHEET_VERSIONS.length - 1]!,
      character_id: "",
      attributes: attrs,
      repeating,
    };

    return { payload, warnings, skipped };
  },

  // ── detectChanges: structural drift check (§5.3) ───────────────────────────
  //
  // Roll20's sandbox is usually not running, so the API's `drift` endpoint
  // returns `Unavailable { reason: SandboxNotRunning }` for Roll20 connections
  // rather than calling this (§6.2, connections.rs). This method exists for
  // completeness and is used only when the collector has sent a fresh dump.

  async detectChanges(
    previous: Roll20Sheet | null,
    current: Roll20Sheet,
  ): Promise<ChangeSet> {
    if (previous === null) {
      return { hasChanges: true, changedPaths: ["*"] };
    }

    const prevJson = JSON.stringify(previous);
    const currJson = JSON.stringify(current);

    if (prevJson === currJson) {
      return { hasChanges: false, changedPaths: [] };
    }

    const changedPaths: string[] = [];
    if (JSON.stringify(previous.attributes) !== JSON.stringify(current.attributes)) {
      changedPaths.push("attributes");
    }
    if (JSON.stringify(previous.repeating) !== JSON.stringify(current.repeating)) {
      changedPaths.push("repeating");
    }

    return { hasChanges: true, changedPaths };
  },

  // ── validate: sheet_version check (§6.4) ───────────────────────────────────
  //
  // Checks the `sheet_version` attribute the collector always includes. Returns
  // `UnsupportedSheetVersion` rather than attempting a best-effort mapping that
  // would silently corrupt a sheet (§6.4).

  async validate(raw: Roll20Sheet): Promise<ValidationReport> {
    const errors: string[] = [];
    const warnings: string[] = [];

    if (!raw || typeof raw !== "object") {
      return { valid: false, errors: ["not an object"], warnings };
    }

    const version = raw.sheet_version;
    if (!version) {
      errors.push("missing required field: sheet_version");
    } else if (!SUPPORTED_SHEET_VERSIONS.includes(version as typeof SUPPORTED_SHEET_VERSIONS[number])) {
      throw new AdapterError(
        { type: "unsupportedSheetVersion", found: version, supported: [...SUPPORTED_SHEET_VERSIONS] },
        `sheet_version "${version}" not in supported list [${SUPPORTED_SHEET_VERSIONS.join(", ")}]`,
      );
    }

    if (!raw.attributes || typeof raw.attributes !== "object") {
      errors.push("missing required field: attributes");
    }

    if (!raw.attributes?.character_name) {
      warnings.push("no character_name attribute present");
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  },
};

// Re-export mapping types for adapter consumers.
export type { Roll20Sheet, RepeatingRow, FieldMapping } from "./mapping.js";
export { ROLL20_PF2E_MAP, ROLL20_FIELD_COVERAGE, SUPPORTED_SHEET_VERSIONS } from "./mapping.js";

// ── Helper (internal) ────────────────────────────────────────────────────────

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function getField(obj: any, path: string): unknown {
  const parts = path.split(".");
  let current: unknown = obj;
  if (parts[0] === "build") parts.shift();
  for (const part of parts) {
    if (current == null || typeof current !== "object") return undefined;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    current = (current as any)[part];
  }
  return current;
}
