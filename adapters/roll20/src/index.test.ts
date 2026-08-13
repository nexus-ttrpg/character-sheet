// Roll20 adapter contract + specific tests (§6, §16).
//
//   describe("roll20", () => runContractTests(adapter, fixtures));
//
// The contract harness gives the second adapter the same correctness bar as
// Foundry for free — no Rust in the loop (§16). The Roll20-specific tests
// below cover the structurally harder parts: sheet_version validation (§6.4),
// lossy ability scores (§5.2), repeating-row fingerprint matching (§6.4), and
// the asleep-sandbox capability declaration (§6.2).

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runContractTests, type AdapterFixture } from "@nexus-sheet/adapter-sdk/testing";
import { AdapterError } from "@nexus-sheet/adapter-sdk";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import { roll20Adapter } from "./index.js";
import type { Roll20Sheet } from "./mapping.js";
import { SUPPORTED_SHEET_VERSIONS, ROLL20_FIELD_COVERAGE } from "./mapping.js";

// ── Load PF2e fixtures (shared ground-truth, §16) ──────────────────────────

function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}

const fighter = loadFixture("pf2e-level1-fighter.json");
const ranger = loadFixture("pf2e-level5-ranger.json");
const wizard = loadFixture("pf2e-level10-wizard.json");

// ── Build AdapterFixtures: export each canonical build to a Roll20Sheet ─────
//
// The adapter's own export() is the source of truth for the platform format,
// so we use it to produce platformPayloads (same pattern as Foundry, which
// uses exportActor — the mapping function the adapter wraps).

async function makeFixture(name: string, build: CharacterBuild): Promise<AdapterFixture<Roll20Sheet>> {
  const exportResult = await roll20Adapter.export(build, { system: "pf2e", credentials: {} });
  return {
    name,
    build,
    platformPayload: exportResult.payload,
    badPayload: {
      sheet_version: "",
      character_id: "",
      attributes: {},
      repeating: {},
    },
  };
}

const fixtures = await Promise.all([
  makeFixture("level1-fighter", fighter),
  makeFixture("level5-ranger", ranger),
  makeFixture("level10-wizard", wizard),
]);

// ── Run the standard contract suite (§16) ──────────────────────────────────

describe("roll20 adapter contract", () => {
  runContractTests(roll20Adapter, fixtures);
});

// ── Roll20-specific: capabilities (§5.4, §6.2) ──────────────────────────────

describe("roll20 capabilities", () => {
  it("does not support session read (§4, §6.2 — sandbox usually asleep)", () => {
    expect(roll20Adapter.capabilities.supportsSessionRead).toBe(false);
  });

  it("supports import and export", () => {
    expect(roll20Adapter.capabilities.supportsImport).toBe(true);
    expect(roll20Adapter.capabilities.supportsExport).toBe(true);
  });

  it("fieldCoverage is non-empty and derived from the mapping table (§6.3)", () => {
    expect(ROLL20_FIELD_COVERAGE.length).toBeGreaterThan(0);
    expect(roll20Adapter.capabilities.fieldCoverage).toBe(ROLL20_FIELD_COVERAGE);
    expect(roll20Adapter.capabilities.fieldCoverage).toContain("build.identity.name");
    expect(roll20Adapter.capabilities.fieldCoverage).toContain("build.abilityScores.str");
    expect(roll20Adapter.capabilities.fieldCoverage).toContain("build.feats");
    expect(roll20Adapter.capabilities.fieldCoverage).toContain("build.skills");
  });

  it("adapter id is roll20", () => {
    expect(roll20Adapter.id).toBe("roll20");
  });
});

// ── Roll20-specific: export (§6.3) ──────────────────────────────────────────

describe("roll20 export", () => {
  it("maps identity.name to character_name attr", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    expect(result.payload.attributes.character_name).toBe(fighter.identity.name);
  });

  it("maps class fields to attrs", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    expect(result.payload.attributes.class).toBe("fighter");
    expect(result.payload.attributes.key_ability_score).toBe("str");
    expect(result.payload.attributes.class_hp_per_level).toBe("10");
  });

  it("maps ancestry/heritage/background to attrs", async () => {
    const result = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    expect(result.payload.attributes.ancestry).toBe("elf");
    expect(result.payload.attributes.heritage).toBe("woodland-elf");
    expect(result.payload.attributes.background).toBe("hunter");
  });

  it("maps ability scores to {ability}_score attrs (derived value)", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    // Fighter str: 10 + 2*3 boosts = 16
    expect(result.payload.attributes.strength_score).toBe("16");
    // Fighter dex: 10 + 2*1 boost = 12
    expect(result.payload.attributes.dexterity_score).toBe("12");
  });

  it("emits lossyExport warnings for ability scores (§5.2)", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const lossyWarnings = result.warnings.filter((w) => w.kind.type === "lossyExport");
    // Fighter has 4 ability scores
    expect(lossyWarnings.length).toBe(4);
    expect(lossyWarnings.some((w) => w.path === "build.abilityScores.str")).toBe(true);
  });

  it("exports feats as repeating section rows", async () => {
    const result = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const featSection = result.payload.repeating.repeating_feats;
    expect(featSection).toBeDefined();
    expect(Object.keys(featSection)).toHaveLength(ranger.feats.length);
  });

  it("exports skills as repeating section rows including lore", async () => {
    const result = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const skillSection = result.payload.repeating.repeating_skills;
    expect(Object.keys(skillSection)).toHaveLength(Object.keys(ranger.skills).length);

    // Lore skill round-trips its lore field
    const loreRow = Object.values(skillSection).find((r) => r.skill_lore);
    expect(loreRow).toBeDefined();
    expect(loreRow!.skill_lore).toBe("Tracking");
  });

  it("uses content fingerprint as repeating row key, not Roll20 row id (§6.4)", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const featSection = result.payload.repeating.repeating_feats;
    const rowKey = Object.keys(featSection)[0]!;
    // Fingerprint is slug|source, not a Roll20-generated row ID like "-abc123"
    expect(rowKey).toBe("power-attack|Player Core");
    expect(rowKey).not.toMatch(/^-[a-z0-9]+$/);
  });

  it("sets sheet_version to the latest supported version", async () => {
    const result = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const latest = SUPPORTED_SHEET_VERSIONS[SUPPORTED_SHEET_VERSIONS.length - 1]!;
    expect(result.payload.sheet_version).toBe(latest);
  });
});

// ── Roll20-specific: import (§6.3) ──────────────────────────────────────────

describe("roll20 import", () => {
  it("imports identity from character_name attr", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.identity?.name).toBe(fighter.identity.name);
  });

  it("imports level from level attr", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.level).toBe(ranger.level);
  });

  it("imports class with keyAbility and hpPerLevel", async () => {
    const exportResult = await roll20Adapter.export(wizard, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.class?.reference.slug).toBe("wizard");
    expect(importResult.build.class?.keyAbility).toBe("int");
    expect(importResult.build.class?.hpPerLevel).toBe(6);
  });

  it("imports feats from repeating section", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.feats).toHaveLength(ranger.feats.length);
    const slugs = importResult.build.feats!.map((f) => f.reference.slug).sort();
    const origSlugs = ranger.feats.map((f) => f.reference.slug).sort();
    expect(slugs).toEqual(origSlugs);
  });

  it("imports skills including lore", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(Object.keys(importResult.build.skills ?? {})).toHaveLength(Object.keys(ranger.skills).length);
    const loreKey = "lore:tracking";
    expect(importResult.build.skills![loreKey]).toBeDefined();
    expect(importResult.build.skills![loreKey]!.lore).toBe("Tracking");
  });

  it("ignores unknown attributes without throwing (§5.2)", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const withUnknown: Roll20Sheet = {
      ...exportResult.payload,
      attributes: { ...exportResult.payload.attributes, custom_homebrew_attr: "whatever" },
    };
    const result = await roll20Adapter.import(withUnknown, { system: "pf2e", credentials: {} });
    expect(result.build).toBeDefined();
  });
});

// ── Roll20-specific: lossy ability scores (§5.2) ────────────────────────────
//
// Roll20 stores the derived score only; the canonical model stores components
// (boosts/flaws/itemBonus). The *value* round-trips (derivedAbilityValue gives
// back the same number) but the *components* are lost — stored as itemBonus =
// value - 10, with empty boosts/flaws. Both export and import emit lossyExport
// warnings.

describe("roll20 lossy ability scores (§5.2)", () => {
  it("value round-trips but components are lost", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    // Export: str 16 (10 + 2*3 boosts, 0 flaws, 0 itemBonus)
    expect(exportResult.payload.attributes.strength_score).toBe("16");
    expect(exportResult.warnings.some((w) =>
      w.kind.type === "lossyExport" && w.path === "build.abilityScores.str"
    )).toBe(true);

    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    const str = importResult.build.abilityScores?.str;
    expect(str).toBeDefined();
    // Components lost: empty boosts/flaws
    expect(str!.boosts).toHaveLength(0);
    expect(str!.flaws).toHaveLength(0);
    // Stored as itemBonus = 16 - 10 = 6
    expect(str!.itemBonus).toBe(6);
    // Import also warns about the lossy reconstruction
    expect(importResult.warnings.some((w) =>
      w.kind.type === "lossyExport" && w.path === "build.abilityScores.str"
    )).toBe(true);
  });

  it("wizard int score round-trips correctly", async () => {
    const exportResult = await roll20Adapter.export(wizard, { system: "pf2e", credentials: {} });
    // Wizard int: 10 + 2*5 boosts = 20
    expect(exportResult.payload.attributes.intelligence_score).toBe("20");

    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    const int = importResult.build.abilityScores?.int;
    expect(int).toBeDefined();
    expect(int!.itemBonus).toBe(10); // 20 - 10
    expect(int!.boosts).toHaveLength(0);
  });
});

// ── Roll20-specific: validate / sheet_version (§6.4) ─────────────────────────

describe("roll20 validate (§6.4)", () => {
  it("accepts all supported sheet versions", async () => {
    for (const version of SUPPORTED_SHEET_VERSIONS) {
      const sheet: Roll20Sheet = {
        sheet_version: version,
        character_id: "test",
        attributes: { character_name: "Test" },
        repeating: {},
      };
      const report = await roll20Adapter.validate(sheet);
      expect(report.valid, `version ${version} should be valid`).toBe(true);
    }
  });

  it("throws AdapterError for unsupported sheet_version", async () => {
    const sheet: Roll20Sheet = {
      sheet_version: "999.0",
      character_id: "test",
      attributes: { character_name: "Test" },
      repeating: {},
    };
    await expect(roll20Adapter.validate(sheet)).rejects.toThrow(AdapterError);
    try {
      await roll20Adapter.validate(sheet);
    } catch (e) {
      expect(e).toBeInstanceOf(AdapterError);
      const err = e as AdapterError;
      expect(err.kind).toEqual({
        type: "unsupportedSheetVersion",
        found: "999.0",
        supported: [...SUPPORTED_SHEET_VERSIONS],
      });
    }
  });

  it("returns invalid for missing sheet_version", async () => {
    const sheet = {
      sheet_version: "",
      character_id: "test",
      attributes: {},
      repeating: {},
    } as Roll20Sheet;
    const report = await roll20Adapter.validate(sheet);
    expect(report.valid).toBe(false);
    expect(report.errors).toContain("missing required field: sheet_version");
  });

  it("warns when character_name is absent", async () => {
    const sheet: Roll20Sheet = {
      sheet_version: "1.0",
      character_id: "test",
      attributes: {},
      repeating: {},
    };
    const report = await roll20Adapter.validate(sheet);
    expect(report.valid).toBe(true);
    expect(report.warnings).toContain("no character_name attribute present");
  });
});

// ── Roll20-specific: detectChanges (§5.3) ───────────────────────────────────

describe("roll20 detectChanges (§5.3)", () => {
  it("returns hasChanges=true for null previous (first sync)", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const result = await roll20Adapter.detectChanges(null, exportResult.payload);
    expect(result.hasChanges).toBe(true);
    expect(result.changedPaths).toEqual(["*"]);
  });

  it("returns hasChanges=false for identical sheets", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const result = await roll20Adapter.detectChanges(exportResult.payload, exportResult.payload);
    expect(result.hasChanges).toBe(false);
    expect(result.changedPaths).toHaveLength(0);
  });

  it("detects attribute changes", async () => {
    const exportResult = await roll20Adapter.export(fighter, { system: "pf2e", credentials: {} });
    const modified: Roll20Sheet = {
      ...exportResult.payload,
      attributes: { ...exportResult.payload.attributes, character_name: "Changed Name" },
    };
    const result = await roll20Adapter.detectChanges(exportResult.payload, modified);
    expect(result.hasChanges).toBe(true);
    expect(result.changedPaths).toContain("attributes");
  });

  it("detects repeating section changes", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const modified: Roll20Sheet = {
      ...exportResult.payload,
      repeating: { ...exportResult.payload.repeating, repeating_feats: {} },
    };
    const result = await roll20Adapter.detectChanges(exportResult.payload, modified);
    expect(result.hasChanges).toBe(true);
    expect(result.changedPaths).toContain("repeating");
  });
});

// ── Roll20-specific: round-trip (§5.1) ──────────────────────────────────────

describe("roll20 round-trip (§5.1)", () => {
  it("preserves identity.name through export→import", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.identity?.name).toBe(ranger.identity.name);
  });

  it("preserves level through export→import", async () => {
    const exportResult = await roll20Adapter.export(wizard, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.level).toBe(wizard.level);
  });

  it("preserves feat slugs through export→import", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    const origSlugs = ranger.feats.map((f) => f.reference.slug).sort();
    const reimpSlugs = importResult.build.feats!.map((f) => f.reference.slug).sort();
    expect(reimpSlugs).toEqual(origSlugs);
  });

  it("preserves skill keys through export→import", async () => {
    const exportResult = await roll20Adapter.export(wizard, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    const origKeys = Object.keys(wizard.skills).sort();
    const reimpKeys = Object.keys(importResult.build.skills ?? {}).sort();
    expect(reimpKeys).toEqual(origKeys);
  });

  it("preserves ancestry/heritage/background slugs", async () => {
    const exportResult = await roll20Adapter.export(ranger, { system: "pf2e", credentials: {} });
    const importResult = await roll20Adapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.ancestry?.reference.slug).toBe(ranger.ancestry.reference.slug);
    expect(importResult.build.heritage?.reference.slug).toBe(ranger.heritage.reference.slug);
    expect(importResult.build.background?.reference.slug).toBe(ranger.background.reference.slug);
  });
});
