import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runContractTests, type AdapterFixture } from "@nexus-ttrpg/adapter-sdk/testing";
import { AdapterError } from "@nexus-ttrpg/adapter-sdk";
import type { CharacterBuild, AbilityScoreKey } from "@nexus-ttrpg/schema-types";
import { pathbuilderAdapter, type PathbuilderCharacter } from "./index.js";
import { canonicalToPathbuilder } from "./mapping.js";
function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}
function makeFixture(name: string, build: CharacterBuild): AdapterFixture<PathbuilderCharacter> {
  return {
    name,
    build,
    platformPayload: canonicalToPathbuilder(build),
    badPayload: { ...canonicalToPathbuilder(build), name: "", level: 0 },
  };
}
const fighter = loadFixture("pf2e-level1-fighter.json");
const ranger = loadFixture("pf2e-level5-ranger.json");
const wizard = loadFixture("pf2e-level10-wizard.json");

const fixtures: AdapterFixture<PathbuilderCharacter>[] = [
  makeFixture("level1-fighter", fighter),
  makeFixture("level5-ranger", ranger),
  makeFixture("level10-wizard", wizard),
];

describe("pathbuilder adapter contract", () => {
  runContractTests(pathbuilderAdapter, fixtures);
});
// ── Pathbuilder-specific behavior ────────────────────────────────────────────

describe("pathbuilder: capabilities (§8)", () => {
  it("declares supportsExport false — no write-back API", () => {
    expect(pathbuilderAdapter.capabilities.supportsExport).toBe(false);
    expect(pathbuilderAdapter.capabilities.supportsImport).toBe(true);
    expect(pathbuilderAdapter.id).toBe("pathbuilder");
  });
});
describe("pathbuilder: ability scores round-trip non-lossy (§3.3)", () => {
  for (const [name, build] of [
    ["fighter", fighter],
    ["ranger", ranger],
    ["wizard", wizard],
  ] as const) {
    it(`${name}: boosts/flaws/itemBonus survive export → import`, async () => {
      const exported = await pathbuilderAdapter.export(build, { system: "pf2e", credentials: {} });
      const imported = await pathbuilderAdapter.import(exported.payload, { system: "pf2e", credentials: {} });

      for (const key of Object.keys(build.abilityScores) as AbilityScoreKey[]) {
        const orig = build.abilityScores[key]!;
        const got = imported.build.abilityScores![key];
        expect(got, `${key} missing after round-trip`).toBeDefined();
        expect(got!.boosts.map((b) => b.source)).toEqual(orig.boosts.map((b) => b.source));
        expect(got!.flaws.map((b) => b.source)).toEqual(orig.flaws.map((b) => b.source));
        expect(got!.itemBonus).toBe(orig.itemBonus);
      }
    });
  }
});
describe("pathbuilder: unsupported schemaVersion throws (§6.4)", () => {
  it("throws AdapterError on an unknown version", async () => {
    const good = canonicalToPathbuilder(fighter);
    const bad = { ...good, schemaVersion: 99 };
    await expect(
      pathbuilderAdapter.validate(bad),
    ).rejects.toThrow(AdapterError);
  });
});

describe("pathbuilder: unknown content warns, never throws (§5.2)", () => {
  it("import warns on an unknown top-level field", async () => {
    const payload = { ...canonicalToPathbuilder(fighter), homebrewThing: "x" };
    const result = await pathbuilderAdapter.import(payload, { system: "pf2e", credentials: {} });
    expect(result.build).toBeDefined();
    expect(result.unmapped).toContain("homebrewThing");
    expect(result.warnings.some((w) => w.path === "homebrewThing")).toBe(true);
  });
});
