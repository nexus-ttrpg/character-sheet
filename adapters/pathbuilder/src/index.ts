// @nexus-sheet/pathbuilder-adapter — first third-party-builder adapter (§8).
// See src/mapping.ts for the FORMAT FIDELITY CAVEAT.
// §8: Pathbuilder has no write-back API, so supportsExport is false.
// export() is still implemented (manual re-import + round-trip contract).
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
import { AdapterError } from "@nexus-sheet/adapter-sdk";
import type { CharacterBuild } from "@nexus-sheet/schema-types";
import {
  canonicalToPathbuilder,
  pathbuilderToCanonical,
  PATHBUILDER_FIELD_COVERAGE,
  PATHBUILDER_TOP_KEYS,
  SUPPORTED_SCHEMA_VERSIONS,
  type PathbuilderCharacter,
} from "./mapping.js";
export const pathbuilderAdapter: CharacterAdapter<PathbuilderCharacter> = {
  id: "pathbuilder",

  capabilities: {
    supportsImport: true,
    // §8: no write-back API — engine won't offer push. export() still
    // implemented (manual re-import + round-trip contract).
    supportsExport: false,
    supportsPartialExport: false,
    // HP/conditions/spell slots are scene-scoped, never synced (§4).
    supportsSessionRead: false,
    fieldCoverage: PATHBUILDER_FIELD_COVERAGE,
  },
  // import: Pathbuilder JSON -> canonical (§8). Never throws on unknown
  // content — unmapped top-level keys -> warnings (§5.2 homebrew invariant).

  async import(
    raw: PathbuilderCharacter,
    _ctx: ImportContext,
  ): Promise<ImportOutcome> {
    const warnings: AdapterWarning[] = [];
    const unmapped: string[] = [];

    for (const key of Object.keys(raw)) {
      if (!PATHBUILDER_TOP_KEYS.has(key)) {
        warnings.push({
          path: key,
          kind: { type: "unmappedField" },
          detail: `Unknown top-level field "${key}" — ignored`,
        });
        unmapped.push(key);
      }
    }

    const build = pathbuilderToCanonical(raw);
    return { build, warnings, unmapped };
  },
  // export: canonical -> Pathbuilder JSON (§8). supportsExport is false, but
  // this is implemented so a user can re-import by hand and the round-trip
  // contract holds. Non-lossy on ability components (§3.3).

  async export(
    build: CharacterBuild,
    _ctx: ExportContext,
  ): Promise<ExportOutcome<PathbuilderCharacter>> {
    const warnings: AdapterWarning[] = [];
    const skipped: string[] = [];

    warnings.push({
      path: "build",
      kind: { type: "lossyExport", reason: "Pathbuilder has no write-back API; payload is for manual re-import only (§8)" },
      detail: "export() produces a Pathbuilder JSON the user imports by hand",
    });

    const payload = canonicalToPathbuilder(build);
    return { payload, warnings, skipped };
  },
  // detectChanges: structural drift check (§5.3).

  async detectChanges(
    previous: PathbuilderCharacter | null,
    current: PathbuilderCharacter,
  ): Promise<ChangeSet> {
    if (previous === null) {
      return { hasChanges: true, changedPaths: ["*"] };
    }
    if (JSON.stringify(previous) === JSON.stringify(current)) {
      return { hasChanges: false, changedPaths: [] };
    }
    return { hasChanges: true, changedPaths: ["*"] };
  },

  // validate: schemaVersion check + structural validation (§6.4).

  async validate(raw: PathbuilderCharacter): Promise<ValidationReport> {
    const errors: string[] = [];
    const warnings: string[] = [];

    if (!raw || typeof raw !== "object") {
      return { valid: false, errors: ["not an object"], warnings };
    }

    const version = raw.schemaVersion;
    if (version == null) {
      errors.push("missing required field: schemaVersion");
    } else if (!SUPPORTED_SCHEMA_VERSIONS.includes(version)) {
      throw new AdapterError(
        { type: "unsupportedSheetVersion", found: String(version), supported: SUPPORTED_SCHEMA_VERSIONS.map(String) },
        `schemaVersion "${version}" not in supported list [${SUPPORTED_SCHEMA_VERSIONS.join(", ")}]`,
      );
    }

    if (!raw.name) errors.push("missing required field: name");
    if (typeof raw.level !== "number" || raw.level < 1) {
      errors.push("level is required and must be a positive number");
    }
    if (!raw.class) errors.push("missing required field: class");

    return { valid: errors.length === 0, errors, warnings };
  },
};

// Re-export mapping types/values for adapter consumers.
export type { PathbuilderCharacter } from "./mapping.js";
export {
  canonicalToPathbuilder,
  pathbuilderToCanonical,
  PATHBUILDER_FIELD_COVERAGE,
  SUPPORTED_SCHEMA_VERSIONS,
} from "./mapping.js";
