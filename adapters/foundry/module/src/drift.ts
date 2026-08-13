// Drift state tracker for the relay module (§5.3, §7.1).
//
// The relay module hooks Actor create/update to maintain drift telemetry — a
// record of which actors have changed since their last sync. This is NOT
// auto-sync (§4): the server still initiates the pull/push. The drift state
// just lets the server ask "which actors changed?" without polling every one.
//
// The tracker is pure logic: a Map<actorId, DriftEntry> with mark/resolve
// operations. It's tested directly — no Foundry APIs needed.
//
// §7.1 point 2: "Hooks Actor create/update to maintain drift telemetry (§5.3)
// — still never auto-syncing (§4), but drift checks don't need to poll."

/**
 * Drift state for a single actor.
 */
export interface DriftEntry {
  /** The actor's UUID (stable across renames). */
  actorUuid: string;
  /** The actor's name at the time of the last change (for human-readable reports). */
  actorName: string;
  /** When the change was detected (ISO 8601). */
  changedAt: string;
  /** Content hash of the actor at the time of the last change (for cheap comparison). */
  contentHash: string;
  /** Whether this actor has been synced since the last change. */
  synced: boolean;
}

/**
 * A drift report returned by the drift endpoint.
 */
export interface DriftReport {
  /** Actors that have changed since their last sync. */
  dirty: DriftEntry[];
  /** Actors that are up-to-date (included for completeness; the server can skip these). */
  clean: string[];
  /** Total actors tracked. */
  total: number;
}

/**
 * The drift state tracker — pure logic, no Foundry APIs.
 *
 * The relay module creates one of these on init, feeds it Actor create/update
 * events, and queries it when the server calls the drift endpoint.
 */
export class DriftTracker {
  private entries = new Map<string, DriftEntry>();

  /**
   * Mark an actor as changed (dirty). Called from the createActor/updateActor
   * hook. Computes a content hash for cheap drift comparison.
   */
  markChanged(actorId: string, actorUuid: string, actorName: string, contentHash: string): void {
    this.entries.set(actorId, {
      actorUuid,
      actorName,
      changedAt: new Date().toISOString(),
      contentHash,
      synced: false,
    });
  }

  /**
   * Mark an actor as synced. Called after a successful pull/push.
   */
  markSynced(actorId: string): void {
    const entry = this.entries.get(actorId);
    if (entry) {
      entry.synced = true;
    }
  }

  /**
   * Generate a drift report for the server.
   */
  report(): DriftReport {
    const dirty: DriftEntry[] = [];
    const clean: string[] = [];

    for (const [id, entry] of this.entries) {
      if (entry.synced) {
        clean.push(id);
      } else {
        dirty.push(entry);
      }
    }

    return {
      dirty,
      clean,
      total: this.entries.size,
    };
  }

  /**
   * Get a specific actor's drift entry (for the server to check a single actor).
   */
  get(actorId: string): DriftEntry | undefined {
    return this.entries.get(actorId);
  }

  /**
   * Check if an actor has changed since its last sync.
   */
  isDirty(actorId: string): boolean {
    const entry = this.entries.get(actorId);
    return entry !== undefined && !entry.synced;
  }

  /**
   * Clear all drift state (used on world reset / module reload).
   */
  clear(): void {
    this.entries.clear();
  }
}

/**
 * Compute a simple content hash for an actor's serialized form.
 *
 * This is NOT the canonical BLAKE3 content hash (§9.2) — that's computed
 * server-side from the canonical build. This is a cheap hash of the raw
 * Foundry Actor JSON for the drift tracker to decide "did anything change?"
 * without a full import cycle. A collision here just means a missed drift
 * signal (the server's pull would still catch it); it's not a correctness
 * issue.
 */
export function actorContentHash(actorJson: string): string {
  // FNV-1a — fast, dependency-free, good enough for drift detection.
  let hash = 0x811c9dc5;
  for (let i = 0; i < actorJson.length; i++) {
    hash ^= actorJson.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}
