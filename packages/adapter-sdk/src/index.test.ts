import { describe, it, expect } from "vitest";
import { runContractTests, type AdapterFixture } from "./testing.js";
import type { CharacterAdapter, ImportOutcome, ExportOutcome, ChangeSet, ValidationReport } from "./index.js";
import type { CharacterBuild } from "@nexus-sheet/schema-types";

// ── Mock adapter for self-testing the contract harness ───────────────────────
//
// A trivial pass-through adapter that exports the build as-is and imports it
// back. Used to verify the harness itself works correctly — if the harness
// has a bug, it will fail against this well-behaved adapter.

type MockPayload = {
  build: CharacterBuild;
  __nexusUnknownTestField?: string;
};

const mockAdapter: CharacterAdapter<MockPayload> = {
  id: "mock",
  capabilities: {
    supportsImport: true,
    supportsExport: true,
    supportsPartialExport: false,
    supportsSessionRead: false,
    fieldCoverage: ["build.identity.name", "build.level"],
  },

  async import(raw, _ctx): Promise<ImportOutcome> {
    return {
      build: raw.build,
      warnings: raw.__nexusUnknownTestField
        ? [{ path: "build", kind: { type: "unmappedField" }, detail: "unknown field" }]
        : [],
      unmapped: [],
    };
  },

  async export(build, _ctx): Promise<ExportOutcome<MockPayload>> {
    return { payload: { build }, warnings: [], skipped: [] };
  },

  async detectChanges(previous, current): Promise<ChangeSet> {
    if (previous === null) {
      return { hasChanges: true, changedPaths: ["*"] };
    }
    const prevJson = JSON.stringify(previous);
    const currJson = JSON.stringify(current);
    return {
      hasChanges: prevJson !== currJson,
      changedPaths: prevJson !== currJson ? ["*"] : [],
    };
  },

  async validate(raw): Promise<ValidationReport> {
    if (!raw || typeof raw !== "object" || !("build" in raw)) {
      return { valid: false, errors: ["missing build field"], warnings: [] };
    }
    return { valid: true, errors: [], warnings: [] };
  },
};

// ── Minimal fixture ──────────────────────────────────────────────────────────

const minimalBuild = {
  identity: { name: "Test" },
  level: 1,
} as unknown as CharacterBuild;

const fixtures: AdapterFixture<MockPayload>[] = [
  {
    name: "minimal",
    build: minimalBuild,
    platformPayload: { build: minimalBuild },
    badPayload: {} as MockPayload,
  },
];

// ── Run the harness against the mock ─────────────────────────────────────────

describe("contract harness self-test", () => {
  // The harness itself is tested by running it against a known-good adapter.
  // If these tests pass, the harness is working correctly.
  runContractTests(mockAdapter, fixtures);
});

// ── Direct harness tests ─────────────────────────────────────────────────────

describe("runContractTests structure", () => {
  it("mock adapter has correct shape", () => {
    expect(mockAdapter.id).toBe("mock");
    expect(mockAdapter.capabilities.fieldCoverage).toContain("build.identity.name");
  });

  it("mock adapter round-trips", async () => {
    const exportResult = await mockAdapter.export(minimalBuild, { system: "pf2e", credentials: {} });
    const importResult = await mockAdapter.import(exportResult.payload, { system: "pf2e", credentials: {} });
    expect(importResult.build.identity?.name).toBe("Test");
  });
});
