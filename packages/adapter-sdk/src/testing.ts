// @nexus-sheet/adapter-sdk/testing — the one-line contract test harness (§16).
//
//   describe("roll20", () => runContractTests(adapter, fixtures));
//
// Asserts:
//   - import(export(fixture)) round-trips within fieldCoverage (§5.1)
//   - validate() rejects known-bad input
//   - detectChanges is a no-op on unchanged input (§5.3)
//   - unknown content produces warnings, never throws (§5.2 — the homebrew invariant)
//
// Runs standalone in pure TS — no Rust, no host, no DB, no network (§16, §21).
// If a contributor ever needs `cargo` to iterate on an adapter, that is a bug
// in the SDK.

import { describe, it, expect } from "vitest";
import type {
  CharacterAdapter,
  ImportContext,
  ExportContext,
} from "./index.js";
import type { CharacterBuild } from "@nexus-sheet/schema-types";

// ── Fixture type ─────────────────────────────────────────────────────────────

/**
 * A test fixture: a canonical `CharacterBuild` plus the raw platform payload
 * that the adapter maps to/from it. Used by `runContractTests` to verify
 * round-trip fidelity.
 *
 * The `badPayload` field is optional — if present, `validate()` must reject it.
 */
export interface AdapterFixture<TPlatform = unknown> {
  /** Human-readable name for the test case. */
  name: string;
  /** The canonical build — what `import(platformPayload)` should approximate. */
  build: CharacterBuild;
  /** The raw platform payload — what `export(build)` should produce. */
  platformPayload: TPlatform;
  /** A known-bad payload that `validate()` must reject. Optional. */
  badPayload?: TPlatform;
}

// ── runContractTests ─────────────────────────────────────────────────────────

/**
 * Run the standard contract test suite against an adapter (§16).
 *
 * ```ts
 * import { runContractTests } from "@nexus-sheet/adapter-sdk/testing";
 * import { myAdapter } from "../src/index.js";
 * import { fixtures } from "../../examples/fixtures/index.js";
 *
 * describe("my-platform", () => runContractTests(myAdapter, fixtures));
 * ```
 *
 * This is the single most important artifact for contribution quality: a new
 * adapter gets a meaningful correctness bar for free, in a test runner its
 * author already knows, with **no Rust in the loop** (§16).
 */
export function runContractTests<TPlatform>(
  adapter: CharacterAdapter<TPlatform>,
  fixtures: AdapterFixture<TPlatform>[],
): void {
  // ── Contract: the adapter has the required static shape ──────────────────

  describe("adapter shape", () => {
    it("has a non-empty id", () => {
      expect(adapter.id).toBeTruthy();
      expect(typeof adapter.id).toBe("string");
    });

    it("declares capabilities with fieldCoverage", () => {
      expect(adapter.capabilities).toBeDefined();
      expect(Array.isArray(adapter.capabilities.fieldCoverage)).toBe(true);
    });
  });

  // ── Contract: import(export(fixture)) round-trips within fieldCoverage ───
  //
  // For each fixture: export the canonical build to the platform payload,
  // then import it back. Every field in `fieldCoverage` that was present in
  // the original build must be present in the re-imported build (§5.1, §16).

  describe("round-trip within fieldCoverage (§5.1)", () => {
    for (const fixture of fixtures) {
      it(`${fixture.name}: export → import preserves covered fields`, async () => {
        const ctx = makeImportContext();
        const exportCtx = makeExportContext();

        // Export canonical → platform
        const exportResult = await adapter.export(fixture.build, exportCtx);
        expect(exportResult.payload).toBeDefined();

        // Import platform → canonical
        const importResult = await adapter.import(exportResult.payload, ctx);
        expect(importResult.build).toBeDefined();

        // Every field in fieldCoverage that was present in the original
        // build should be present in the re-imported build.
        for (const path of adapter.capabilities.fieldCoverage) {
          const original = getField(fixture.build, path);
          if (original !== undefined) {
            const reimported = getField(importResult.build, path);
            expect(reimported, `field "${path}" was present in original but missing after round-trip`).toBeDefined();
          }
        }
      });
    }
  });

  // ── Contract: validate() rejects known-bad input ──────────────────────────

  describe("validate rejects bad input (§5.1)", () => {
    for (const fixture of fixtures) {
      const badPayload = fixture.badPayload;
      if (badPayload === undefined) continue;

      it(`${fixture.name}: validate() rejects badPayload`, async () => {
        const report = await adapter.validate(badPayload);
        expect(report.valid).toBe(false);
        expect(report.errors.length).toBeGreaterThan(0);
      });
    }

    // If no fixtures have badPayload, we still test that validate doesn't
    // throw on valid input.
    const hasBadPayload = fixtures.some((f) => f.badPayload !== undefined);
    if (!hasBadPayload) {
      it("validate accepts valid input (no badPayload fixtures provided)", async () => {
        const fixture = fixtures[0];
        if (fixture) {
          const report = await adapter.validate(fixture.platformPayload);
          expect(report.valid).toBe(true);
        }
      });
    }
  });

  // ── Contract: detectChanges is a no-op on unchanged input (§5.3) ──────────

  describe("detectChanges no-op on unchanged input (§5.3)", () => {
    for (const fixture of fixtures) {
      it(`${fixture.name}: detectChanges(payload, payload) → hasChanges=false`, async () => {
        const result = await adapter.detectChanges(fixture.platformPayload, fixture.platformPayload);
        expect(result.hasChanges).toBe(false);
      });
    }
  });

  // ── Contract: unknown content produces warnings, never throws (§5.2) ──────
  //
  // The homebrew invariant. Feeding an adapter unknown/unrecognized content
  // must produce warnings and a successful result, not a rejection. An adapter
  // that throws on unrecognized content is unusable in real Foundry worlds,
  // which are full of it (§5.2).

  describe("unknown content warns, never throws (§5.2)", () => {
    for (const fixture of fixtures) {
      it(`${fixture.name}: import handles unknown content without throwing`, async () => {
        // Inject an unknown field into the platform payload. The adapter
        // should not throw — it should return warnings.
        const payloadWithUnknown = injectUnknownField(fixture.platformPayload);
        const ctx = makeImportContext();

        // Must not throw.
        const result = await adapter.import(payloadWithUnknown, ctx);

        // Should succeed (warnings are OK, errors are not).
        expect(result.build).toBeDefined();
      });
    }
  });
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeImportContext(): ImportContext {
  return {
    system: "pf2e",
    credentials: {},
  };
}

function makeExportContext(): ExportContext {
  return {
    system: "pf2e",
    credentials: {},
  };
}

/**
 * Get a value at a dotted path from an object. Returns `undefined` if any
 * segment is missing.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function getField(obj: any, path: string): unknown {
  const parts = path.split(".");
  let current: unknown = obj;

  // Skip "build." prefix if present — PartialBuild is already the build.
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

/**
 * Inject an unknown field into a platform payload. Creates a shallow copy
 * with an extra `__nexusUnknownTestField` property so the adapter sees
 * content it doesn't recognize.
 */
function injectUnknownField<T>(payload: T): T {
  if (payload != null && typeof payload === "object") {
    return { ...(payload as object), __nexusUnknownTestField: "homebrew-content" } as T;
  }
  return payload;
}
