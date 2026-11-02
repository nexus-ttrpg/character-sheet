# Nexus Character Sheet — A Cross-VTT Character Synchronization Platform for PF2e / SF2e

**Status:** Architecture proposal (pre-implementation)
**Core language:** Rust, with a deliberately minimal JavaScript surface where target platforms force it (§6, §7)
**Scope:** Canonical character model, sync engine, adapter architecture, Roll20 + Foundry adapters, API, storage, and roadmap.

---

## 0. Guiding Principle

Every design decision below answers one question: **"Does this make it easier to add the next adapter, without touching the engine?"** If a choice makes the Roll20 adapter simpler but couples the engine to Roll20's data shape, it's the wrong choice. The project's long-term value isn't the Roll20 or Foundry adapter — it's the canonical model and the diff/merge engine sitting between them. Adapters are cheap and disposable; the core must not be.

This is why the project is pitched as "Git for TTRPG characters" rather than "a Roll20-Foundry sync tool." Git doesn't know what a file *means*; it knows how to diff and merge byte streams and delegates meaning to the tools built on top. Nexus Character Sheet doesn't know what Roll20 or Foundry *are*; it knows how to diff and merge a canonical character graph, and delegates platform meaning to adapters.

### 0.1 The language boundary: Rust core, TypeScript adapters

Language choice follows contribution patterns, not preference. Two observations drive the split:

1. **The merge engine is written once and must be right.** A subtly wrong merge silently corrupts a character someone spent months building, and users cannot eyeball a diff to catch it. This code is rarely touched by outside contributors and benefits enormously from exhaustive `match`, no null, and property testing.
2. **Adapters are written many times, mostly by other people.** TTRPG tooling is overwhelmingly a JavaScript ecosystem — Foundry modules, Roll20 Mod Scripts, and every existing builder integration are JS. Adapters are the contribution this project most needs from outsiders, and requiring a Rust toolchain to write one would suppress exactly the contributions that matter most.

So:

| Surface | Language | Rationale |
|---|---|---|
| Sync engine, canonical schema, store, API, CLI | **Rust** | Correctness-critical, rarely contributed to, benefits from the type system and single-binary distribution. |
| Adapter SDK + all adapters | **TypeScript** | The contribution surface. Familiar tooling, no Rust toolchain required, publishable to npm. |
| Roll20 in-sandbox collector | **TypeScript → plain JS** | <cite index="5-1">Roll20 Mod Scripts are written in JavaScript</cite>; now authored in the same language as its adapter. |
| Foundry relay module | **TypeScript** | Foundry is <cite index="14-1">a Node.js web application</cite> with a JS module system — and now shares mapping code with the Foundry adapter directly. |

**A contributor writing a new adapter never installs Rust.** Their loop is `npm install`, `npm test`, open a PR. That is the single most important property of this design, and every decision in §5, §10, and §15 defers to it.

This also removes complexity an all-Rust design carried: an earlier draft compiled the Foundry mapping crate to WASM so the browser-side module and the server could share it. With TS adapters, that sharing is just an import. The `wasm-bindgen` toolchain, the dual-target build, and the WASM parity test suite all disappear.

### 0.2 What this costs, honestly

The Rust footprint is now narrower than a Rust-everywhere design implies: essentially the merge engine, the canonical schema, storage, and a thin HTTP layer. Against that, the split introduces two real costs:

- **A cross-language schema boundary** requiring codegen and a CI staleness check (§3.6).
- **An adapter host** that must execute TS from a Rust process (§5.5).

There is a coherent position that says the remaining Rust core is small enough that all-TypeScript would be simpler overall, and that position is not wrong. The split is worth it if you weight the merge engine's correctness guarantees highly — property-tested pure functions over an exhaustive conflict enum, catching interleaving bugs that example-based tests structurally cannot. That is the bet. It is worth revisiting if the codegen boundary or the adapter host turn out to consume more maintenance attention than the engine's correctness saves.

---

## 1. Overall Architecture

```
╔══════════════════════════ RUST CORE ═══════════════════════════╗
║                    ┌─────────────────────────┐                  ║
║                    │   Nexus Character Sheet API        │                 ║
║                    │   (Axum, PostgreSQL)     │                 ║
║                    └────────────┬─────────────┘                 ║
║             ┌───────────────────┼───────────────────┐           ║
║   ┌─────────▼────────┐ ┌────────▼────────┐ ┌────────▼────────┐  ║
║   │  sync-engine      │ │ canonical store │ │ adapter-host    │  ║
║   │  diff/merge/DAG   │◄►│ snapshot DAG,  │ │ embedded JS rt, │  ║
║   │  [pure, no I/O]   │ │ hashes, SQLx    │ │ capability grant│  ║
║   └───────────────────┘ └─────────────────┘ └────────┬────────┘  ║
╚══════════════════════════════════════════════════════╪══════════╝
                                                        │
                    generated types ▲                   │ host ops:
                    (schema → TS)   │                   │ fetch, log
╔═══════════════════════════════════╪═══════════════════▼══════════╗
║                        TYPESCRIPT ADAPTERS                        ║
║   ┌──────────────────────────────────────────────────────────┐   ║
║   │        @nexus-ttrpg/adapter-sdk                            │  ║
║   │   types · CharacterAdapter interface · contract harness    │  ║
║   └───┬──────────┬──────────────┬──────────────┬──────────────┘  ║
║   ┌───▼───┐ ┌────▼────┐ ┌───────▼──────┐ ┌─────▼───────┐         ║
║   │roll20 │ │ foundry │ │ pathbuilder  │ │ wanderer's  │  ...    ║
║   └───┬───┘ └────┬────┘ └───────┬──────┘ └─────┬───────┘         ║
╚═══════╪══════════╪══════════════╪══════════════╪═════════════════╝
        │ signed   │ HTTP; module │ JSON export  │ share-link API
        │ payloads │ shares code  │              │
   ┌────▼─────┐ ┌──▼───────────┐ ┌▼───────────┐ ┌▼────────────┐
   │  Roll20  │ │Foundry world │ │Pathbuilder │ │ Wanderer's  │
   │ sandbox  │ │+ relay module│ │            │ │ Guide       │
   └──────────┘ └──────────────┘ └────────────┘ └─────────────┘
```

Key properties:

- **Hub-and-spoke, not mesh.** Every platform talks only to the canonical model. O(n) adapters instead of O(n²) point-to-point integrations, and diff/merge is written and tested exactly once.
- **The engine has zero platform dependencies, and now it's structurally impossible to violate.** In an all-Rust design this was enforced by Cargo's dependency graph. Here it's stronger: adapters are in a different language, in a different package manager, running in a sandboxed runtime that only reaches the core through a narrow op interface. There is no import an adapter author could write that would reach into `sync-engine`.
- **`sync-engine` is pure and I/O-free.** Owned data in, owned data out — no database handles, no HTTP clients, no `async`. Trivially unit- and property-testable (§16), reusable unchanged in server, CLI, and (compiled to WASM) a browser-side diff preview.
- **Adapters are the only place platform knowledge lives.** If you need to know a Roll20 attribute name, you're in the wrong package unless it's `adapters/roll20`.
- **The core never trusts adapter output.** Everything crossing the boundary upward is validated against the canonical schema in Rust before it reaches the engine (§5.5). An adapter is untrusted community code, and the type checker on the TS side is a convenience for its author, not a guarantee to the host.

---

## 2. Why Explicit Sync Instead of Real-Time Sync

Real-time (continuous, bidirectional, live) sync sounds more impressive, but it's the wrong model here, for reasons specific to this domain:

1. **VTT session state and canonical build state have different lifecycles.** A GM's Foundry actor accrues damage, conditions, and temporary buffs mid-combat. None of that should silently propagate to a player's Pathbuilder export or vice versa. Real-time sync makes it very hard to keep session state and build state cleanly separated, because every write anywhere immediately becomes a write everywhere.
2. **Conflicting simultaneous edits are common and expected, not exceptional.** A player might respec a feat in Wanderer's Guide while their GM adjusts the same character's gear in Foundry for a houserule. Real-time sync forces last-writer-wins or operational-transform complexity to resolve this instantly; explicit sync lets a human look at a diff and decide.
3. **Not all platforms *can* be pushed to.** Roll20's sandbox is unreachable from outside (§6.1) and <cite index="7-1">shuts down on campaign inactivity</cite> — it isn't even running most of the time. Foundry is normally self-hosted behind someone's home network, frequently offline. There is no reliable transport that would make "real time" meaningful across this platform set without requiring every user to expose a public endpoint.
4. **Determinism and auditability.** An explicit "Push v14 → Roll20" action produces one diff, one snapshot, one rollback point. Continuous sync produces an unbounded stream of micro-changes that's nearly impossible to reason about or roll back cleanly.
5. **Cost and complexity.** Real-time sync needs persistent connections, presence tracking, and conflict resolution running continuously per character per platform. Explicit sync is a batch job that runs on user action or schedule — dramatically simpler infrastructure, and the natural fit for a project that will largely be self-hosted.

The tradeoff we accept: a player must remember to Push/Pull. We mitigate with **change detection and notifications** (§5.3, §18) rather than automatic propagation — the system tells you "your Foundry actor has diverged from canonical by 3 fields" and lets you decide, rather than doing it for you.

---

## 3. Canonical Character Model

### 3.1 Design approach

In the TypeScript version of this design, Zod was the single source of truth from which types, validation, and JSON Schema were all derived. Rust inverts that relationship: **the Rust types are the source of truth**, and everything else is derived from them.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Character {
    pub meta: CharacterMeta,        // identity, versioning, ownership
    pub build: CharacterBuild,      // permanent, sync-eligible build data
    pub session: SessionState,      // ephemeral, per-platform, NOT auto-synced (§4)
    pub extensions: ExtensionBag,   // unknown/future/homebrew fields, namespaced
}
```

- `serde` handles serialization and *structural* validation (types, required fields, unknown-field rejection) at deserialize time.
- `schemars` (`JsonSchema` derive) generates JSON Schema from the same types, which feeds the OpenAPI spec (§13) and is published as an artifact so third-party tooling in any language can validate canonical documents without linking Rust.
- `garde` handles *semantic* validation that the type system can't express (level between 1 and 20, ability scores within legal bounds, "a spellcasting entry with tradition `prepared` must have a slot table").

This three-layer split is deliberate. In Zod all three collapsed into one artifact, which was convenient but meant every validation concern was runtime. Here, the first layer is compile-time-ish (deserialization is total — if you have a `CharacterBuild`, it's structurally valid by construction), and only genuinely runtime concerns reach `garde`. The practical effect on the engine: `sync-engine` never has to defensively check whether a field it was handed exists.

**`deny_unknown_fields` and the extension bag interact deliberately.** Strict rejection applies to the *typed* regions of the schema, so a malformed adapter payload fails loudly rather than silently dropping data. Anything genuinely unmodeled must go through `extensions` (§3.5), where it's explicitly namespaced and visible, rather than being quietly tolerated in the core schema.

The model splits into three top-level partitions so the sync engine can apply a blanket rule ("diff/merge `build`, never touch `session` automatically") instead of maintaining a field-by-field allowlist that has to be updated every time someone adds a field. The partition *is* the policy — and in Rust it's enforced by the function signature: `merge()` takes `&CharacterBuild`, so it is not *able* to see `session`.

### 3.2 `meta`

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterMeta {
    pub id: Ulid,
    pub schema_version: semver::Version,
    pub system: SystemId,            // "pf2e" | "sf2e"; extensible via plugin registry (§10)
    pub owner_id: Ulid,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub content_hash: ContentHash,   // BLAKE3 of canonicalized `build` (§9.2)
}
```

### 3.3 `build` (permanent, synchronizable)

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterBuild {
    pub identity: Identity,
    pub ancestry: Ancestry,
    pub heritage: Heritage,
    pub background: Background,
    pub class: ClassEntry,
    pub level: Level,                                  // newtype, 1..=20 enforced on construction
    pub ability_scores: BTreeMap<AbilityScoreKey, AbilityScoreEntry>,
    pub skills: BTreeMap<SkillKey, SkillEntry>,        // incl. lore skills
    pub proficiencies: ProficiencyTable,
    pub feats: Vec<FeatEntry>,                         // tagged by category
    pub class_features: Vec<ClassFeatureEntry>,
    pub equipment: Equipment,
    pub currencies: Currencies,
    pub spellcasting: Spellcasting,
    pub actions: Vec<ActionEntry>,
    pub reactions: Vec<ReactionEntry>,
    pub exploration_activities: Vec<ExplorationActivityEntry>,
    pub companions: Vec<CompanionEntry>,
    pub familiars: Vec<FamiliarEntry>,
    pub drones: Vec<DroneEntry>,                       // SF2e mechanic
    pub vehicles: Vec<VehicleEntry>,
    pub notes: Vec<NoteEntry>,
}
```

Two Rust-specific choices worth justifying:

**`BTreeMap`, not `HashMap`.** Content hashing (§9.2) requires a deterministic serialization order. `HashMap` iteration order is nondeterministic by design (and randomly seeded per process), which would make the same character hash differently on two runs and produce phantom diffs. `BTreeMap` gives stable key ordering for free, so canonicalization is just "serialize it." This is a small decision with an outsized correctness impact and is easy to get wrong.

**Newtypes over primitives** (`Level`, `SkillKey`, `ContentHash`, `Ulid`) rather than bare `u8`/`String`. This closes off a whole class of adapter bugs — you cannot pass a character's level where a proficiency rank is expected, and a `ContentHash` cannot be accidentally compared to an arbitrary string. In the TypeScript design these were all `number` and `string` and the mistakes were caught, if at all, by tests.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityScoreEntry {
    pub boosts: Vec<AbilityBoost>,      // ancestry, background, class, free
    pub flaws: Vec<AbilityBoost>,
    pub item_bonus: i8,
    // NOTE: the final score is *derived*, not stored — see below.
}
```

`AbilityScoreEntry` deliberately stores the *components* that produce the final score rather than only the final number, because several source platforms (Pathbuilder, Wanderer's Guide) recompute the final number themselves, and a flattened "final score only" field would force each adapter to reverse-engineer the components to import correctly. Store the inputs; let `SystemPlugin::derived_stats` (§10) compute the output.

### 3.4 SF2e-specific accommodations

SF2e reuses most of the PF2e chassis (three-action economy, proficiency ranks, four degrees of success) with a few structural differences the schema carries:

- **Resolve Points** alongside Hero Points — modeled in `session`, not `build` (§4).
- **Drones** — mechanic class feature, structurally similar to companions but with chassis/mod slots instead of a leveling table.
- **Credits/UPB** instead of gp/sp/cp. Modeled as one `Currencies` struct with optional fields per denomination rather than an enum over "PF2e currency" and "SF2e currency", so a crossover campaign carrying both isn't structurally impossible. (An enum would be more type-safe but would make the perfectly legitimate "my PF2e character found a credstick" case unrepresentable — the wrong trade here.)
- **Vehicles and starship roles** — kept lightweight (role + relevant skill). The starship itself is a party-level resource, not a character-level one, and is out of scope for v1.

### 3.5 `extensions` — the escape hatch

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ExtensionBag(pub BTreeMap<Namespace, serde_json::Value>);
// e.g. "foundry.pf2e" -> { "flags": {...} }
//      "homebrew.myGuild" -> { "customTrait": "..." }
```

Any adapter or plugin can stash platform-specific or homebrew data here, namespaced. The sync engine treats values as opaque `serde_json::Value`: it diffs them structurally (so conflicts are still detected) but applies no schema validation beyond "is valid JSON." This is the pressure-release valve that keeps the core schema from needing a breaking change every time Foundry adds an actor flag — and it's why `deny_unknown_fields` on the typed regions is safe rather than hostile.

### 3.6 Crossing the language boundary: one source, generated downstream

The canonical model must exist in both languages without drift. Two schema definitions maintained by hand would diverge within weeks, and the divergence would show up as adapters silently producing data the core rejects. So the pipeline is strictly one-directional:

```
crates/schema (Rust types)          ← the only hand-written definition
        │ schemars
        ▼
  canonical.schema.json             ← published artifact, versioned
        │
        ├─► json-schema-to-typescript ─► @nexus-ttrpg/schema-types  (TS types)
        ├─► json-schema-to-zod         ─► @nexus-ttrpg/schema-zod   (runtime validation)
        └─► consumed directly by any third-party tooling, in any language
```

Run via `cargo xtask codegen`. The generated output is **committed to the repo**, and CI regenerates and fails on any diff. Committing generated code is usually a smell, but here it earns its place: a TS contributor can clone, `npm install`, and have working types without a Rust toolchain anywhere in their loop, which is the whole point of §0.1. A CI staleness check gives the correctness property that not committing would have given, without imposing Rust on adapter authors.

Adapters are also free to ignore the generated Zod validators for speed and rely on host-side validation (§5.5) — they're a development aid for fast local feedback, not the enforcement point. Enforcement is always in Rust, always at the boundary.

**Why not TypeScript as the source of truth, with Rust generated from it?** Because the Rust side needs things JSON Schema cannot express — newtypes with construction-time invariants (`Level` bounded 1–20), `BTreeMap` ordering guarantees that make content hashing deterministic (§9.2), and exhaustive enums the compiler checks. Generating Rust from JSON Schema would produce a flat, `String`-and-`i64` shaped model and forfeit precisely the guarantees the Rust core exists to provide. Generation loses information, so it must run in the direction of the less expressive target.

---

## 4. Session State — What Never Auto-Syncs

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub platform: PlatformId,
    pub hit_points: HitPoints,                       // current, temporary, max
    pub spell_slots: BTreeMap<SlotKey, SlotUsage>,
    pub focus_points: PointPool,
    pub resolve_points: Option<PointPool>,           // SF2e
    pub hero_points: u8,
    pub conditions: Vec<ConditionInstance>,          // incl. value, e.g. frightened 2
    pub initiative: Option<i16>,
    pub temporary_modifiers: Vec<TemporaryModifier>,
    pub encounter_effects: Vec<EncounterEffect>,
    pub last_observed_at: DateTime<Utc>,
}
```

**Why these fields live outside canonical sync, structurally rather than by convention:**

1. **They're scene-scoped, not character-scoped.** Current HP belongs to *this session, this VTT instance*. If a player has the same character in Foundry for one table and Roll20 for a one-shot with a different group, those HP totals are legitimately different things, not two copies of one fact.
2. **They change too fast for diff/merge to be meaningful.** Conditions and HP change dozens of times per session. Treating them as sync targets would make every Push/Pull dominated by session noise, burying the build changes the engine actually needs to surface.
3. **VTTs are already good at this.** Foundry and Roll20 both have mature, real-time-appropriate systems for HP/conditions/initiative during play. Duplicating that would be strictly worse than what exists and would fight the VTT's own UI.
4. **Session state has no canonical "correct" value to converge on.** Build data has a truth (the sheet as designed); mid-combat HP does not — it's whatever happened at the table. Merge resolution assumes a reconcilable single truth; that assumption is false here.

The API exposes session state as read-only telemetry (`GET /characters/:id/session?platform=foundry`) so the UI can show "Foundry currently reports 12/40 HP, 2 conditions" — visibility only, never sync input. Enforced by types: the ingest path deserializes session data into a `SessionTelemetry` struct that has no conversion into `CharacterBuild`.

---

## 5. Adapter Architecture

### 5.1 The interface (TypeScript)

This is the surface community contributors write against. It is published as `@nexus-ttrpg/adapter-sdk` and is the only part of the system an adapter author needs to understand.

```ts
import type { CharacterBuild, PartialBuild } from "@nexus-ttrpg/schema-types";

export interface CharacterAdapter<TPlatform = unknown> {
  readonly id: string;                       // "roll20" | "foundry" | ...
  readonly capabilities: AdapterCapabilities;

  /** Platform payload -> canonical. Never throws on unknown content;
   *  unmapped data goes to `extensions` and is reported in warnings. */
  import(raw: TPlatform, ctx: ImportContext): Promise<ImportOutcome>;

  /** Canonical -> platform payload. */
  export(build: CharacterBuild, ctx: ExportContext): Promise<ExportOutcome<TPlatform>>;

  /** Cheap drift check — no full import/merge cycle. */
  detectChanges(previous: TPlatform | null, current: TPlatform): Promise<ChangeSet>;

  /** Structural + platform-specific validation. */
  validate(raw: TPlatform): Promise<ValidationReport>;
}

export interface ImportOutcome {
  build: PartialBuild;
  warnings: AdapterWarning[];   // e.g. unrecognized feat, preserved in extensions
  unmapped: string[];           // canonical field paths
}

export interface ExportOutcome<T> {
  payload: T;
  warnings: AdapterWarning[];   // e.g. target has no `homeland` field, dropped
  skipped: string[];
}
```

Note the interface is **generic over the platform format** (`CharacterAdapter<TPlatform>`), which the all-Rust draft could not do — a generic trait isn't object-safe, so the registry couldn't hold `Box<dyn CharacterAdapter>` and everything had to funnel through an untyped `RawPayload`. TypeScript has no such constraint, so an adapter author gets full type inference over their own platform's shapes with no boilerplate. This is a small but genuine ergonomic win from putting adapters in TS, and it lands exactly where ergonomics matter most: the code strangers write.

### 5.2 Errors and warnings are different, on purpose

```ts
export class AdapterError extends Error {     // the operation could not proceed
  constructor(readonly kind: AdapterErrorKind, message: string) { super(message); }
}

export type AdapterErrorKind =
  | { type: "transport"; status?: number }
  | { type: "auth"; detail: string }
  | { type: "unsupportedSheetVersion"; found: string; supported: string[] }
  | { type: "malformedPayload"; detail: string };

export interface AdapterWarning {             // the operation succeeded, with caveats
  path: string;
  kind:
    | { type: "unmappedField" }
    | { type: "lossyExport"; reason: string }
    | { type: "fuzzyMatch"; confidence: number };
  detail: string;
}
```

Homebrew content (§7.7) must always be a warning, never an error. An adapter that throws on unrecognized content is unusable in real Foundry worlds, which are full of it. The SDK's contract tests (§16) assert this directly: feeding an adapter a fixture containing unknown content must produce warnings and a successful result, not a rejection.

### 5.3 `detectChanges` is a first-class operation

Separated from `import` so the host can cheaply answer "has anything changed since we last synced?" without a full import/merge cycle — feeding the drift-check job (§18) that surfaces notifications without mutating anything.

### 5.4 Adapters are stateless

Adapters own no persistent state; all state lives in the canonical store (§9). They run as plain objects in tests, so an adapter's unit tests need no database, no host, and no network. Credentials arrive in `ImportContext`/`ExportContext` from the connection record (§13.3) and are never held by the adapter.

```ts
export interface AdapterCapabilities {
  supportsImport: boolean;
  supportsExport: boolean;
  supportsPartialExport: boolean;
  supportsSessionRead: boolean;
  fieldCoverage: readonly string[];   // canonical paths this adapter round-trips
}
```

`fieldCoverage` is a static declaration, readable without instantiating the adapter or touching the network, so the UI can render an honest "Roll20 covers 78% of this character; these 4 fields will be dropped" preview *before* the user commits to a push.

### 5.5 How the Rust core runs TypeScript adapters

The adapter host embeds a JavaScript runtime (`deno_core` — V8 plus a custom op layer, without the full Deno runtime) inside the Rust process. Adapters are loaded as ES modules and invoked through the interface above.

This keeps the single-binary property from §12: a self-hoster still deploys one file, with adapter bundles alongside it. It does **not** require Node on the server.

```rust
pub struct AdapterHost { runtime: JsRuntime, manifest: AdapterManifest }

impl AdapterHost {
    pub async fn import(&mut self, raw: serde_json::Value, ctx: ImportContext)
        -> Result<ImportOutcome, HostError>
    {
        let result = self.call_adapter("import", &[raw, ctx.into()]).await?;
        // The boundary check: adapter output is untrusted and validated here,
        // in Rust, against the canonical schema — before the engine ever sees it.
        let outcome: ImportOutcome = serde_json::from_value(result)?;
        self.schema.validate_partial(&outcome.build)?;
        Ok(outcome)
    }
}
```

**Capabilities are granted, not ambient.** An adapter gets no filesystem access and no global `fetch`. Network access is a host op, scoped by an allowlist in the adapter's manifest:

```json
{
  "id": "pathbuilder",
  "entry": "./dist/index.js",
  "capabilities": { "fetch": { "allow": ["https://pathbuilder2e.com"] } }
}
```

This matters more here than it would in an all-TypeScript system. Adapters are community code running on servers that hold other people's characters and platform credentials; in a Node host they would have ambient access to the filesystem, the environment, and the network by default. Here a malicious or merely careless adapter cannot read `/etc/passwd`, cannot exfiltrate to an arbitrary host, and cannot reach the database. **The security posture that an all-Rust design only got by deferring plugins to a future WASM runtime, this design gets on day one** — which is a large part of why the split is worth its costs.

**Escape hatch for adapters that genuinely need full Node.** An adapter may declare `"runtime": "external"` and ship as a subprocess speaking JSON-RPC over stdio, implementing the identical interface. It loses in-process performance and forces the operator to have Node installed, so it's documented as a last resort — but it means a platform requiring some Node-only library is never architecturally blocked. Adapters written in other languages can use the same protocol.

---

## 6. Roll20 Adapter

### 6.1 Constraints

Roll20's extensibility surface here is **Mod Scripts (the API sandbox)**, <cite index="5-1">a Pro-subscriber feature whose scripts are written in JavaScript</cite>. Architecturally, <cite index="9-1">Roll20 runs a server-side virtual machine per campaign, providing a sandbox where custom scripts run without affecting other users' campaigns and preventing a malicious GM from writing scripts that could harm players</cite>. That security posture constrains us: no compiled code, no subprocesses, outbound network limited to a gated `HttpRequest`.

With adapters in TypeScript, the collector and its adapter are now the same language — but they should still be different *artifacts*, and the reason is not language, it's deployment:

```
Roll20 sandbox (~150 lines, no logic)              Nexus Character Sheet server
┌──────────────────────────────┐                   ┌──────────────────────────────┐
│ nexus-collector.js            │ raw attribute JSON│ adapters/roll20 (TS)          │
│  - reads Character/Attribute  │ ─────────────────►│  - mapping table              │
│    objects verbatim           │(HttpRequest,signed)│ - parsing + validation       │
│  - on !nexus-pull, POSTs dump │ ◄─────────────────│  - runs in adapter-host       │
│  - on !nexus-apply, writes    │ signed, pre-mapped│                               │
│    attrs verbatim             │ attribute payload  │  ── validated in Rust ──►     │
└──────────────────────────────┘                   │      sync-engine              │
                                                    └──────────────────────────────┘
```

**Why keep the collector logic-free even though it could now share code with the adapter?** Because updating it requires every GM to re-paste a script into their campaign. Any logic that lives there is effectively frozen at whatever version each user pasted, possibly years ago. Mapping rules change often — Roll20 sheet authors rename attributes, and the PF2e sheet evolves. Keeping all of it server-side means a mapping fix ships instantly to every user; putting it in the collector would mean a support burden of "which script version are you running?" forever.

This is a deployment-topology argument, not a language argument, and it would hold identically if the whole system were TypeScript. Worth stating explicitly, because the temptation to share code across that boundary now that it's all TS is real and should be resisted.

### 6.2 The sandbox is not always running

<cite index="7-1">Roll20 shuts the sandbox down on campaign inactivity</cite>, and <cite index="6-1">it won't spin up when nobody is logged into the game</cite>. This kills any design that assumes scheduled polling from inside Roll20. The consequences are load-bearing:

- Roll20 sync is **always user-initiated from inside an active game session**. There is no "sync my Roll20 character nightly" feature, and promising one would be dishonest.
- `detect_changes` for Roll20 cannot run server-side on a timer. The `drift` endpoint (§13.3) returns `Unavailable { reason: SandboxNotRunning }` for Roll20 connections rather than a stale "no changes" — an honest unknown beats a confident wrong answer.
- The collector re-announces itself on sandbox start, so the server learns when a campaign becomes reachable and can surface "Roll20 is online, sync now" to the user.

### 6.3 Sheet field mapping (TypeScript)

Roll20 sheets store data as flat `Attribute` rows keyed by name (`strength_score`, `repeating_skills_-abc123_skill_name`), plus repeating sections addressed by generated row IDs — very different from canonical's nested graph. Handled in the adapter as declarative data, not imperative field-by-field code:

```ts
import type { CharacterBuild } from "@nexus-ttrpg/schema-types";
import { canonicalPath } from "@nexus-ttrpg/adapter-sdk";

type MappingTarget =
  | { kind: "attr"; name: string }
  | { kind: "repeating"; section: string; rows: (b: CharacterBuild) => RepeatingRow[] };

export const ROLL20_PF2E_MAP = [
  { path: canonicalPath("build.abilityScores.str"), target: { kind: "attr", name: "strength_score" } },
  { path: canonicalPath("build.identity.name"),     target: { kind: "attr", name: "character_name" } },
  { path: canonicalPath("build.level"),             target: { kind: "attr", name: "level" } },
  { path: canonicalPath("build.skills"),            target: { kind: "repeating", section: "repeating_skills", rows: skillRows } },
  { path: canonicalPath("build.feats"),             target: { kind: "repeating", section: "repeating_feats", rows: featRows } },
] as const satisfies readonly FieldMapping[];
```

A declarative table means:
1. A Roll20 sheet author renaming an attribute is a one-line data edit — the most common maintenance task on this adapter, and now a change any contributor can make and test without a Rust toolchain.
2. `fieldCoverage` (§5.4) is derived from the table rather than hand-maintained, so it can't silently drift out of date.
3. `canonicalPath()` is typed against the generated schema types (§3.6), so a typo is a **type error at build time**, not a silently-skipped field. This preserves the compile-time guarantee the Rust version had, via template-literal types over the generated model, and is the main reason the generated types are worth committing.

Note what did *not* survive the move to TS: the Rust version could assert at build time that mapping paths exist in the schema via a build script. The TS version gets the equivalent from `canonicalPath()`'s type signature, which is comparable in strength but relies on contributors not reaching for `as any`. The contract tests (§16) backstop this by asserting every declared `fieldCoverage` path round-trips on a real fixture.

### 6.4 Other limitations to design around

- **Repeating row IDs are opaque and regenerate** on some sheet operations, so the parser matches rows by content fingerprint (feat name + source), not row id, and treats id churn as a non-change.
- **Sheet versions vary.** `validate()` checks a `sheet_version` attribute the collector always includes and returns `UnsupportedSheetVersion` rather than attempting a best-effort mapping that would silently corrupt a sheet.
- **Execution time is limited per tick** and the sandbox restarts periodically, so large exports are chunked across multiple `!nexus-apply` invocations, with the server tracking which chunks have been acknowledged.

### 6.5 Authentication & deployment

- A **per-campaign install token** is issued when a GM connects a Roll20 game, embedded in the one-time collector script pasted into the Mod Scripts panel.
- The collector authenticates outbound calls with a **short-lived signed JWT** obtained via a `!nexus-auth` handshake, not a long-lived secret — script source is visible to anyone with GM access and gets copy-pasted around.
- Deployment is "copy-paste a script," Roll20's only distribution mechanism. `adapters/roll20/collector/nexus-collector.js` is versioned in-repo, deliberately unminified and unbuilt so a suspicious GM can read all 150 lines before pasting it into their game. **Auditability is a feature here, not laziness** — asking users to paste opaque code into a system with access to their campaign would be an unreasonable ask.

---

## 7. Foundry Adapter

Foundry is the richer target: <cite index="14-1">a Node.js web application run locally by the GM or hosted on a remote server, which players connect to over the network using their browser</cite>, with a real module system and a document/embedded-document model that maps far more naturally onto canonical than Roll20's flat attributes.

Correcting an earlier draft of this document: a Foundry **module can expose local HTTP endpoints itself** — there is prior art in community modules that provide <cite index="18-1">local-only REST endpoints for actors, items, scenes, and world data, authenticated with a generated API key</cite>. No separate companion process is needed, which removes a whole deployment step for self-hosters.

### 7.1 Shape of the integration

Foundry's client runs in a browser and its modules are JavaScript, so with adapters in TypeScript the mapping logic is shared by **plain import** — no WASM toolchain, no dual-target build, no parity test suite:

```
              packages/foundry-mapping (TS)
              └── the actual logic, written once
                          │
        ┌─────────────────┴─────────────────┐
        │                                    │
  adapters/foundry                    foundry relay module
  (runs in adapter-host,              (runs in the GM's browser,
   server-side)                        maps Actor↔canonical locally,
                                       works fully offline)
```

An earlier all-Rust draft achieved this same sharing by compiling a Rust crate to WASM with `wasm-bindgen`, plus a test suite asserting the native and WASM builds produced identical output. That machinery exists only to move Rust into a JS runtime. With the adapter already in TS, it's an import statement. This is the clearest concrete win from the split, and it removes an entire category of build complexity and a whole test layer from §16.

The relay module (`nexus-character-sheet`) then:
1. Registers authenticated local HTTP endpoints, keyed by an API key generated in module settings.
2. Hooks `Actor` create/update to maintain drift telemetry (§5.3) — still never auto-syncing (§4), but drift checks don't need to poll.
3. Imports `foundry-mapping` directly to convert Actors to canonical in-browser.

The intricate part of this adapter is the PF2e/SF2e mapping rules, where a subtle bug quietly corrupts characters. Those rules exist in exactly one module, exercised by one test suite, running identically server-side and in a GM's browser.

### 7.2 Actor import/export

Foundry PF2e/SF2e Actors are documents with a `system` data object (DataModel-validated in current versions) plus an `items` collection of embedded `Item` documents — feats, weapons, spells, and class features are *all* items, distinguished by `item.type`.

```ts
export function exportActor(build: CharacterBuild, system: SystemId): FoundryActor {
  return {
    name: build.identity.name,
    type: "character",
    system: {
      abilities: mapAbilityScores(build.abilityScores),
      // max HP only — current HP is session state and is never written (§4)
      attributes: { hp: { max: derivedMaxHp(build) } },
      details: { level: { value: build.level }, ancestry: refOrText(build.ancestry) },
    },
    items: [
      ...build.feats.map(featToItem),
      ...build.equipment.weapons.map(weaponToItem),
      ...build.spellcasting.entries.flatMap(spellcastingToItems),
    ],
  };
}
```

Import is the inverse: group `actor.items` by `item.type` and reconstruct canonical collections, using each item's `system.slug` and `system.source` where present.

### 7.3 Compendium lookups and UUID handling

Foundry items usually originate from compendium packs and carry a **UUID** (`Compendium.pf2e.feats-srd.Item.AbC123XyZ`). The adapter:

- Stores it in `SystemRefId::foundry_uuid` (§9.1) when an imported item resolves to a compendium source.
- On export, if a canonical entry has a UUID resolvable in the *target world's* module set, exports a **compendium link** (a minimal stub Foundry resolves) rather than an inlined item — smaller payloads, and the item tracks upstream errata automatically.
- Otherwise falls back to a **fully inlined item** built from canonical data, tagged `extensions["foundry.pf2e"].inlined_fallback = true` so a later sync can distinguish "linked" from "manually inlined."

### 7.4 Offline and direct world access — with a hard caveat

Since v11, <cite index="12-1">Foundry migrated database persistence from NeDB to LevelDB</cite>. It is tempting to have a Rust sidecar read the world database directly (Rust has solid LevelDB/RocksDB bindings), and for a fully offline self-hosted flow that's attractive. **But this must be done carefully:** <cite index="11-1">LevelDB manages an intricate structure of files through its own API, and modifying those files outside that managed process will lead to database corruption</cite>.

So direct world access is supported **only** under these conditions, and the CLI enforces them:
- The world must be **shut down**, verified by lock-file check, before any direct read or write.
- Round-tripping goes through Foundry's own tooling, not raw LevelDB writes: <cite index="15-1">Foundry provides a command-line application that unpacks LevelDB databases into JSON or YAML and re-packs them</cite>. The CLI shells out to it rather than reimplementing the format.
- Writes are taken only after a backup of the world directory, with the backup path reported to the user.

For a *running* world, the relay module (§7.1) is the only supported path. This is a case where the more elegant Rust-native option is the wrong one, and the design defers to the platform's own tooling.

### 7.5 Embedded documents beyond items

Actors also carry **ActiveEffects** (buffs/conditions — session state, not synced), and for companions/familiars/eidolons, separate linked Actor documents. A companion is treated as its own mini sub-tree: `CompanionEntry` carries enough to round-trip a simplified companion Actor, referencing the parent character's canonical id.

### 7.6 Validation

Validation runs Foundry's *own* DataModel validators where possible: the relay module hands a constructed `system` object to the running world's PF2e/SF2e Actor schema and returns native validation errors, translated into `ValidationReport`. This catches system-specific rules (a spellcasting entry missing a required tradition) that canonical-only validation wouldn't know about — and it means we don't have to reimplement the PF2e system's rules in Rust to stay correct as the system updates.

### 7.7 Homebrew support

Foundry worlds routinely add homebrew via world-level compendia or unlinked items, so "doesn't match a known slug" is never an error — it's the expected extensibility path. Homebrew items round-trip as inlined items with `extensions["foundry.pf2e"].homebrew = true`, preserving their `system` data opaquely, so Foundry→Foundry round trips are lossless even though a Roll20 export of the same item is necessarily lossy (reported via `ExportOutcome::warnings`).

---

## 8. Future Adapters (Pathbuilder, Wanderer's Guide, Demiplane)

None require engine changes, because the engine only ever calls the five trait methods (§5.1). What differs is purely *how* each adapter talks to its platform:

| Platform | Integration shape | Adapter-specific concern |
|---|---|---|
| **Pathbuilder** | JSON export / share-code | Its JSON is already close to canonical's shape, so mapping is mostly renaming. No write-back API, so `supports_export = false` until one exists. |
| **Wanderer's Guide** | Share-link character API | Read-heavy like Pathbuilder. Richer homebrew/variant support, so expect heavier use of `extensions["wanderersguide"]`. |
| **Demiplane** | No official public API | **Import-only, best-effort**, behind a feature flag, `supports_export = false`, with a clear disclaimer. Do not couple a release to an unofficial integration's stability. |

The pattern that generalizes: **most third-party builders are import sources, not export targets**, because they're products with their own roadmaps that never designed for external writes. `AdapterCapabilities` already models import and export independently, so this asymmetry isn't a special case — it's the default the trait was built for.

A note on contributions: adapters for these platforms are the most likely community contributions, and they're also where the Rust choice costs the most (§21). Mitigating that is a first-class concern, not an afterthought — see the WASM plugin path in §10.

---

## 9. Data Mapping, Versioning, and Schema Evolution

### 9.1 `SystemRefId` — the reference resolution primitive

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemRefId {
    pub slug: Slug,                                   // cross-platform anchor, e.g. "fireball"
    pub source: Option<String>,                       // rulebook, e.g. "Player Core"
    pub foundry_uuid: Option<String>,
    pub roll20_id: Option<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, String>,              // open for future platforms
}
```

`slug` is the cross-platform anchor. A curated, versioned **system content index** maps slugs to canonical rules data, so an adapter with no native ID can resolve by slug plus fuzzy name match — with match confidence surfaced as `WarningKind::FuzzyMatch { confidence }` rather than silently accepted. `#[serde(flatten)]` on `other` means a new platform's ID field round-trips through older builds of the software without data loss.

### 9.2 Content hashing

`meta.content_hash` is a **BLAKE3** hash of a canonicalized (stable key order, no whitespace) serialization of `build` only — not `session`, not `meta`. BLAKE3 over SHA-256 because hashing happens on every write and every drift check across potentially thousands of characters, and it's several times faster with no security downside for a non-adversarial integrity check.

Determinism is the whole game here, which is why §3.3 mandates `BTreeMap`. The canonicalization function is a single choke point with property tests asserting `hash(x) == hash(roundtrip(x))` for arbitrary generated characters (§16).

### 9.3 Schema versioning and migrations

- `meta.schema_version` is semver. **Minor/patch** bumps are additive-only (new `Option` fields) and need no migration. **Major** bumps may restructure and ship with a registered migration.
- Every stored snapshot keeps its original version; migrations run **lazily on read**, producing the current version in memory, optionally persisted back by a compaction pass — rather than a synchronous mass migration on deploy. Critical for self-hosters who skip upgrades for months.
- Migrations are pure functions chained automatically (`v1→v2→v3`), so we never hand-write every version pair:

```rust
pub trait Migration {
    const FROM: semver::Version;
    const TO: semver::Version;
    fn migrate(input: serde_json::Value) -> Result<serde_json::Value, MigrationError>;
}
```

Migrations operate on `serde_json::Value`, not typed structs — deliberately. A typed migration would need the *old* version's structs kept compiled in the tree forever, which becomes unmaintainable by v5. Untyped migration functions with fixture-based tests (§16) are the standard trade, and the loss of type safety is bounded because the output is immediately deserialized into the current typed schema, which fails loudly if a migration is wrong.

### 9.4 Unknown fields and extension points

Any field an adapter encounters with no canonical home is preserved, not dropped:

1. **Known-unknown** (maps to an existing concept under a different name, just unmapped): logged as `WarningKind::UnmappedField` pointing at the exact source field — actionable enough to become an easy first contribution.
2. **Genuinely platform/homebrew-specific**: stored in `extensions[adapter_id]`, round-trip lossless for platforms that understand the namespace, degraded with a warning for those that don't.

This two-tier approach is what lets canonical tolerate future game updates: a Remaster-style rules change can land in `extensions["pf2e.remaster"]` immediately, get used by adapters right away, and be promoted into the core schema in a later version once its shape has stabilized — without blocking on a schema RFC first.

---

## 10. Plugin System

The long-term goal ("D&D 5e, PF1e, other RPGs, without touching the core engine") means the **ruleset** must be pluggable, not just the platform adapters. Two axes, and — importantly — they now sit on opposite sides of the language boundary, because they have very different contribution profiles.

### 10.1 Adapters: TypeScript, loaded at runtime, contributed often

Covered in §5. An adapter is an npm package implementing `CharacterAdapter`, loaded by the adapter host, sandboxed with granted capabilities. **This is the extension point that actually gets used**, and the whole design is arranged so that adding one requires no Rust, no recompilation, and no core changes.

Adding an adapter is: write a package against the SDK, pass the contract tests, register it. Nothing in the core is touched.

### 10.2 Systems: Rust, compiled in, contributed rarely

```rust
pub trait SystemPlugin: Send + Sync {
    fn id(&self) -> SystemId;
    fn json_schema(&self) -> schemars::schema::RootSchema;
    fn validate(&self, build: &CharacterBuild) -> ValidationReport;
    fn derived_stats(&self, build: &CharacterBuild) -> DerivedStats;
    fn content_index(&self) -> &ContentIndex;
}
```

System plugins own "what is a valid character in this ruleset, and how are its derived stats computed." PF2e and SF2e both extend a shared `schema-core` (ability scores, proficiency ranks, the three-action economy) rather than being one merged schema — which is what makes adding D&D 5e a matter of writing a new plugin against the same base rather than editing the PF2e model.

These stay in Rust and compiled in, for a specific reason: **a system plugin defines part of the canonical schema**, and the canonical schema is the Rust source of truth that everything else is generated from (§3.6). A TS-defined system would invert the codegen direction and forfeit the newtypes, ordering guarantees, and exhaustive enums the core depends on.

The cost is that adding a game system requires a Rust PR and a redeploy. This is acceptable because the frequency is completely different from adapters: a project supports a handful of systems and dozens of adapters. Optimizing the rare path at the expense of the common one would be backwards.

**A stretch goal softens this** (§20): expose the *rules* half of a system plugin — `derived_stats` and the content index, which is where rules-lawyer contributors would want to work — as a TS interface running in the adapter host, while the schema half stays Rust. That splits the plugin along the same line as everything else: data model in Rust, community-facing logic in TS.

### 10.3 What this design does not need

An earlier all-Rust draft required a second, future plugin runtime (`wasmtime` plus the Component Model) purely so third parties could contribute adapters without writing Rust. That is now unnecessary — the adapter host already runs TypeScript, sandboxed, from day one. A whole phase of speculative infrastructure, and its dependency on still-maturing Component Model tooling, drops out of the plan entirely.

The registry resolves `(system, platform) → adapter` at runtime, and the engine depends on neither axis directly.

---

## 11. Conflict Resolution — a Git-Inspired Merge System

### 11.1 Core structures

```rust
pub struct Snapshot {
    pub id: Ulid,
    pub character_id: Ulid,
    pub parent: Option<Ulid>,
    pub merge_parent: Option<Ulid>,        // Some(_) only for merge commits
    pub build: CharacterBuild,
    pub content_hash: ContentHash,
    pub origin: SnapshotOrigin,
    pub created_at: DateTime<Utc>,
    pub created_by: Ulid,
}

pub enum SnapshotOrigin {
    ManualEdit,
    Import { adapter: AdapterId },
    Merge { resolved_conflicts: u32 },
    Migration { from: semver::Version },
    Rollback { to: Ulid },
}
```

Every Push, Pull, or Import creates an immutable snapshot, forming a DAG in the spirit of Git commits. Current canonical state is whatever `HEAD` points at.

`SnapshotOrigin` as an enum rather than a string is a small thing with real consequences: any future code that switches on provenance gets a compile error when a new origin is added, so a new sync path can't silently fall into a default branch and be mis-attributed in history.

### 11.2 Diff generation

```rust
pub struct FieldDiff {
    pub path: FieldPath,
    pub change: Change,
}

pub enum Change {
    Added   { after: serde_json::Value },
    Removed { before: serde_json::Value },
    Modified { before: serde_json::Value, after: serde_json::Value },
}

pub fn diff(a: &CharacterBuild, b: &CharacterBuild) -> Vec<FieldDiff>;
```

Diffing walks the schema **structurally**, not via a generic deep-object-diff run blind. Collections with identity (feats, inventory — anything with a stable id or slug) are diffed by identity-matched pairs, not array index. Without this, inserting one feat in the middle of a list reports the entire tail as changed, which is the single fastest way to make a merge UI useless.

### 11.3 Three-way merge

```rust
pub fn merge(
    ancestor: &CharacterBuild,
    ours: &CharacterBuild,
    theirs: &CharacterBuild,
) -> MergePreview;

pub struct MergePreview {
    pub auto_merged: Vec<FieldDiff>,
    pub conflicts: Vec<Conflict>,
}

pub struct Conflict {
    pub path: FieldPath,
    pub ancestor: serde_json::Value,
    pub ours: serde_json::Value,
    pub theirs: serde_json::Value,
}
```

Note the signature: `merge` is a **pure function of three `CharacterBuild`s**. No database, no `async`, no adapter. This is what makes the highest-risk code in the project exhaustively property-testable (§16) and is the concrete reason `sync-engine` is I/O-free.

Algorithm: find the common ancestor (via `parent` walk, or the last-synced snapshot recorded per connection, §13.3); compute `diff(ancestor, ours)` and `diff(ancestor, theirs)`; then per path — changed on one side only → take it; changed on both to the same value → no conflict; changed on both to different values → conflict. **Conflicts are never auto-resolved.** They're returned for a human to resolve per field before the merge snapshot is committed.

### 11.4 Rollback and history

Because states are immutable snapshots in a DAG, rollback records a *new* snapshot with `origin: Rollback { to }` carrying the old content — Git's `revert` semantics, not `reset --hard`. History is never destructively rewritten, which is the right default for a multi-user tool where one player's rollback shouldn't erase a GM's view of what happened. Full history is walkable as a DAG for a timeline UI.

---

## 12. Offline Support

- **The CLI is offline-first and is a single static binary.** `cargo build --target x86_64-unknown-linux-musl` produces one file with no runtime to install — a materially better self-hosting story than the Node equivalent, which needed a Node version, a package manager, and a `node_modules` tree.
- **Local canonical store is SQLite; hosted is PostgreSQL, from one codebase.** SQLx supports both, so the CLI and server share query code with the provider swapped.
- **Queued operations flush on reconnect.** The Foundry relay and the CLI queue outbound syncs locally when the API is unreachable.
- **Every sync operation is idempotent and content-addressed.** Operations are keyed by `content_hash` + parent snapshot, so replaying a queued op after a flaky reconnect is a no-op rather than a duplicate snapshot.
- **Conflict resolution already assumes divergence**, so "offline for two weeks with local edits" isn't a special case — it's an ordinary three-way merge with a bigger diff.
- **No feature requires the hosted service.** A GM can run an entire self-hosted campaign against a local SQLite store with the CLI and never contact any Nexus Character Sheet server. The hosted API is a convenience (multi-device, hosted history, sharing), never a dependency. For Foundry specifically, §7.4's world-shutdown-gated direct access means even the relay module is optional for offline workflows.

---

## 13. API Design

**REST over GraphQL.** Character sync is CRUD-plus-actions over a few well-known resource shapes, not a graph-shaped query problem that would benefit from client-driven field selection. REST also keeps the constrained integration clients simple — the Roll20 collector is ~150 lines of sandboxed JS and has no business carrying a GraphQL client. OpenAPI is generated from the Rust types via `utoipa`, sharing `schemars` output with §3.1, so spec, validation, and types cannot drift.

### 13.1 Authentication & authorization

- **Hosted:** OAuth 2.0 (authorization code + PKCE) for the web app; scoped, revocable **API keys** (`nsk_live_…`) for CLI, relay module, and collector, which cannot do interactive OAuth.
- **Self-hosted:** API keys only. OAuth optional for orgs wanting SSO, never required.
- **Model:** characters belong to an owner and may be shared with a **party** with per-member roles — `gm` (full write, plus read of partymates' session telemetry) and `player` (full write on own, read-only on partymates' `build`, no access to others' `session`). Enforced in one tower middleware layer shared by every route, not re-implemented per handler.

### 13.2 Character CRUD

```
GET    /v1/characters
POST   /v1/characters
GET    /v1/characters/:id
PATCH  /v1/characters/:id                              # manual edit -> new snapshot
DELETE /v1/characters/:id
GET    /v1/characters/:id/history                      # snapshot DAG
GET    /v1/characters/:id/session?platform=foundry     # read-only telemetry (§4)
```

**`PATCH /v1/characters/:id`**

```json
{
  "path": "build.skills.athletics.rank",
  "value": "expert",
  "expectedContentHash": "af5806f2…",
  "reason": "leveled up, chose Athletics"
}
```

```json
{
  "characterId": "01J8Z3F9K2N4Q6R8S0T2V4W6X8",
  "newSnapshotId": "01J8Z3FAK1M3P5Q7R9S1T3U5V7",
  "contentHash": "e3b0c44298fc1c14…",
  "diff": [
    { "path": "build.skills.athletics.rank", "change": "modified", "before": "trained", "after": "expert" }
  ]
}
```

`expectedContentHash` is the optimistic-locking token (§18); a mismatch returns `409` with current state rather than clobbering a concurrent edit.

### 13.3 Sync endpoints

```
POST   /v1/characters/:id/connections
GET    /v1/characters/:id/connections
DELETE /v1/characters/:id/connections/:connId

POST   /v1/characters/:id/connections/:connId/pull     # platform -> canonical
POST   /v1/characters/:id/connections/:connId/push     # canonical -> platform
POST   /v1/characters/:id/connections/:connId/diff     # dry run, no state change
GET    /v1/characters/:id/connections/:connId/drift    # detect_changes result
```

**`POST …/pull`** with no resolutions, when both sides diverged — `409`, nothing committed:

```json
{
  "status": "conflict",
  "mergePreview": {
    "autoMerged": [
      { "path": "build.feats", "change": "added", "after": { "slug": "toughness", "source": "Player Core" } }
    ],
    "conflicts": [
      {
        "path": "build.skills.athletics.rank",
        "ancestor": "trained",
        "ours": "expert",
        "theirs": "master"
      }
    ]
  }
}
```

Re-request with resolutions:

```json
{ "resolutions": [ { "path": "build.skills.athletics.rank", "resolution": "theirs" } ] }
```

```json
{
  "status": "merged",
  "newSnapshotId": "01J8Z3FBQ7X9Y1Z3A5B7C9D1E3",
  "contentHash": "9f86d081884c7d65…"
}
```

**`GET …/drift`** for a Roll20 connection whose sandbox is asleep (§6.2):

```json
{ "status": "unavailable", "reason": "sandboxNotRunning", "lastSeenAt": "2026-07-29T20:14:03Z" }
```

### 13.4 History & rollback

```
POST   /v1/characters/:id/rollback        { "toSnapshotId": "…" }
GET    /v1/characters/:id/diff?from=…&to=…
```

### 13.5 Webhooks

```
POST   /v1/webhooks        { "url": "…", "events": ["character.pushed", "character.driftDetected"] }
GET    /v1/webhooks
DELETE /v1/webhooks/:id
```

Payloads are signed (HMAC-SHA256 over the raw body, `X-Nexus-Signature`) so self-hosted consumers can verify authenticity without mTLS.

### 13.6 Plugin registration

```
POST   /v1/plugins/systems
POST   /v1/plugins/platforms
GET    /v1/plugins
```

`/v1/plugins/systems` is read-only introspection of compiled-in system plugins (§10.2). Adapter registration is the live surface: it accepts an adapter manifest (entry point plus declared capabilities), admin-gated on the hosted service, a local config file for self-hosters. The declared capability set is shown to the operator before install — an adapter asking for a suspiciously broad fetch allowlist should be visibly asking for it.

---

## 14. Technology Stack

| Choice | Why |
|---|---|
| **Rust (core)** | The correctness-critical surface here is a merge engine operating on data users cannot easily verify by eye — a subtly wrong merge silently corrupts a character someone spent months building. Exhaustive `match` on conflict cases, no null, and no accidental type coercion move a real class of bugs to compile time. Secondary benefits: one static binary for self-hosting, and low enough memory footprint to run the whole stack on a small VPS. |
| **Deliberately minimal JS** | Not a stack choice so much as an acknowledgment (§0.1): Roll20 and Foundry are JS runtimes. The strategy is to make every JS artifact a logic-free shim, and to use WASM where the platform allows (Foundry) so mapping code is shared rather than duplicated. |
| **Axum** | Tower-based middleware composes cleanly for the cross-cutting concerns this API actually has (auth, rate limiting, tracing), the API surface is small and flat, and it's the mainstream Tokio-ecosystem choice with the best library support. *Alternative:* Actix Web is comparably fast but its actor-influenced ergonomics buy nothing here; `poem` has nicer built-in OpenAPI but a much smaller ecosystem. |
| **SQLx** | Compile-time-checked queries against a real schema catch a class of errors ORMs defer to runtime, and one codebase targets both PostgreSQL (hosted) and SQLite (offline CLI, §12) — which is a hard requirement here, not a nicety. *Alternative:* SeaORM or Diesel give more abstraction; the query surface is small enough that an ORM's abstraction cost exceeds its benefit. |
| **PostgreSQL + SQLite** | Snapshot DAG plus JSONB `build` documents fits Postgres's mix of relational integrity and JSONB indexing (hash lookups now, "find characters with feat X" later). SQLite covers zero-ops self-hosting. |
| **serde / schemars / garde** | Together they replace what Zod did alone: serialization + structural validation, JSON Schema generation, semantic validation. Splitting them means structural validity is guaranteed by construction rather than re-checked at runtime (§3.1). |
| **utoipa** | Generates OpenAPI from the same annotated types, so docs, generated clients, and validation can't drift. *Alternative:* `aide` pairs well with Axum too; either is fine, but hand-written OpenAPI is not — it always rots. |
| **deno_core** | Embeds V8 with a custom op layer to run TS adapters in-process (§5.5), keeping the single-binary deployment while giving adapters no ambient capabilities. *Alternative:* `rquickjs` is far lighter but much slower and lacks V8's ecosystem compatibility; a Node sidecar is simpler but reintroduces a Node runtime dependency for every self-hoster. |
| **TypeScript + pnpm + Vitest + tsup** | The adapter workspace. Chosen because they're the defaults an adapter contributor already knows — the goal is that nothing about this project's tooling is novel to someone who has written a Foundry module. |
| **schemars → json-schema-to-typescript / json-schema-to-zod** | The one-way codegen pipeline (§3.6). Generated output is committed so TS contributors never need a Rust toolchain; CI fails on staleness. |
| **Cargo workspace + pnpm workspace, side by side** | Cargo handles the crate graph, pnpm the adapter packages. Two lockfiles and two dependency graphs is real overhead, accepted so each language keeps its native, familiar tooling. Turborepo is deliberately *not* used — the TS side is a handful of small leaf packages, and adding a task runner would be tooling for its own sake. |
| **wasm-bindgen** (narrow use) | Only to compile `sync-engine` for the browser-side diff preview (§19 Phase 3), so the UI shows exactly what the server would compute. Not used for adapters, which are TS natively. |
| **BLAKE3** | Content hashing on every write and drift check; several times faster than SHA-256 with no downside for non-adversarial integrity checking (§9.2). |
| **`cargo-deny` / `cargo-audit` + `pnpm audit`** | Supply-chain and license auditing on both sides. The TS side matters more here than in a typical project: adapters run in-process (sandboxed, but still), and an adapter's transitive npm dependencies are part of the trust surface. |
| **Docker (multi-stage, distroless)** | A Rust static binary makes final images tiny, which meaningfully improves the self-hoster experience — the persona this project explicitly targets. |
| **GitHub Actions + `Swatinem/rust-cache`** | Free for OSS and the default expectation for contributors. Rust's compile times make caching mandatory rather than optional — see §21. |

### 14.1 What the split costs

Stated plainly, because a stack section that only lists upsides is not useful:

- **Two toolchains in CI**, two lockfiles, two dependency-audit stories, two release processes. Every maintainer needs both installed even though most contributors need only one.
- **A codegen boundary that can rot.** Mitigated by committing generated output and failing CI on staleness (§3.6), but it's a moving part that an all-one-language design wouldn't have.
- **Debugging across the boundary is unpleasant.** A failure that starts in Axum, crosses into V8, and returns as a deserialization error has a worse stack trace than either language alone would produce. Mitigated by validating at the boundary with precise errors (§5.5) and by making adapters independently testable in pure TS (§16), so most adapter debugging never crosses the boundary at all.
- **`deno_core` is a substantial dependency** with its own upgrade cadence tied to V8. This is the single largest piece of infrastructure the split adds, and §21 treats it as a named risk with a fallback.
- **Rust's dynamic-data ceremony still applies** to the core, though much less than before — the messiest data (Foundry `system` blobs, Roll20 attribute soup) now lives in TS, which handles it far more gracefully. The split happens to put each language where its strengths are.

Accepted because the merge engine's correctness and the adapter contribution story are the two things this project lives or dies on, and no single language optimizes both.

---

## 15. Repository Structure

```
character-sheet/
├── Cargo.toml                    # Rust workspace
├── pnpm-workspace.yaml           # TS workspace
│
├── crates/                       # ─────────── RUST CORE ───────────
│   ├── schema-core/              # base canonical types — SOURCE OF TRUTH (§3.6)
│   ├── schema-pf2e/              # PF2e SystemPlugin
│   ├── schema-sf2e/              # SF2e SystemPlugin
│   ├── sync-engine/              # diff, merge, snapshot DAG — pure, no I/O, no adapter deps
│   ├── store/                    # SQLx queries + migrations, Postgres and SQLite
│   ├── adapter-host/             # deno_core runtime, ops, capability enforcement (§5.5)
│   └── api/                      # Axum: routes, auth, webhooks
│
├── packages/                     # ──────── TYPESCRIPT SHARED ───────
│   ├── schema-types/             # GENERATED — do not hand-edit
│   ├── schema-zod/               # GENERATED — do not hand-edit
│   ├── adapter-sdk/              # CharacterAdapter interface + contract test harness
│   └── foundry-mapping/          # shared by the Foundry adapter and relay module (§7.1)
│
├── adapters/                     # ───── TYPESCRIPT, CONTRIBUTED ────
│   ├── roll20/
│   │   ├── src/                  # mapping table, parser, validation
│   │   └── collector/            # nexus-collector.js — unbuilt, unminified, auditable
│   ├── foundry/
│   │   ├── src/
│   │   └── module/               # nexus-character-sheet + module.json
│   ├── pathbuilder/              # import-only
│   ├── wanderers-guide/          # import-only
│   └── demiplane/                # feature-flagged, best-effort
│
├── cli/                          # `nexus` — Rust, clap, single static binary
├── xtask/                        # cargo xtask codegen | openapi | fixtures
├── docs/                         # architecture (this file), ADRs, adapter-authoring guide
├── examples/
│   ├── fixtures/                 # sample characters per system, per schema version
│   └── example-adapter/          # template: clone, rename, implement, ship
└── .github/workflows/
```

**The layout encodes the contribution story.** A contributor adding an adapter works entirely inside `adapters/` and `packages/`, runs `pnpm test`, and never opens `crates/`. The README's contribution section should say exactly that, in the first paragraph.

**The one boundary that matters:** `sync-engine` depends on `schema-core` and nothing else — not `store`, not `api`, not `adapter-host`. Enforced by Cargo, so violating it is a compile error rather than a review comment. Adapters can't reach it at all; they're in another language behind an op interface.

**Generated packages are marked as such** — in their `package.json` description, in a header comment on every emitted file, and via a CI check. Committed generated code invites hand-edits, and that's the failure mode to design against (§3.6).

**`xtask`** holds codegen, OpenAPI export, and fixture regeneration as ordinary Rust programs (`cargo xtask codegen`), keeping build tooling in one place rather than split across npm scripts and Makefiles.

---

## 16. Testing Strategy

| Layer | Approach |
|---|---|
| **Unit tests** | `cargo test`, per crate, weighted heavily toward `sync-engine` — the highest-risk, most-reused code. |
| **Property tests** | `proptest` against the merge engine, which is where Rust pays off most. Because `merge` is pure (§11.3), we can assert real algebraic laws over generated characters: merging a build with itself is identity; `merge(a, x, x)` never conflicts; auto-merged fields never lose a change made on exactly one side; `hash(x) == hash(roundtrip(x))`. These catch the interleaving bugs that example-based tests structurally cannot. |
| **Adapter contract tests (TS)** | Shipped as `@nexus-ttrpg/adapter-sdk/testing` — a Vitest suite an adapter runs against itself in one line: `describe("roll20", () => runContractTests(adapter, fixtures))`. Asserts `import(export(fixture))` round-trips within `fieldCoverage`; `validate()` rejects known-bad input; `detectChanges` is a no-op on unchanged input; unknown content produces warnings rather than throwing (§5.2). This is the single most important artifact for contribution quality: a new adapter gets a meaningful correctness bar for free, in a test runner its author already knows, with no Rust in the loop. |
| **Adapter unit tests (TS)** | Plain Vitest, no host, no database, no network — adapters are stateless objects (§5.4), so `pnpm test` in an adapter directory is fast and self-contained. |
| **Host boundary tests (Rust)** | Assert the core rejects adapter output that violates the canonical schema, that capability grants are enforced (an adapter fetching a non-allowlisted host fails), and that a panicking or hanging adapter is contained rather than taking down the server. These test that the core distrusts adapters correctly (§1) — the security-relevant half of the boundary. |
| **Codegen staleness check (CI)** | `cargo xtask codegen && git diff --exit-code`. The whole cross-language schema guarantee (§3.6) rests on this one job. |
| **Integration tests** | `testcontainers` for Postgres; full route exercises including the conflict flow end to end (create → divergent platform edit → pull → resolve → verify DAG), running real adapters in the real host so the boundary is exercised, not mocked. |
| **Snapshot tests** | `insta` on the Rust side, Vitest snapshots on the TS side, for export payload shapes — catching accidental mapping regressions. |
| **Fixtures** | `examples/fixtures/` holds curated PF2e and SF2e characters across complexity tiers (fresh level 1, multiclassed level 10, companion-heavy, drone-heavy SF2e mechanic). Shared by both languages as plain JSON so there is exactly one ground-truth set, not one per language. |
| **Migration tests** | Every migration runs against the previous version's fixture, asserting the output deserializes into the current schema and preserves known values — a generated matrix, so adding a migration automatically tests it against every fixture. |
| **Canary tests** | A scheduled job imports a known test character from each live third-party platform, alerting maintainers when an upstream export format changes (§21). |

Dropped from the all-Rust plan: the WASM parity suite, which existed only to prove that the native and `wasm-bindgen` builds of the Foundry mapping crate agreed. With mapping in TS and shared by import, there is nothing to prove.

---

## 17. Security

- **API keys:** scoped (read-only / sync / admin), environment-prefixed, stored Argon2-hashed, never retrievable after creation.
- **OAuth:** hosted web login only; never required for machine-to-machine auth, which stays on API keys because neither the Roll20 sandbox nor a headless CLI can run an interactive flow.
- **Encrypted secrets:** connection credentials are encrypted at rest (AES-256-GCM, key from a KMS in hosted deployments, an operator-supplied key file self-hosted), so a leaked database dump doesn't yield plaintext platform credentials. Key material is held in `secrecy::Secret` wrappers so it isn't accidentally logged via `Debug` — a small ergonomic win that closes a common real-world leak.
- **Adapter trust (§5.5):** adapters are community code handling other people's characters and platform credentials, and they are sandboxed from day one. No filesystem access, no ambient `fetch`, no database handle, no environment. Network access is an allowlisted host op declared in the adapter manifest and reviewable in one glance at a JSON file. A Node-hosted equivalent would give every adapter ambient access to all of these by default; a compromised or careless adapter there could read server secrets or exfiltrate freely. Making the contribution surface *both* familiar and confined is the main security dividend of the split.
- **Review is defense in depth, not the only defense.** Hosted-service adapter registration is still reviewed, but because capability grants are enforced at runtime, a missed review isn't a full compromise. Self-hosters can inspect an adapter's declared capabilities before installing it.
- **System plugins are compiled in**, so trusting a system plugin equals trusting the repository — appropriate for the rare, high-stakes path (§10.2).
- **Self-hosting trust boundary:** self-hosters run their own database and API. The project's obligation is a well-documented deployment path and never assuming reachability of any hosted infrastructure.
- **The pasted-script problem:** the Roll20 collector asks users to paste code into a system with access to their campaign. It stays unminified, unbuilt, and short enough to read (§6.5), and its signed-payload protocol means a compromised Nexus Character Sheet server still can't make the collector do anything outside writing sheet attributes.
- **Rate limiting:** per-key token bucket in tower middleware (Redis-backed for multi-instance hosted, in-memory self-hosted), with more generous limits on read-heavy `drift`/`session` endpoints than on write-heavy `push`/`pull`.
- **Supply chain:** `cargo-deny` and `cargo-audit` in CI, with a pinned `Cargo.lock` committed for the binaries.

---

## 18. Performance & Scale

- **Caching:** derived-stat computation and serialized responses are cached keyed by `content_hash` — unchanged hash means skip recomputation entirely.
- **Incremental sync:** hash-gated diffing (§9.2) means routine "check drift across all my characters" short-circuits for the large majority of characters whose hash hasn't moved, without a structural diff.
- **Batching:** the drift job batches by connection, not character, using adapter-level batch calls where a platform supports them (one Foundry relay call returning change detection for every actor in a world, rather than one round trip per character). Roll20 is excluded from this job entirely, since its sandbox usually isn't running (§6.2).
- **Pagination:** cursor-based throughout, using ULID ordering as a natural stable cursor. Offset pagination degrades badly at the "thousands of characters" scale in the brief.
- **Optimistic locking:** writes carry `expectedContentHash`; mismatch returns `409` with current state rather than silently overwriting a concurrent edit — the same "never last-writer-wins" stance as the merge system (§11), applied to manual edits.
- **Concurrency:** Tokio's work-stealing scheduler handles the I/O-bound sync workload well, and the CPU-bound parts (diff, merge, hashing across thousands of characters) can move to `rayon` on a blocking pool without touching engine code, precisely because `sync-engine` is synchronous and pure (§1).

---

## 19. MVP Roadmap

**Phase 0 — Foundations (weeks 1–4)**
- `schema-core` + `schema-pf2e`, with fixtures.
- `sync-engine`: snapshot DAG, structural diff, three-way merge, with the proptest suite from day one — this is the code everything else trusts.
- `store` (SQLx, Postgres + SQLite); both workspaces and CI wired up, including the codegen staleness check (§3.6) before anything depends on generated types.

**Phase 1 — The adapter boundary (weeks 5–10)**
- `adapter-host`: deno_core runtime, ops, capability enforcement, boundary validation (§5.5).
- `adapter-sdk` including the contract test harness — built *alongside* the first adapter, not after it, so the harness is shaped by real use rather than guessed at.
- Foundry adapter, PF2e only: identity, ancestry, class, level, abilities, skills, feats, equipment, spellcasting. Serves as the reference implementation every later adapter is modeled on.
- CLI: `nexus import --foundry`, `nexus push --foundry`, `nexus diff`.

This phase is the project's real risk concentration: it's where the cross-language machinery either proves itself or reveals that all-TypeScript was the better call (§0.2). Worth an explicit checkpoint at the end of it rather than sailing past.

**Phase 2 — API + second adapter (weeks 11–16)**
- Axum API: CRUD, connections, push/pull/diff, webhooks (§13).
- Roll20 adapter: collector script, signed-payload handshake, chunked apply, sandbox-availability handling (§6.2).
- Foundry relay module, sharing `foundry-mapping` with the adapter (§7.1).
- SF2e system plugin + Foundry SF2e coverage (drones, Resolve Points, credits).

**Phase 3 — Conflict UX + hardening (weeks 17–21)**
- Minimal web UI for history and conflict resolution, with `sync-engine` compiled to WASM driving the diff preview so the UI shows exactly what the server would compute.
- Exercise the migration framework for real by shipping one intentional v1→v2 change.
- Security pass (§17), rate limiting, Docker Compose, self-hosting docs.
- **Adapter-authoring guide and `example-adapter` template.** Listed as a deliverable, not documentation cleanup: the entire architecture is arranged around outside adapter contributions, and shipping without a clear on-ramp wastes that.

**Done criteria:** a player can build a PF2e character in Foundry, push to canonical, pull into a second Foundry world (or export to Roll20), make divergent edits in both, and resolve a real merge conflict end to end — with SF2e at parity for the fields SF2e uses. Plus: someone outside the core team can write a working adapter from the guide alone.

Explicitly **not** in the MVP: any third-party builder adapter (those are the first community contributions, deliberately left on the table), TS system plugins (§20), and the hosted multi-tenant service.

---

## 20. Stretch Goals

- Pathbuilder / Wanderer's Guide import adapters — low effort once the pattern is proven, good candidates for the first community contributions.
- **TS system plugins** (§10.2): expose `derivedStats` and the content index — the rules-math half — as a TypeScript interface running in the adapter host, so rules contributors can work without a Rust PR while the schema half stays Rust. The natural next step once adapters have proven the host.
- Full web character viewer/editor against canonical directly, reducing reliance on any one VTT just to *view* a character.
- A D&D 5e system plugin as the first non-PF-family system, validating the plugin architecture's generality beyond "PF2e and its close cousin."
- Party-level resources: shared loot, downtime tracking, SF2e starships as entities.
- A hosted offering for users who don't want to self-host — strictly optional, never a dependency of the offline path (§12).

---

## 21. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| **The two-language split costs more than it returns.** The Rust core is now roughly a merge engine, a schema, storage, and a thin API. Against that sit a codegen pipeline, an embedded JS runtime, two toolchains, and cross-boundary debugging. It is entirely possible that all-TypeScript would be simpler overall for comparable real-world correctness. | Treat the end of Phase 1 as an explicit go/no-go (§19): by then the host, the codegen, and one real adapter all exist, and the maintenance cost is measurable rather than speculative. If the boundary is consuming more attention than the engine's guarantees are saving, collapsing to TypeScript is still cheap at that point — the canonical model, the adapter interface, and the whole conceptual architecture survive the move, because none of them are language-specific. Deferring the decision past Phase 1 is what makes it expensive. |
| **`deno_core` is heavy infrastructure with a V8-tied upgrade cadence**, and it sits on the critical path for every sync operation. | Keep the op surface deliberately tiny (fetch, log, nothing else), so the integration is shallow and replaceable. Fallback if it becomes untenable: the `"runtime": "external"` subprocess protocol (§5.5) already exists as an escape hatch and can be promoted to the primary mechanism, trading in-process performance and the single-binary property for a Node dependency. This is a real fallback, not a theoretical one — the interface is identical either way. |
| **Generated schema packages get hand-edited**, silently breaking the single-source-of-truth guarantee. | Header comments on every generated file, `package.json` descriptions marking them generated, and a CI job that regenerates and fails on diff (§3.6). The CI job is the actual control; the rest is signage. |
| **Contributors need both toolchains to run the full test suite**, which undercuts the "no Rust required" promise. | Adapter tests must run standalone with `pnpm test`, with no Rust and no host — an explicit requirement on the SDK design (§16), not an aspiration. CI runs the full cross-boundary suite; contributors run their adapter's suite. If a contributor ever needs `cargo` to iterate on an adapter, that's a bug in the SDK. |
| **Roll20's sandbox is unreachable and frequently asleep, making that adapter feel second-class.** | Document the asymmetry prominently (§6.2) rather than overselling parity. Roadmap Foundry first, where the constraint doesn't apply. Return honest `Unavailable` responses instead of stale data. Never build a fragile scraping workaround to fake parity. |
| **Direct LevelDB access corrupts a user's world.** | Never write raw LevelDB. Gate direct access on a verified world shutdown, route through Foundry's own pack/unpack CLI, and back up first (§7.4). The one place the design defers to platform tooling over a native Rust implementation. |
| **The canonical schema becomes a bottleneck** — every rules update needing a schema PR before adapters can use it. | The `extensions` escape hatch (§3.5, §9.4) means adapters are never blocked; promotion into the core schema happens later, informed by real usage. |
| **Adapter authors couple engine logic to their platform.** | Enforced by the Cargo dependency graph — it doesn't compile (§1, §15). |
| **Merge conflicts overwhelm users with noise.** | Identity-aware diffing (§11.2) removes spurious array-reordering conflicts; `session` is structurally excluded from diffing (§4), removing the largest source of would-be noise. Property tests assert that identical inputs never produce conflicts. |
| **Third-party builders change export formats without notice.** | Contract tests against pinned fixtures, plus a scheduled canary job importing a known character from each live platform, alerting maintainers before users report breakage (§16). |
| **Self-hosters skip upgrades and hit a painful multi-version jump.** | Chained, lazily-applied migrations (§9.3) — a v1 character read by a v5 binary migrates through automatically, with no manual stepping. |
| **Homebrew makes validation either too strict or too loose.** | The error/warning type split (§5.2): structurally invalid is an `Err`, unrecognized-but-well-formed is a warning with the data preserved. Homebrew defaults to the permissive path. |
| **A malicious or careless adapter reaches something it shouldn't.** Adapters are community code running beside other people's characters and platform credentials. | Capabilities are granted, never ambient (§5.5): no filesystem, no environment, no database handle, and network only via an allowlisted host op. Boundary tests assert enforcement rather than assuming it (§16). Adapter panics and hangs are contained by the host with timeouts, so one bad adapter degrades one connection rather than the server. |
| **Scope creep across too many platforms at launch.** | Sequence Foundry → Roll20 → community-contributed builders (§19–20), proving the trait is sufficient against two structurally very different platforms before asking anyone to build more against it. |

---

*A living reference. Expect ADRs in `docs/adr/` to supersede specific choices here as the project evolves — the same versioning philosophy §9.3 applies to data, applied to the design itself.*
