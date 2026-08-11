// @nexus-sheet/adapter-sdk — the contribution surface (§5).
//
// The CharacterAdapter interface, contexts, warnings/errors, AdapterCapabilities
// with static fieldCoverage, and the contract test harness.
//
// canonicalPath() is the template-literal type helper that makes a typo in a
// field mapping a compile-time error, not a silently-skipped field (§6.3, §3.6).

import type { Character } from "@nexus-sheet/schema-types";

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

// The CharacterAdapter interface, contexts, warnings/errors, and the contract
// test harness land in 1.2 (§5, §16). canonicalPath() lands now because the
// type machinery depends on generated types existing (§3.6, 0.4 deliverable).
