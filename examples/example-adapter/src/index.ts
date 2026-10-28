import type {
  CharacterAdapter,
  AdapterCapabilities,
  ImportOutcome,
  ExportOutcome,
  ChangeSet,
  ValidationReport,
  ImportContext,
  ExportContext,
  AdapterWarning,
  PartialBuild,
} from "@nexus-sheet/adapter-sdk";
import type { CharacterBuild } from "@nexus-sheet/schema-types";

export interface ExamplePlatformPayload {
  characterName: string;
  level: number;
  ancestry?: CharacterBuild["ancestry"];
  heritage?: CharacterBuild["heritage"];
  background?: CharacterBuild["background"];
  class?: CharacterBuild["class"];
}

const FIELD_COVERAGE: readonly string[] = [
  "build.identity.name",
  "build.level",
  "build.ancestry",
  "build.heritage",
  "build.background",
  "build.class",
];

const CAPABILITIES: AdapterCapabilities = {
  supportsImport: true,
  supportsExport: true,
  supportsPartialExport: true,
  supportsSessionRead: false,
  fieldCoverage: FIELD_COVERAGE,
};

export const exampleAdapter: CharacterAdapter<ExamplePlatformPayload> = {
  id: "example",

  capabilities: CAPABILITIES,

  async import(
    raw: ExamplePlatformPayload,
    _ctx: ImportContext,
  ): Promise<ImportOutcome> {
    const warnings: AdapterWarning[] = [];
    const unmapped: string[] = [];

    const build: PartialBuild = {
      identity: { name: raw.characterName },
      level: raw.level,
    };

    if (raw.ancestry) build.ancestry = raw.ancestry;
    if (raw.heritage) build.heritage = raw.heritage;
    if (raw.background) build.background = raw.background;
    if (raw.class) build.class = raw.class;

    for (const key of Object.keys(raw)) {
      if (!["characterName", "level", "ancestry", "heritage", "background", "class"].includes(key)) {
        warnings.push({
          path: `raw.${key}`,
          kind: { type: "unmappedField" },
          detail: `Unknown platform field "${key}" — not mapped to canonical.`,
        });
        unmapped.push(`raw.${key}`);
      }
    }

    return { build, warnings, unmapped };
  },

  async export(
    build: CharacterBuild,
    _ctx: ExportContext,
  ): Promise<ExportOutcome<ExamplePlatformPayload>> {
    const warnings: AdapterWarning[] = [];
    const skipped: string[] = [];

    const payload: ExamplePlatformPayload = {
      characterName: build.identity.name,
      level: build.level,
    };

    if (build.ancestry) payload.ancestry = build.ancestry;
    else skipped.push("build.ancestry");

    if (build.heritage) payload.heritage = build.heritage;
    else skipped.push("build.heritage");

    if (build.background) payload.background = build.background;
    else skipped.push("build.background");

    if (build.class) payload.class = build.class;
    else skipped.push("build.class");

    for (const field of [
      "abilityScores", "skills", "proficiencies", "feats", "classFeatures",
      "equipment", "currencies", "spellcasting", "actions", "reactions",
      "explorationActivities", "companions", "familiars", "drones", "vehicles",
      "notes",
    ]) {
      if ((build as unknown as Record<string, unknown>)[field] !== undefined) {
        warnings.push({
          path: `build.${field}`,
          kind: { type: "lossyExport", reason: "not covered by example adapter" },
          detail: `Field "${field}" has no platform representation — dropped.`,
        });
        skipped.push(`build.${field}`);
      }
    }

    return { payload, warnings, skipped };
  },

  async detectChanges(
    previous: ExamplePlatformPayload | null,
    current: ExamplePlatformPayload,
  ): Promise<ChangeSet> {
    if (previous === null) {
      return { hasChanges: true, changedPaths: ["*"] };
    }

    const changedPaths: string[] = [];
    const keys: (keyof ExamplePlatformPayload)[] = [
      "characterName", "level", "ancestry", "heritage", "background", "class",
    ];

    for (const key of keys) {
      if (JSON.stringify(previous[key]) !== JSON.stringify(current[key])) {
        changedPaths.push(key);
      }
    }

    return { hasChanges: changedPaths.length > 0, changedPaths };
  },

  async validate(raw: ExamplePlatformPayload): Promise<ValidationReport> {
    const errors: string[] = [];
    const warnings: string[] = [];

    if (!raw || typeof raw !== "object") {
      return { valid: false, errors: ["payload must be an object"], warnings };
    }

    if (!raw.characterName || typeof raw.characterName !== "string") {
      errors.push("characterName is required and must be a string");
    }

    if (raw.level === undefined || typeof raw.level !== "number" || raw.level < 1) {
      errors.push("level is required and must be a positive number");
    }

    return { valid: errors.length === 0, errors, warnings };
  },
};
