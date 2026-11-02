// API key management for the relay module (§7.1, §6.5).
//
// The relay module generates a per-world API key in module settings. The Nexus
// Sheet server (or CLI) includes this key as `Authorization: Bearer <key>` when
// calling the relay's HTTP endpoints. This is the relay-side authentication —
// distinct from the server-side `nsk_live_…` API key the relay uses to call
// the server (§13.1).
//
// The key is generated once on first module init and stored in world-scoped
// settings. It's never logged or transmitted except in the Authorization header
// over localhost.

import type { FoundrySettings } from "./types.js";

const MODULE_ID = "nexus-character-sheet";
const SETTING_KEY = "apiKey";

/**
 * A generated relay API key (opaque string, not an `nsk_live_…` server key).
 * Format: `nsk_relay_<32 hex chars>` — visually distinct from server keys.
 */
export function generateApiKey(): string {
  const bytes = new Uint8Array(16);
  // crypto.getRandomValues is available in Foundry's browser context.
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `nsk_relay_${hex}`;
}

/**
 * Register the API key setting on module init (§7.1).
 *
 * If no key exists yet, generates one. The key is world-scoped so all GMs in
 * the same world share it (the server uses one key per connection).
 */
export function registerApiKeySetting(settings: FoundrySettings): void {
  settings.register(MODULE_ID, SETTING_KEY, {
    scope: "world",
    config: false,
    type: String,
    default: "",
  });
}

/**
 * Get the relay API key, generating one if none exists yet.
 */
export async function getOrGenerateApiKey(settings: FoundrySettings): Promise<string> {
  const existing = settings.get(MODULE_ID, SETTING_KEY) as string;
  if (existing) return existing;

  const generated = generateApiKey();
  await settings.set(MODULE_ID, SETTING_KEY, generated);
  return generated;
}

/**
 * Validate an Authorization header against the stored API key.
 *
 * Returns true if the header matches `Bearer <key>`. Constant-time comparison
 * to prevent timing attacks (§17 — the relay runs on localhost, but the key
 * is still a credential).
 */
export function validateAuthHeader(authHeader: string | undefined, expectedKey: string): boolean {
  if (!authHeader || !authHeader.startsWith("Bearer ")) return false;
  const provided = authHeader.slice(7);
  return constantTimeEqual(provided, expectedKey);
}

/**
 * Constant-time string comparison to prevent timing side-channels.
 */
function constantTimeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

export { MODULE_ID, SETTING_KEY };
