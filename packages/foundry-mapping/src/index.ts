// @nexus-sheet/foundry-mapping — Actor↔canonical mapping, written once (§7.1).
//
// Shared by plain import between adapters/foundry (server-side) and the
// foundry relay module (in-browser). The clearest concrete win from the
// language split: no WASM toolchain, no dual-target build, no parity suite.
//
// Phase 1 scope (§19): identity, ancestry, heritage, background, class, level,
// ability scores, skills, proficiencies, feats, classFeatures, equipment,
// spellcasting, currencies, actions, reactions, explorationActivities, notes.
// SF2e coverage (§3.4, §2.4): drones (chassis + mods), vehicles (role), credits/
// UPB (in currencies). Max HP only on export — current HP is session state and
// is never written (§4, §7.2). Resolve Points are also session state (§3.4, §4)
// and are never written — they live in `session`, not `build`.

import type {
  CharacterBuild,
  AbilityScoreKey,
  AbilityBoost,
  AbilityBoostSource,
  ActionType,
  Character,
} from "@nexus-sheet/schema-types";

// ── Foundry platform types ───────────────────────────────────────────────────
//
// Modeled on the real Foundry PF2e system data model, simplified to the
// fields the Phase 1 mapping covers. An adapter author working on a real
// Foundry integration would refine these against the live system's DataModel.

export type FoundrySystemId = "pf2e" | "sf2e";

/** A Foundry Actor document (PF2e character). */
export interface FoundryActor {
  name: string;
  type: "character";
  system: {
    abilities: Partial<Record<AbilityScoreKey, FoundryAbility>>;
    attributes: {
      hp: { max: number };
    };
    details: {
      level: { value: number };
      ancestry: FoundryDetailRef;
      heritage: FoundryDetailRef;
      background: FoundryDetailRef;
      class: FoundryDetailRef & { keyAbility: AbilityScoreKey; hpPerLevel: number };
    };
    skills: Record<string, FoundrySkill>;
    proficiencies: Record<string, string>;
    currency: FoundryCurrency;
  };
  items: FoundryItem[];
  flags?: Record<string, Record<string, unknown>>;
}

interface FoundryAbility {
  value: number;
  boosts: Array<{ source: AbilityBoostSource }>;
  flaws: Array<{ source: AbilityBoostSource }>;
  itemBonus: number;
}

interface FoundryDetailRef {
  value: string;
  uuid?: string | null;
  source?: string | null;
}

interface FoundrySkill {
  rank: string;
  itemBonus: number;
  lore?: string | null;
}

interface FoundryCurrency {
  gp?: number;
  sp?: number;
  cp?: number;
  pp?: number;
  credits?: number;
  upb?: number;
}

export type FoundryItemType =
  | "feat"
  | "weapon"
  | "armor"
  | "consumable"
  | "spellcastingEntry"
  | "spell"
  | "action"
  | "reaction"
  | "explorationActivity"
  | "classFeature"
  | "note"
  | "drone"
  | "vehicle";

export interface FoundryItem {
  type: FoundryItemType;
  name: string;
  system: {
    slug?: string;
    source?: string;
    level?: { value: number };
    category?: string;
    quantity?: number;
    tradition?: string;
    prepared?: string;
    attribute?: string;
    slots?: Record<string, number>;
    body?: string;
    title?: string;
    actionType?: string;
    // SF2e drone (§3.4): chassis type + installed mods.
    chassis?: string | null;
    mods?: FoundryDroneMod[];
    // SF2e vehicle (§3.4): character's role aboard the vehicle.
    role?: string | null;
  };
  uuid?: string | null;
  flags?: {
    "nexus-sheet"?: {
      homebrew?: boolean;
      inlinedFallback?: boolean;
    };
  };
}

/** A drone mod slot on a Foundry drone item (§3.4). Mirrors `SystemRefId`. */
export interface FoundryDroneMod {
  slug: string;
  source?: string | null;
  foundryUuid?: string | null;
}

// ── Canonical types (re-exported for adapter convenience) ────────────────────

export type {
  CharacterBuild,
  AbilityScoreKey,
  AbilityBoost,
  AbilityBoostSource,
  ActionType,
  Character,
} from "@nexus-sheet/schema-types";

// ── Helpers ──────────────────────────────────────────────────────────────────

const ABILITY_KEYS: readonly AbilityScoreKey[] = ["str", "dex", "con", "int", "wis", "cha"];

/**
 * Derive the final ability score value from its components (§3.3).
 *
 * PF2e: start at 10, each boost is +2, each flaw is -2, plus item bonus.
 * (The real PF2e rule has diminishing returns after the first boost, but
 * the canonical model stores components — the derived value is for the
 * platform payload only and doesn't need to match the system's exact
 * derivation rules for the round-trip to work.)
 */
export function derivedAbilityValue(entry: {
  boosts: AbilityBoost[];
  flaws: AbilityBoost[];
  itemBonus: number;
}): number {
  return 10 + 2 * entry.boosts.length - 2 * entry.flaws.length + entry.itemBonus;
}

/**
 * Derive the ability modifier from the score value (PF2e standard).
 */
export function abilityMod(score: number): number {
  return Math.floor((score - 10) / 2);
}

/**
 * Derive max HP from the build (§7.2).
 *
 * PF2e: class.hpPerLevel * level + con_modifier * level.
 * (Ancestry HP is not in the canonical model — the derived value is
 * approximate, and that's fine: current HP is session state and never
 * written; max HP on export is a convenience, not a round-trip field.)
 */
export function derivedMaxHp(build: CharacterBuild): number {
  const conEntry = build.abilityScores.con;
  const conScore = conEntry ? derivedAbilityValue(conEntry) : 10;
  const conMod = abilityMod(conScore);
  return build.class.hpPerLevel * build.level + conMod * build.level;
}

function refToDetail(
  ref: { reference: { slug: string; source?: string | null; foundryUuid?: string | null } },
  displayName?: string | null,
): FoundryDetailRef {
  const detail: FoundryDetailRef = {
    value: displayName ?? ref.reference.slug,
  };
  if (ref.reference.foundryUuid) {
    detail.uuid = ref.reference.foundryUuid;
  }
  if (ref.reference.source) {
    detail.source = ref.reference.source;
  }
  return detail;
}

function detailToRef(detail: FoundryDetailRef): {
  reference: { slug: string; source?: string | null; foundryUuid?: string | null };
  displayName?: string | null;
} {
  const ref: {
    reference: { slug: string; source?: string | null; foundryUuid?: string | null };
    displayName?: string | null;
  } = {
    reference: { slug: detail.value },
  };
  if (detail.uuid) {
    ref.reference.foundryUuid = detail.uuid;
  }
  if (detail.source) {
    ref.reference.source = detail.source;
  }
  return ref;
}

// ── Export: canonical → Foundry Actor (§7.2) ─────────────────────────────────

function mapAbilityScores(
  abilities: CharacterBuild["abilityScores"],
): Partial<Record<AbilityScoreKey, FoundryAbility>> {
  const result: Partial<Record<AbilityScoreKey, FoundryAbility>> = {};
  for (const key of ABILITY_KEYS) {
    const entry = abilities[key];
    if (entry) {
      result[key] = {
        value: derivedAbilityValue(entry),
        boosts: entry.boosts.map((b) => ({ source: b.source })),
        flaws: entry.flaws.map((f) => ({ source: f.source })),
        itemBonus: entry.itemBonus,
      };
    }
  }
  return result;
}

function mapSkills(skills: CharacterBuild["skills"]): Record<string, FoundrySkill> {
  const result: Record<string, FoundrySkill> = {};
  for (const [key, entry] of Object.entries(skills)) {
    const skill: FoundrySkill = {
      rank: entry.rank,
      itemBonus: entry.itemBonus,
    };
    if (entry.lore) {
      skill.lore = entry.lore;
    }
    result[key] = skill;
  }
  return result;
}

function mapProficiencies(profs: CharacterBuild["proficiencies"]): Record<string, string> {
  const result: Record<string, string> = {};
  for (const [key, rank] of Object.entries(profs)) {
    result[key] = rank;
  }
  return result;
}

function mapCurrencies(currencies: CharacterBuild["currencies"]): FoundryCurrency {
  const result: FoundryCurrency = {};
  if (currencies.gp != null) result.gp = currencies.gp;
  if (currencies.sp != null) result.sp = currencies.sp;
  if (currencies.cp != null) result.cp = currencies.cp;
  if (currencies.pp != null) result.pp = currencies.pp;
  if (currencies.credits != null) result.credits = currencies.credits;
  if (currencies.upb != null) result.upb = currencies.upb;
  return result;
}

function featToItem(feat: CharacterBuild["feats"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "feat",
    name: feat.reference.slug,
    system: {
      slug: feat.reference.slug,
      level: { value: feat.level },
      category: feat.category,
    },
  };
  if (feat.reference.source) {
    item.system.source = feat.reference.source;
  }
  if (feat.reference.foundryUuid) {
    item.uuid = feat.reference.foundryUuid;
  }
  return item;
}

function classFeatureToItem(cf: CharacterBuild["classFeatures"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "classFeature",
    name: cf.reference.slug,
    system: {
      slug: cf.reference.slug,
      level: { value: cf.level },
    },
  };
  if (cf.reference.source) {
    item.system.source = cf.reference.source;
  }
  if (cf.reference.foundryUuid) {
    item.uuid = cf.reference.foundryUuid;
  }
  return item;
}

function weaponToItem(weapon: CharacterBuild["equipment"]["weapons"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "weapon",
    name: weapon.reference.slug,
    system: {
      slug: weapon.reference.slug,
      quantity: weapon.quantity,
    },
  };
  if (weapon.reference.source) {
    item.system.source = weapon.reference.source;
  }
  if (weapon.reference.foundryUuid) {
    item.uuid = weapon.reference.foundryUuid;
  }
  return item;
}

function armorToItem(armor: CharacterBuild["equipment"]["armor"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "armor",
    name: armor.reference.slug,
    system: {
      slug: armor.reference.slug,
      quantity: armor.quantity,
    },
  };
  if (armor.reference.source) {
    item.system.source = armor.reference.source;
  }
  if (armor.reference.foundryUuid) {
    item.uuid = armor.reference.foundryUuid;
  }
  return item;
}

function inventoryToItem(item: CharacterBuild["equipment"]["inventory"][number]): FoundryItem {
  const result: FoundryItem = {
    type: "consumable",
    name: item.reference.slug,
    system: {
      slug: item.reference.slug,
      quantity: item.quantity,
    },
  };
  if (item.reference.source) {
    result.system.source = item.reference.source;
  }
  if (item.reference.foundryUuid) {
    result.uuid = item.reference.foundryUuid;
  }
  return result;
}

function spellcastingToItems(
  entry: CharacterBuild["spellcasting"]["entries"][number],
): FoundryItem[] {
  const items: FoundryItem[] = [];

  const spellcastingItem: FoundryItem = {
    type: "spellcastingEntry",
    name: `${entry.tradition}-spellcasting`,
    system: {
      tradition: entry.tradition,
      prepared: entry.preparedness,
      attribute: entry.attribute,
      slots: { ...entry.slots },
    },
  };
  items.push(spellcastingItem);

  for (const spell of entry.spells) {
    const spellItem: FoundryItem = {
      type: "spell",
      name: spell.slug,
      system: {
        slug: spell.slug,
      },
    };
    if (spell.source) {
      spellItem.system.source = spell.source;
    }
    if (spell.foundryUuid) {
      spellItem.uuid = spell.foundryUuid;
    }
    items.push(spellItem);
  }

  return items;
}

function actionToItem(action: CharacterBuild["actions"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "action",
    name: action.reference.slug,
    system: {
      slug: action.reference.slug,
      actionType: action.actionType,
    },
  };
  if (action.reference.source) {
    item.system.source = action.reference.source;
  }
  if (action.reference.foundryUuid) {
    item.uuid = action.reference.foundryUuid;
  }
  return item;
}

function reactionToItem(reaction: CharacterBuild["reactions"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "reaction",
    name: reaction.reference.slug,
    system: {
      slug: reaction.reference.slug,
    },
  };
  if (reaction.reference.source) {
    item.system.source = reaction.reference.source;
  }
  if (reaction.reference.foundryUuid) {
    item.uuid = reaction.reference.foundryUuid;
  }
  return item;
}

function explorationToItem(
  activity: CharacterBuild["explorationActivities"][number],
): FoundryItem {
  const item: FoundryItem = {
    type: "explorationActivity",
    name: activity.reference.slug,
    system: {
      slug: activity.reference.slug,
    },
  };
  if (activity.reference.source) {
    item.system.source = activity.reference.source;
  }
  if (activity.reference.foundryUuid) {
    item.uuid = activity.reference.foundryUuid;
  }
  return item;
}

function noteToItem(note: CharacterBuild["notes"][number]): FoundryItem {
  return {
    type: "note",
    name: note.title,
    system: {
      title: note.title,
      body: note.body,
    },
  };
}

function droneToItem(drone: CharacterBuild["drones"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "drone",
    name: drone.reference.slug,
    system: {
      slug: drone.reference.slug,
      mods: drone.mods.map((m) => {
        const mod: FoundryDroneMod = { slug: m.slug };
        if (m.source) mod.source = m.source;
        if (m.foundryUuid) mod.foundryUuid = m.foundryUuid;
        return mod;
      }),
    },
  };
  if (drone.chassis != null) {
    item.system.chassis = drone.chassis;
  }
  if (drone.reference.source) {
    item.system.source = drone.reference.source;
  }
  if (drone.reference.foundryUuid) {
    item.uuid = drone.reference.foundryUuid;
  }
  return item;
}

function vehicleToItem(vehicle: CharacterBuild["vehicles"][number]): FoundryItem {
  const item: FoundryItem = {
    type: "vehicle",
    name: vehicle.reference.slug,
    system: {
      slug: vehicle.reference.slug,
    },
  };
  if (vehicle.role != null) {
    item.system.role = vehicle.role;
  }
  if (vehicle.reference.source) {
    item.system.source = vehicle.reference.source;
  }
  if (vehicle.reference.foundryUuid) {
    item.uuid = vehicle.reference.foundryUuid;
  }
  return item;
}

/**
 * Export a canonical `CharacterBuild` to a Foundry Actor (§7.2).
 *
 * Max HP only — current HP is session state and is never written (§4, §7.2).
 * Items are grouped by type: feats, weapons, armor, inventory, spells,
 * spellcasting entries, actions, reactions, exploration activities, notes.
 */
export function exportActor(build: CharacterBuild, _system: FoundrySystemId): FoundryActor {
  return {
    name: build.identity.name,
    type: "character",
    system: {
      abilities: mapAbilityScores(build.abilityScores),
      attributes: {
        hp: { max: derivedMaxHp(build) },
      },
      details: {
        level: { value: build.level },
        ancestry: refToDetail(build.ancestry, build.ancestry.displayName),
        heritage: refToDetail(build.heritage, build.heritage.displayName),
        background: refToDetail(build.background, build.background.displayName),
        class: {
          ...refToDetail(build.class, build.class.reference.slug),
          keyAbility: build.class.keyAbility,
          hpPerLevel: build.class.hpPerLevel,
        },
      },
      skills: mapSkills(build.skills),
      proficiencies: mapProficiencies(build.proficiencies),
      currency: mapCurrencies(build.currencies),
    },
    items: [
      ...build.feats.map(featToItem),
      ...build.classFeatures.map(classFeatureToItem),
      ...build.equipment.weapons.map(weaponToItem),
      ...build.equipment.armor.map(armorToItem),
      ...build.equipment.inventory.map(inventoryToItem),
      ...build.spellcasting.entries.flatMap(spellcastingToItems),
      ...build.actions.map(actionToItem),
      ...build.reactions.map(reactionToItem),
      ...build.explorationActivities.map(explorationToItem),
      ...build.notes.map(noteToItem),
      ...build.drones.map(droneToItem),
      ...build.vehicles.map(vehicleToItem),
    ],
  };
}

// ── Import: Foundry Actor → canonical PartialBuild (§7.2) ────────────────────

function importAbilities(
  foundryAbilities: Partial<Record<AbilityScoreKey, FoundryAbility>>,
): CharacterBuild["abilityScores"] {
  const result: CharacterBuild["abilityScores"] = {};
  for (const key of ABILITY_KEYS) {
    const fa = foundryAbilities[key];
    if (fa) {
      result[key] = {
        boosts: fa.boosts.map((b) => ({ source: b.source })),
        flaws: fa.flaws.map((b) => ({ source: b.source })),
        itemBonus: fa.itemBonus,
      };
    }
  }
  return result;
}

function importSkills(skills: Record<string, FoundrySkill>): CharacterBuild["skills"] {
  const result: CharacterBuild["skills"] = {};
  for (const [key, entry] of Object.entries(skills)) {
    const skill: CharacterBuild["skills"][string] = {
      rank: entry.rank as CharacterBuild["skills"][string]["rank"],
      itemBonus: entry.itemBonus,
    };
    if (entry.lore) {
      skill.lore = entry.lore;
    }
    result[key] = skill;
  }
  return result;
}

function importProficiencies(
  profs: Record<string, string>,
): CharacterBuild["proficiencies"] {
  const result: CharacterBuild["proficiencies"] = {};
  for (const [key, rank] of Object.entries(profs)) {
    result[key] = rank as CharacterBuild["proficiencies"][string];
  }
  return result;
}

function importCurrencies(currency: FoundryCurrency): CharacterBuild["currencies"] {
  return {
    gp: currency.gp ?? null,
    sp: currency.sp ?? null,
    cp: currency.cp ?? null,
    pp: currency.pp ?? null,
    credits: currency.credits ?? null,
    upb: currency.upb ?? null,
  };
}

function importItems(
  items: FoundryItem[],
): Pick<
  CharacterBuild,
  "feats" | "classFeatures" | "equipment" | "spellcasting" | "actions" | "reactions" | "explorationActivities" | "notes" | "drones" | "vehicles"
> {
  const feats: CharacterBuild["feats"] = [];
  const classFeatures: CharacterBuild["classFeatures"] = [];
  const weapons: CharacterBuild["equipment"]["weapons"] = [];
  const armor: CharacterBuild["equipment"]["armor"] = [];
  const inventory: CharacterBuild["equipment"]["inventory"] = [];
  const spellcastingEntries: CharacterBuild["spellcasting"]["entries"] = [];
  const actions: CharacterBuild["actions"] = [];
  const reactions: CharacterBuild["reactions"] = [];
  const explorationActivities: CharacterBuild["explorationActivities"] = [];
  const notes: CharacterBuild["notes"] = [];
  const drones: CharacterBuild["drones"] = [];
  const vehicles: CharacterBuild["vehicles"] = [];

  for (const item of items) {
    switch (item.type) {
      case "feat": {
        const ref: CharacterBuild["feats"][number] = {
          reference: itemToRef(item),
          category: (item.system.category ?? "general") as CharacterBuild["feats"][number]["category"],
          level: item.system.level?.value ?? 1,
        };
        feats.push(ref);
        break;
      }
      case "classFeature": {
        classFeatures.push({
          reference: itemToRef(item),
          level: item.system.level?.value ?? 1,
        });
        break;
      }
      case "weapon": {
        weapons.push({
          reference: itemToRef(item),
          quantity: item.system.quantity ?? 1,
        });
        break;
      }
      case "armor": {
        armor.push({
          reference: itemToRef(item),
          quantity: item.system.quantity ?? 1,
        });
        break;
      }
      case "consumable": {
        inventory.push({
          reference: itemToRef(item),
          quantity: item.system.quantity ?? 1,
        });
        break;
      }
      case "spellcastingEntry": {
        spellcastingEntries.push({
          tradition: (item.system.tradition ?? "arcane") as CharacterBuild["spellcasting"]["entries"][number]["tradition"],
          preparedness: (item.system.prepared ?? "prepared") as CharacterBuild["spellcasting"]["entries"][number]["preparedness"],
          attribute: (item.system.attribute ?? "int") as AbilityScoreKey,
          slots: { ...(item.system.slots ?? {}) },
          spells: [],
        });
        break;
      }
      case "spell": {
        // Spells follow their spellcasting entry in the items array —
        // attach to the most recent entry. If none exists, create a
        // default innate entry (§7.7 homebrew spell without a parent entry).
        if (spellcastingEntries.length > 0) {
          spellcastingEntries[spellcastingEntries.length - 1]!.spells.push(itemToRef(item));
        } else {
          spellcastingEntries.push({
            tradition: "arcane",
            preparedness: "innate",
            attribute: "cha",
            slots: {},
            spells: [itemToRef(item)],
          });
        }
        break;
      }
      case "action": {
        actions.push({
          reference: itemToRef(item),
          actionType: (item.system.actionType ?? "single") as ActionType,
        });
        break;
      }
      case "reaction": {
        reactions.push({ reference: itemToRef(item) });
        break;
      }
      case "explorationActivity": {
        explorationActivities.push({ reference: itemToRef(item) });
        break;
      }
      case "note": {
        notes.push({
          title: item.system.title ?? item.name,
          body: item.system.body ?? "",
        });
        break;
      }
      case "drone": {
        const drone: CharacterBuild["drones"][number] = {
          reference: itemToRef(item),
          mods: (item.system.mods ?? []).map((m) => {
            const ref: { slug: string; source?: string | null; foundryUuid?: string | null } = { slug: m.slug };
            if (m.source) ref.source = m.source;
            if (m.foundryUuid) ref.foundryUuid = m.foundryUuid;
            return ref;
          }),
        };
        if (item.system.chassis != null) {
          drone.chassis = item.system.chassis;
        }
        drones.push(drone);
        break;
      }
      case "vehicle": {
        const vehicle: CharacterBuild["vehicles"][number] = {
          reference: itemToRef(item),
        };
        if (item.system.role != null) {
          vehicle.role = item.system.role;
        }
        vehicles.push(vehicle);
        break;
      }
    }
  }

  return {
    feats,
    classFeatures,
    equipment: { weapons, armor, inventory },
    spellcasting: { entries: spellcastingEntries },
    actions,
    reactions,
    explorationActivities,
    notes,
    drones,
    vehicles,
  };
}

function itemToRef(item: FoundryItem): {
  slug: string;
  source?: string | null;
  foundryUuid?: string | null;
} {
  const ref: { slug: string; source?: string | null; foundryUuid?: string | null } = {
    slug: item.system.slug ?? item.name,
  };
  if (item.system.source) {
    ref.source = item.system.source;
  }
  if (item.uuid) {
    ref.foundryUuid = item.uuid;
  }
  return ref;
}

/**
 * Import a Foundry Actor into a canonical `PartialBuild` (§7.2).
 *
 * Import is the inverse of `exportActor`: group `actor.items` by `item.type`
 * and reconstruct canonical collections, using each item's `system.slug` and
 * `system.source` where present (§7.2).
 */
export function importActor(actor: FoundryActor): {
  identity: CharacterBuild["identity"];
  ancestry: CharacterBuild["ancestry"];
  heritage: CharacterBuild["heritage"];
  background: CharacterBuild["background"];
  class: CharacterBuild["class"];
  level: CharacterBuild["level"];
  abilityScores: CharacterBuild["abilityScores"];
  skills: CharacterBuild["skills"];
  proficiencies: CharacterBuild["proficiencies"];
  currencies: CharacterBuild["currencies"];
  feats: CharacterBuild["feats"];
  classFeatures: CharacterBuild["classFeatures"];
  equipment: CharacterBuild["equipment"];
  spellcasting: CharacterBuild["spellcasting"];
  actions: CharacterBuild["actions"];
  reactions: CharacterBuild["reactions"];
  explorationActivities: CharacterBuild["explorationActivities"];
  notes: CharacterBuild["notes"];
  drones: CharacterBuild["drones"];
  vehicles: CharacterBuild["vehicles"];
} {
  const imported = importItems(actor.items);

  const ancestryRef = detailToRef(actor.system.details.ancestry);
  const heritageRef = detailToRef(actor.system.details.heritage);
  const backgroundRef = detailToRef(actor.system.details.background);
  const classDetail = actor.system.details.class;
  const classRef = detailToRef(classDetail);

  return {
    identity: { name: actor.name },
    ancestry: {
      reference: ancestryRef.reference,
      ...(ancestryRef.displayName !== undefined ? { displayName: ancestryRef.displayName } : {}),
    },
    heritage: {
      reference: heritageRef.reference,
      ...(heritageRef.displayName !== undefined ? { displayName: heritageRef.displayName } : {}),
    },
    background: {
      reference: backgroundRef.reference,
      ...(backgroundRef.displayName !== undefined ? { displayName: backgroundRef.displayName } : {}),
    },
    class: {
      reference: classRef.reference,
      keyAbility: classDetail.keyAbility,
      hpPerLevel: classDetail.hpPerLevel,
    },
    level: actor.system.details.level.value,
    abilityScores: importAbilities(actor.system.abilities),
    skills: importSkills(actor.system.skills),
    proficiencies: importProficiencies(actor.system.proficiencies),
    currencies: importCurrencies(actor.system.currency),
    ...imported,
  };
}
