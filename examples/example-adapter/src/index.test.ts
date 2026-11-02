import { describe } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runContractTests, type AdapterFixture } from "@nexus-ttrpg/adapter-sdk/testing";
import type { CharacterBuild } from "@nexus-ttrpg/schema-types";
import { exampleAdapter, type ExamplePlatformPayload } from "./index.js";

function loadFixture(filename: string): CharacterBuild {
  const path = resolve(__dirname, "../../../examples/fixtures", filename);
  return JSON.parse(readFileSync(path, "utf-8")) as CharacterBuild;
}

function makeFixture(
  name: string,
  build: CharacterBuild,
): AdapterFixture<ExamplePlatformPayload> {
  const payload: ExamplePlatformPayload = {
    characterName: build.identity.name,
    level: build.level,
  };
  if (build.ancestry) payload.ancestry = build.ancestry;
  if (build.heritage) payload.heritage = build.heritage;
  if (build.background) payload.background = build.background;
  if (build.class) payload.class = build.class;
  return {
    name,
    build,
    platformPayload: payload,
    badPayload: { characterName: "", level: 0 },
  };
}

const fighter = loadFixture("pf2e-level1-fighter.json");
const ranger = loadFixture("pf2e-level5-ranger.json");
const wizard = loadFixture("pf2e-level10-wizard.json");
const mechanic = loadFixture("sf2e-level5-mechanic.json");

const fixtures: AdapterFixture<ExamplePlatformPayload>[] = [
  makeFixture("level1-fighter", fighter),
  makeFixture("level5-ranger", ranger),
  makeFixture("level10-wizard", wizard),
  makeFixture("level5-mechanic", mechanic),
];

describe("example adapter contract", () => {
  runContractTests(exampleAdapter, fixtures);
});
