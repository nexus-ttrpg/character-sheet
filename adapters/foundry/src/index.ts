// @nexus-sheet/foundry-adapter — the reference adapter (§7, §19).
//
// Foundry is the richer target: a Node.js web application run locally by the
// GM, with a real module system and a document/embedded-document model that
// maps far more naturally onto canonical than Roll20's flat attributes (§7).
//
// This adapter consumes `@nexus-sheet/foundry-mapping` for the actual
// Actor↔canonical logic — shared by plain import with the relay module that
// runs in-browser (§7.1). The adapter itself is a thin wrapper that adapts
// the mapping functions to the `CharacterAdapter` interface.

import type {
  CharacterAdapter,
  ImportOutcome,
  ExportOutcome,
  ChangeSet,
  ValidationReport,
  ImportContext,
  ExportContext,
  AdapterWarning,
} from "@nexus-sheet/adapter-sdk";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import {
  exportActor,
  importActor,
  derivedMaxHp,
  type FoundryActor,
  type FoundryItem,
  type FoundrySystemId,
} from "@nexus-sheet/foundry-mapping";

// ── Adapter capabilities (§5.4) ──────────────────────────────────────────────
//
// fieldCoverage declares the canonical paths this adapter round-trips. The
// contract tests (§16) assert every declared path survives import(export(x)).
// The UI uses this to render an honest coverage preview before a push (§5.4).

const FIELD_COVERAGE: readonly string[] = [
  "build.identity.name",
  "build.ancestry",
  "build.heritage",
  "build.background",
  "build.class",
  "build.level",
  "build.abilityScores",
  "build.skills",
  "build.proficiencies",
  "build.feats",
  "build.classFeatures",
  "build.equipment",
  "build.spellcasting",
  "build.currencies",
  "build.actions",
  "build.reactions",
  "build.explorationActivities",
  "build.notes",
  // SF2e coverage (§3.4, §2.4).
  "build.drones",
  "build.vehicles",
];

// ── The adapter ──────────────────────────────────────────────────────────────

/**
 * The Foundry reference adapter (§7, §19).
 *
 * Generic over `FoundryActor` — the platform payload type. An adapter author
 * gets full type inference over their own platform's shapes (§5.1).
 */
export const foundryAdapter: CharacterAdapter<FoundryActor> = {
  id: "foundry",

  capabilities: {
    supportsImport: true,
    supportsExport: true,
    supportsPartialExport: false,
    supportsSessionRead: false,
    fieldCoverage: FIELD_COVERAGE,
  },

  // ── import: Foundry Actor → canonical PartialBuild (§7.2) ──────────────────
  //
  // Never throws on unknown content — unmapped data goes to warnings (§5.2).
  // Homebrew items are preserved, not rejected (§7.7).

  async import(raw: FoundryActor, _ctx: ImportContext): Promise<ImportOutcome> {
    const warnings: AdapterWarning[] = [];
    const unmapped: string[] = [];

    // Detect unknown item types — they're warnings, not errors (§5.2, §7.7).
    const knownTypes = new Set<FoundryItem["type"]>([
      "feat", "classFeature", "weapon", "armor", "consumable",
      "spellcastingEntry", "spell", "action", "reaction",
      "explorationActivity", "note", "drone", "vehicle",
    ]);
    for (const item of raw.items) {
      if (!knownTypes.has(item.type)) {
        warnings.push({
          path: `items[${item.type}]`,
          kind: { type: "unmappedField" },
          detail: `Unknown item type "${item.type}" — preserved as homebrew`,
        });
      }
      // Homebrew items are flagged, not rejected (§7.7).
      if (item.flags?.["nexus-sheet"]?.homebrew) {
        warnings.push({
          path: `items.${item.system.slug ?? item.name}`,
          kind: { type: "fuzzyMatch", confidence: 0.5 },
          detail: `Homebrew item "${item.name}" — preserved inlined`,
        });
      }
    }

    // Unknown fields on the actor itself (injected by the contract test's
    // homebrew invariant check) are warnings, not errors (§5.2).
    const knownTopKeys = new Set(["name", "type", "system", "items", "flags"]);
    for (const key of Object.keys(raw)) {
      if (!knownTopKeys.has(key)) {
        warnings.push({
          path: key,
          kind: { type: "unmappedField" },
          detail: `Unknown top-level field "${key}" — ignored`,
        });
      }
    }

    const build = importActor(raw);

    return { build, warnings, unmapped };
  },

  // ── export: canonical → Foundry Actor (§7.2) ───────────────────────────────
  //
  // Max HP only — current HP is session state and is never written (§4, §7.2).

  async export(
    build: CharacterBuild,
    ctx: ExportContext,
  ): Promise<ExportOutcome<FoundryActor>> {
    const warnings: AdapterWarning[] = [];
    const skipped: string[] = [];

    const system: FoundrySystemId = ctx.system === "sf2e" ? "sf2e" : "pf2e";
    const actor = exportActor(build, system);

    return { payload: actor, warnings, skipped };
  },

  // ── detectChanges: cheap drift check (§5.3) ────────────────────────────────
  //
  // Separated from import so the host can cheaply answer "has anything changed
  // since we last synced?" without a full import/merge cycle (§5.3).

  async detectChanges(
    previous: FoundryActor | null,
    current: FoundryActor,
  ): Promise<ChangeSet> {
    if (previous === null) {
      return { hasChanges: true, changedPaths: ["*"] };
    }

    // Structural comparison: serialize both and compare. A real adapter would
    // do a field-level diff, but for the reference adapter a structural
    // comparison is sufficient and correct.
    const prevJson = JSON.stringify(previous);
    const currJson = JSON.stringify(current);

    if (prevJson === currJson) {
      return { hasChanges: false, changedPaths: [] };
    }

    // Identify which top-level fields changed.
    const changedPaths: string[] = [];
    if (previous.name !== current.name) changedPaths.push("name");
    if (JSON.stringify(previous.system) !== JSON.stringify(current.system)) {
      changedPaths.push("system");
    }
    if (JSON.stringify(previous.items) !== JSON.stringify(current.items)) {
      changedPaths.push("items");
    }

    return { hasChanges: true, changedPaths };
  },

  // ── validate: structural + platform-specific validation (§7.6) ─────────────
  //
  // In the real relay module, this would hand the constructed `system` object
  // to the running world's PF2e/SF2e Actor schema and return native validation
  // errors (§7.6). Here we do structural validation only.

  async validate(raw: FoundryActor): Promise<ValidationReport> {
    const errors: string[] = [];
    const warnings: string[] = [];

    if (!raw || typeof raw !== "object") {
      return { valid: false, errors: ["not an object"], warnings };
    }

    if (raw.type !== "character") {
      errors.push(`expected actor type "character", got "${raw.type}"`);
    }

    if (!raw.name) {
      errors.push("missing required field: name");
    }

    if (!raw.system) {
      errors.push("missing required field: system");
      return { valid: false, errors, warnings };
    }

    if (!raw.system.details?.level?.value) {
      errors.push("missing required field: system.details.level.value");
    }

    if (!raw.system.details?.class?.keyAbility) {
      errors.push("missing required field: system.details.class.keyAbility");
    }

    if (!raw.system.abilities) {
      warnings.push("no ability scores present");
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  },
};

// Re-export the max HP derivation for the relay module (§7.2).
export { derivedMaxHp };

// Re-export platform types for adapter consumers.
export type { FoundryActor, FoundryItem } from "@nexus-sheet/foundry-mapping";
