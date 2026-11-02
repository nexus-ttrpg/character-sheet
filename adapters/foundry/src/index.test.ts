// Foundry adapter contract tests (§16).
//
//   describe("foundry", () => runContractTests(adapter, fixtures));
//
// The single most important artifact for contribution quality: a new adapter
// gets a meaningful correctness bar for free, with no Rust in the loop (§16).
//
// Fixtures are the shared JSON ground-truth from examples/fixtures/ (§16) —
// one set, consumed by both Rust and TypeScript as plain JSON.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runContractTests, type AdapterFixture } from "@nexus-ttrpg/adapter-sdk/testing";
import type { CharacterBuild } from "@nexus-ttrpg/schema-types";
import { foundryAdapter } from "./index.js";
import { exportActor, importActor, derivedMaxHp, derivedAbilityValue, type FoundryActor, type FoundrySystemId } from "@nexus-ttrpg/foundry-mapping";

// ── Load PF2e fixtures ───────────────────────────────────────────────────────

function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}

const fighter = loadFixture("pf2e-level1-fighter.json");
const ranger = loadFixture("pf2e-level5-ranger.json");
const wizard = loadFixture("pf2e-level10-wizard.json");
const mechanic = loadFixture("sf2e-level5-mechanic.json");

// Build AdapterFixtures: export each canonical build to a Foundry Actor,
// which becomes the platformPayload for the contract harness.
function makeFixture(name: string, build: CharacterBuild, system: FoundrySystemId): AdapterFixture<FoundryActor> {
  const actor = exportActor(build, system);
  return {
    name,
    build,
    platformPayload: actor,
    badPayload: { name: "", type: "character", system: {} } as FoundryActor,
  };
}

const fixtures: AdapterFixture<FoundryActor>[] = [
  makeFixture("level1-fighter", fighter, "pf2e"),
  makeFixture("level5-ranger", ranger, "pf2e"),
  makeFixture("level10-wizard", wizard, "pf2e"),
  makeFixture("level5-mechanic", mechanic, "sf2e"),
];

// ── Run the standard contract suite ──────────────────────────────────────────

describe("foundry adapter contract", () => {
  runContractTests(foundryAdapter, fixtures);
});

// ── Foundry-specific tests ───────────────────────────────────────────────────
//
// These test Foundry-specific behavior beyond the standard contract: the §7.2
// max-HP-only export rule, the §7.3 UUID round-trip, and the §7.7 homebrew
// invariant.

describe("foundry export (§7.2)", () => {
  it("writes max HP only — no current HP or temp HP (§4, §7.2)", () => {
    const actor = exportActor(fighter, "pf2e");
    expect(actor.system.attributes.hp).toEqual({ max: expect.any(Number) });
    expect(actor.system.attributes.hp).not.toHaveProperty("value");
    expect(actor.system.attributes.hp).not.toHaveProperty("temp");
  });

  it("derived max HP is positive for a level 1 fighter", () => {
    const hp = derivedMaxHp(fighter);
    expect(hp).toBeGreaterThan(0);
    // Fighter: 10 hp/level, level 1, con score 12 (1 boost), mod +1
    // 10 * 1 + 1 * 1 = 11
    expect(hp).toBe(11);
  });

  it("maps identity.name to actor.name", () => {
    const actor = exportActor(fighter, "pf2e");
    expect(actor.name).toBe(fighter.identity.name);
  });

  it("maps class keyAbility and hpPerLevel", () => {
    const actor = exportActor(wizard, "pf2e");
    expect(actor.system.details.class.keyAbility).toBe("int");
    expect(actor.system.details.class.hpPerLevel).toBe(6);
  });

  it("maps ability score boosts/flaws/itemBonus", () => {
    const actor = exportActor(fighter, "pf2e");
    const str = actor.system.abilities.str;
    expect(str).toBeDefined();
    expect(str!.boosts).toHaveLength(3);
    expect(str!.flaws).toHaveLength(0);
    expect(str!.itemBonus).toBe(0);
    // Derived: 10 + 2*3 - 0 + 0 = 16
    expect(str!.value).toBe(16);
  });

  it("exports feats as items with type 'feat'", () => {
    const actor = exportActor(fighter, "pf2e");
    const feats = actor.items.filter((i) => i.type === "feat");
    expect(feats).toHaveLength(fighter.feats.length);
    expect(feats[0]!.system.slug).toBe(fighter.feats[0]!.reference.slug);
    expect(feats[0]!.system.category).toBe(fighter.feats[0]!.category);
  });

  it("exports spellcasting entries and their spells", () => {
    const actor = exportActor(wizard, "pf2e");
    const scEntries = actor.items.filter((i) => i.type === "spellcastingEntry");
    expect(scEntries).toHaveLength(1);
    expect(scEntries[0]!.system.tradition).toBe("arcane");
    expect(scEntries[0]!.system.slots).toMatchObject({ "1": 3, "2": 3 });

    const spells = actor.items.filter((i) => i.type === "spell");
    expect(spells).toHaveLength(8);
  });
});

describe("foundry import (§7.2)", () => {
  it("imports identity from actor.name", () => {
    const actor = exportActor(fighter, "pf2e");
    const build = importActor(actor);
    expect(build.identity.name).toBe(fighter.identity.name);
  });

  it("imports ability scores with boosts/flaws preserved", () => {
    const actor = exportActor(fighter, "pf2e");
    const build = importActor(actor);
    const str = build.abilityScores.str;
    expect(str).toBeDefined();
    expect(str!.boosts).toHaveLength(3);
    expect(str!.boosts[0]!.source).toBe("ancestry");
  });

  it("imports feats from items", () => {
    const actor = exportActor(fighter, "pf2e");
    const build = importActor(actor);
    expect(build.feats).toHaveLength(fighter.feats.length);
    expect(build.feats[0]!.reference.slug).toBe(fighter.feats[0]!.reference.slug);
    expect(build.feats[0]!.category).toBe(fighter.feats[0]!.category);
  });

  it("imports spellcasting entries and spells", () => {
    const actor = exportActor(wizard, "pf2e");
    const build = importActor(actor);
    expect(build.spellcasting.entries).toHaveLength(1);
    expect(build.spellcasting.entries[0]!.tradition).toBe("arcane");
    expect(build.spellcasting.entries[0]!.spells).toHaveLength(8);
  });
});

describe("foundry round-trip (§5.1, §16)", () => {
  it("import(export(build)) preserves all fieldCoverage fields", () => {
    const actor = exportActor(ranger, "pf2e");
    const imported = importActor(actor);

    for (const path of foundryAdapter.capabilities.fieldCoverage) {
      const original = getField(ranger, path);
      const reimported = getField(imported, path);

      if (original !== undefined) {
        expect(reimported, `field "${path}" missing after round-trip`).toBeDefined();
      }
    }
  });

  it("round-trip preserves feat slugs", () => {
    const actor = exportActor(ranger, "pf2e");
    const imported = importActor(actor);

    const originalSlugs = ranger.feats.map((f) => f.reference.slug).sort();
    const reimportedSlugs = imported.feats.map((f) => f.reference.slug).sort();
    expect(reimportedSlugs).toEqual(originalSlugs);
  });

  it("round-trip preserves ability score components", () => {
    const actor = exportActor(wizard, "pf2e");
    const imported = importActor(actor);

    const origInt = wizard.abilityScores.int!;
    const reimpInt = imported.abilityScores.int!;

    expect(reimpInt.boosts).toHaveLength(origInt.boosts.length);
    expect(reimpInt.flaws).toHaveLength(origInt.flaws.length);
    expect(reimpInt.itemBonus).toBe(origInt.itemBonus);
  });
});

describe("foundry SF2e coverage (§3.4, §2.4)", () => {
  it("exports drones as items with type 'drone'", () => {
    const actor = exportActor(mechanic, "sf2e");
    const drones = actor.items.filter((i) => i.type === "drone");
    expect(drones).toHaveLength(mechanic.drones.length);
    expect(drones[0]!.system.slug).toBe(mechanic.drones[0]!.reference.slug);
    expect(drones[0]!.system.chassis).toBe(mechanic.drones[0]!.chassis);
    expect(drones[0]!.system.mods).toHaveLength(mechanic.drones[0]!.mods.length);
    expect(drones[0]!.system.mods![0]!.slug).toBe(mechanic.drones[0]!.mods[0]!.slug);
  });

  it("exports vehicles as items with type 'vehicle'", () => {
    const actor = exportActor(mechanic, "sf2e");
    const vehicles = actor.items.filter((i) => i.type === "vehicle");
    expect(vehicles).toHaveLength(mechanic.vehicles.length);
    expect(vehicles[0]!.system.slug).toBe(mechanic.vehicles[0]!.reference.slug);
    expect(vehicles[0]!.system.role).toBe(mechanic.vehicles[0]!.role);
  });

  it("exports credits/UPB in currency (§3.4)", () => {
    const actor = exportActor(mechanic, "sf2e");
    expect(actor.system.currency.credits).toBe(mechanic.currencies.credits);
    expect(actor.system.currency.upb).toBe(mechanic.currencies.upb);
  });

  it("round-trip preserves drone chassis + mods", () => {
    const actor = exportActor(mechanic, "sf2e");
    const imported = importActor(actor);

    expect(imported.drones).toHaveLength(mechanic.drones.length);
    expect(imported.drones[0]!.reference.slug).toBe(mechanic.drones[0]!.reference.slug);
    expect(imported.drones[0]!.chassis).toBe(mechanic.drones[0]!.chassis);
    const origModSlugs = mechanic.drones[0]!.mods.map((m) => m.slug).sort();
    const reimpModSlugs = imported.drones[0]!.mods.map((m) => m.slug).sort();
    expect(reimpModSlugs).toEqual(origModSlugs);
  });

  it("round-trip preserves vehicle role", () => {
    const actor = exportActor(mechanic, "sf2e");
    const imported = importActor(actor);

    expect(imported.vehicles).toHaveLength(mechanic.vehicles.length);
    expect(imported.vehicles[0]!.reference.slug).toBe(mechanic.vehicles[0]!.reference.slug);
    expect(imported.vehicles[0]!.role).toBe(mechanic.vehicles[0]!.role);
  });

  it("round-trip preserves credits/UPB", () => {
    const actor = exportActor(mechanic, "sf2e");
    const imported = importActor(actor);

    expect(imported.currencies.credits).toBe(mechanic.currencies.credits);
    expect(imported.currencies.upb).toBe(mechanic.currencies.upb);
  });
});

describe("foundry homebrew invariant (§7.7)", () => {
  it("import does not throw on unknown item type", async () => {
    const actor = exportActor(fighter, "pf2e");
    const actorWithHomebrew: FoundryActor = {
      ...actor,
      items: [
        ...actor.items,
        {
          type: "customHomebrewItem" as unknown as FoundryActor["items"][number]["type"],
          name: "Homebrew Feat",
          system: { slug: "homebrew-feat" },
        },
      ],
    };

    const result = await foundryAdapter.import(actorWithHomebrew, {
      system: "pf2e",
      credentials: {},
    });

    expect(result.build).toBeDefined();
    expect(result.warnings.length).toBeGreaterThan(0);
    const unmappedWarning = result.warnings.find((w) => w.kind.type === "unmappedField");
    expect(unmappedWarning).toBeDefined();
  });
});

// ── Helper ───────────────────────────────────────────────────────────────────

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function getField(obj: any, path: string): unknown {
  const parts = path.split(".");
  let current: unknown = obj;

  if (parts[0] === "build") {
    parts.shift();
  }

  for (const part of parts) {
    if (current == null || typeof current !== "object") {
      return undefined;
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    current = (current as any)[part];
  }
  return current;
}
