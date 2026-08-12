// @nexus-sheet/adapter-sdk — the contribution surface (§5).
//
// The CharacterAdapter interface, contexts, warnings/errors, AdapterCapabilities
// with static fieldCoverage, and the contract test harness.
//
// canonicalPath() is the template-literal type helper that makes a typo in a
// field mapping a compile-time error, not a silently-skipped field (§6.3, §3.6).

import type { Character, CharacterBuild } from "@nexus-sheet/schema-types";

// ── canonicalPath() — compile-time-checked canonical field paths (§6.3) ──────
//
// A recursive template-literal type that walks the generated `Character` type
// and produces a union of every valid dotted path: `"build.identity.name"`,
// `"build.level"`, `"build.abilityScores.str"`, etc. A typo is a type error.
//
// `Prev` caps recursion at depth 10 (more than enough for our schema depth).
// Index signatures (`[k: string]: T`) contribute `string` as a valid key
// segment, so `"build.abilityScores.anyKey"` type-checks — the contract tests
// (§16) backstop that the *value* of the key is real on a fixture.

type Prev = [never, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

type Join<K extends string, P extends string> = P extends "" ? K : `${K}.${P}`;

type Paths<T, D extends number = 10> = [D] extends [never]
  ? never
  : T extends (infer U)[]
    ? `${number}` | Join<`${number}`, Paths<U, Prev[D]>>
    : T extends Record<string, infer V>
      ? string | Join<string, Paths<V, Prev[D]>>
      : T extends object
        ? {
            [K in keyof T & string]: K | Join<K, Paths<T[K], Prev[D]>>;
          }[keyof T & string]
        : never;

/** Every valid dotted path into `Character` (e.g. `"build.identity.name"`). */
export type CanonicalPath = Paths<Character>;

/**
 * Assert at compile time that `path` is a valid canonical field path (§6.3).
 *
 * At runtime this is the identity function — the value is the point: a typo
 * in a field mapping table is a **type error at build time**, not a silently
 * skipped field. This is the main reason the generated types are worth
 * committing (§3.6, §6.3).
 *
 * ```ts
 * import { canonicalPath } from "@nexus-sheet/adapter-sdk";
 * const map = [
 *   { path: canonicalPath("build.identity.name"), target: { kind: "attr", name: "character_name" } },
 *   { path: canonicalPath("build.level"),         target: { kind: "attr", name: "level" } },
 * ] as const;
 * ```
 *
 * Note: contributors who reach for `as any` bypass this; the contract tests
 * (§16) backstop by asserting every declared `fieldCoverage` path round-trips
 * on a real fixture.
 */
export function canonicalPath<P extends CanonicalPath>(path: P): P {
  return path;
}

// ── PartialBuild — all fields of CharacterBuild optional (§5.1, §5.2) ────────
//
// Mirrors the Rust `PartialCharacterBuild` (crates/adapter-host/src/outcome.rs).
// An adapter may not cover every canonical field; the `?` represents that.
// `deny_unknown_fields` on the Rust side ensures unknown field names are
// rejected at the boundary (§5.5).

/**
 * A partial `CharacterBuild` — all fields optional (§5.1, §5.2).
 *
 * This is what `ImportOutcome.build` contains. An adapter that only maps a
 * subset of canonical fields returns only those fields present.
 */
export type PartialBuild = {
  [K in keyof CharacterBuild]?: CharacterBuild[K];
};

// ── Warnings (§5.2) ──────────────────────────────────────────────────────────
//
// Homebrew content (§7.7) must always be a warning, never an error. An adapter
// that throws on unrecognized content is unusable in real Foundry worlds,
// which are full of it. The SDK's contract tests (§16) assert this directly.

/**
 * What kind of warning this is (§5.2). Discriminated union, tagged by `type`.
 */
export type WarningKind =
  | { type: "unmappedField" }
  | { type: "lossyExport"; reason: string }
  | { type: "fuzzyMatch"; confidence: number };

/**
 * A warning from the adapter — the operation succeeded with caveats (§5.2).
 * Homebrew content must always be a warning, never an error.
 */
export interface AdapterWarning {
  /** Canonical field path, e.g. `build.feats[2]`. */
  path: string;
  /** What kind of warning this is (§5.2). */
  kind: WarningKind;
  /** Human-readable detail. */
  detail: string;
}

// ── Errors (§5.2) ────────────────────────────────────────────────────────────

/**
 * What kind of error this is (§5.2). Discriminated union, tagged by `type`.
 */
export type AdapterErrorKind =
  | { type: "transport"; status?: number }
  | { type: "auth"; detail: string }
  | { type: "unsupportedSheetVersion"; found: string; supported: string[] }
  | { type: "malformedPayload"; detail: string };

/**
 * The operation could not proceed (§5.2). Thrown by adapter methods.
 */
export class AdapterError extends Error {
  constructor(
    readonly kind: AdapterErrorKind,
    message: string,
  ) {
    super(message);
    this.name = "AdapterError";
  }
}

// ── Outcomes (§5.1) ──────────────────────────────────────────────────────────

/**
 * The outcome of `adapter.import()` (§5.1).
 */
export interface ImportOutcome {
  build: PartialBuild;
  /** e.g. unrecognized feat, preserved in extensions. */
  warnings: AdapterWarning[];
  /** Canonical field paths the adapter could not map. */
  unmapped: string[];
}

/**
 * The outcome of `adapter.export()` (§5.1).
 */
export interface ExportOutcome<TPlatform = unknown> {
  payload: TPlatform;
  /** e.g. target has no `homeland` field, dropped. */
  warnings: AdapterWarning[];
  /** Canonical field paths that were skipped (no platform representation). */
  skipped: string[];
}

/**
 * A change set from `adapter.detectChanges()` (§5.3).
 */
export interface ChangeSet {
  /** True if anything changed since the last sync. */
  hasChanges: boolean;
  /** Field paths that changed. */
  changedPaths: string[];
}

/**
 * A validation report from `adapter.validate()` (§5.1).
 */
export interface ValidationReport {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

// ── Contexts (§5.4) ──────────────────────────────────────────────────────────
//
// Adapters are stateless (§5.4). Credentials arrive in ImportContext /
// ExportContext from the connection record (§13.3) and are never held by the
// adapter.

/**
 * Context passed to `adapter.import()` (§5.4). Carries credentials from the
 * connection record — adapters never hold credentials themselves.
 */
export interface ImportContext {
  /** The system this import targets (e.g. "pf2e"). */
  system: string;
  /** Platform credentials (API tokens, etc.) — from the connection record.
   *  Opaque to the host; the adapter knows how to use them. */
  credentials: Record<string, unknown>;
}

/**
 * Context passed to `adapter.export()` (§5.4).
 */
export interface ExportContext {
  system: string;
  credentials: Record<string, unknown>;
}

// ── AdapterCapabilities (§5.4) ───────────────────────────────────────────────

/**
 * Static capabilities declared by the adapter (§5.4). Readable without
 * instantiating the adapter or touching the network, so the UI can render
 * an honest "Roll20 covers 78% of this character; these 4 fields will be
 * dropped" preview *before* the user commits to a push.
 */
export interface AdapterCapabilities {
  supportsImport: boolean;
  supportsExport: boolean;
  supportsPartialExport: boolean;
  supportsSessionRead: boolean;
  /** Canonical paths this adapter round-trips (§5.4). */
  fieldCoverage: readonly string[];
}

// ── CharacterAdapter<TPlatform> (§5.1) ───────────────────────────────────────
//
// The surface community contributors write against. Generic over the platform
// format — an adapter author gets full type inference over their own platform's
// shapes with no boilerplate (§5.1). This is a small but genuine ergonomic win
// from putting adapters in TS, and it lands exactly where ergonomics matter
// most: the code strangers write.

/**
 * The interface community contributors implement (§5.1).
 *
 * ```ts
 * import type { CharacterAdapter } from "@nexus-sheet/adapter-sdk";
 *
 * const myAdapter: CharacterAdapter<MyPlatformPayload> = {
 *   id: "my-platform",
 *   capabilities: { ... },
 *   async import(raw, ctx) { ... },
 *   async export(build, ctx) { ... },
 *   async detectChanges(previous, current) { ... },
 *   async validate(raw) { ... },
 * };
 * ```
 */
export interface CharacterAdapter<TPlatform = unknown> {
  /** "roll20" | "foundry" | "pathbuilder" | ... */
  readonly id: string;
  readonly capabilities: AdapterCapabilities;

  /**
   * Platform payload -> canonical. Never throws on unknown content;
   * unmapped data goes to `extensions` and is reported in warnings (§5.2).
   */
  import(raw: TPlatform, ctx: ImportContext): Promise<ImportOutcome>;

  /** Canonical -> platform payload. */
  export(build: CharacterBuild, ctx: ExportContext): Promise<ExportOutcome<TPlatform>>;

  /** Cheap drift check — no full import/merge cycle (§5.3). */
  detectChanges(previous: TPlatform | null, current: TPlatform): Promise<ChangeSet>;

  /** Structural + platform-specific validation. */
  validate(raw: TPlatform): Promise<ValidationReport>;
}
