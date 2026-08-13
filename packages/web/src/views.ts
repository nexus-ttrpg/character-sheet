// View renderers for the minimal web UI (Phase 3.1). Three surfaces:
//   1. Character list
//   2. Character detail — snapshot DAG timeline (History)
//   3. Diff (server ↔ WASM parity) + Merge Lab (client-side conflict resolution)
//
// The diff view is the §19 Phase 3 parity check made visible: it computes the
// diff both server-side (`GET /diff`) and client-side (WASM `diff`) on the same
// two snapshot builds and shows whether they agree. The merge lab drives
// conflict resolution entirely through the WASM engine (`merge` → resolve →
// `apply_merge`), exactly what the server would compute (§11.3).

import { h, clear, fmtJson, shortHash } from "./dom";
import { ApiClient } from "./api";
import type { WasmEngine, Resolution } from "./wasm";
import type {
  CharacterSummary,
  Conflict,
  FieldDiff,
  JsonValue,
  MergePreview,
  SnapshotSummary,
} from "./types";

export interface Ctx {
  api: ApiClient;
  wasm: WasmEngine | null;
  onOpenCharacter: (id: string) => void;
  onBack: () => void;
}

// ── Deep equality (key-order independent) for the parity check ──────────────

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== typeof b) return false;
  if (Array.isArray(a)) {
    return Array.isArray(b) && a.length === b.length && a.every((v, i) => deepEqual(v, b[i]));
  }
  if (typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a as object).sort();
    const kb = Object.keys(b as object).sort();
    return (
      ka.length === kb.length &&
      ka.every((k, i) => k === kb[i]) &&
      ka.every((k) => deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
    );
  }
  return false;
}

function originKind(origin: JsonValue): string {
  if (origin && typeof origin === "object" && "type" in origin) {
    return String((origin as { type: unknown }).type);
  }
  return "snapshot";
}

// ── Character list ──────────────────────────────────────────────────────────

export async function renderCharacterList(ctx: Ctx): Promise<HTMLElement> {
  const root = h("div");
  root.appendChild(h("h2", null, "Characters"));
  const panel = h("div", { class: "panel" });
  root.appendChild(panel);
  panel.appendChild(h("p", { class: "muted small" }, "Loading…"));

  let chars: CharacterSummary[];
  try {
    const page = await ctx.api.listCharacters();
    chars = page.characters;
  } catch (err) {
    clear(panel);
    panel.appendChild(
      h("div", { class: "banner err" }, `Failed to load characters: ${(err as Error).message}`),
    );
    return root;
  }

  clear(panel);
  if (chars.length === 0) {
    panel.appendChild(
      h("p", { class: "muted" }, "No characters yet. Create one via the API or CLI."),
    );
    return root;
  }

  const table = h(
    "table",
    null,
    h(
      "thead",
      null,
      h(
        "tr",
        null,
        h("th", null, "Name"),
        h("th", null, "System"),
        h("th", null, "Content hash"),
        h("th", null, "Updated"),
      ),
    ),
    h("tbody"),
  );
  const tbody = table.querySelector("tbody")!;
  for (const c of chars) {
    const tr = h(
      "tr",
      { class: "clickable" },
      h("td", null, c.display_name || h("span", { class: "muted" }, "(unnamed)")),
      h("td", null, h("span", { class: "badge" }, c.system)),
      h("td", { class: "mono hash" }, shortHash(c.content_hash)),
      h("td", { class: "small muted" }, new Date(c.updated_at).toLocaleString()),
    );
    tr.addEventListener("click", () => ctx.onOpenCharacter(c.id));
    tbody.appendChild(tr);
  }
  panel.appendChild(table);
  return root;
}

// ── Character detail (tabs) ─────────────────────────────────────────────────

export async function renderCharacterDetail(
  ctx: Ctx,
  characterId: string,
): Promise<HTMLElement> {
  const root = h("div");
  root.appendChild(
    h("div", { class: "row" }, h("button", { class: "btn", onClick: () => ctx.onBack() }, "← Back")),
  );

  const [char, history] = await Promise.all([
    ctx.api.getCharacter(characterId),
    ctx.api.getHistory(characterId),
  ]);
  const snaps = history.snapshots;

  root.append(
    h("h2", null, char.display_name || "(unnamed)"),
    h("div", { class: "row small muted" },
      h("span", { class: "badge" }, char.system),
      h("span", null, "id: "),
      h("span", { class: "mono" }, char.id),
      h("span", null, "head: "),
      h("span", { class: "mono hash" }, shortHash(char.content_hash)),
    ),
  );

  const tabs = h("div", { class: "tabs" });
  const content = h("div");
  root.append(tabs, content);

  const tabDefs = [
    { id: "history", label: "History (DAG)" },
    { id: "diff", label: "Diff (parity)" },
    { id: "merge", label: "Merge Lab" },
  ] as const;

  let active = "history";
  const renderTab = (id: string) => {
    active = id;
    clear(tabs);
    for (const t of tabDefs) {
      tabs.appendChild(
        h("div", { class: `tab ${t.id === id ? "active" : ""}`, onClick: () => renderTab(t.id) }, t.label),
      );
    }
    clear(content);
    if (id === "history") content.appendChild(renderHistory(ctx, snaps, char.build));
    else if (id === "diff") content.appendChild(renderDiffTab(ctx, characterId, snaps));
    else content.appendChild(renderMergeTab(ctx, characterId, snaps, char.build));
  };
  renderTab("history");
  return root;
}

// ── History: snapshot DAG timeline ──────────────────────────────────────────

function renderHistory(
  _ctx: Ctx,
  snaps: SnapshotSummary[],
  headBuild: JsonValue,
): HTMLElement {
  const panel = h("div", { class: "panel" });
  panel.appendChild(h("h3", null, "Snapshot DAG timeline"));
  if (snaps.length === 0) {
    panel.appendChild(h("p", { class: "muted" }, "No snapshots."));
    return panel;
  }
  const timeline = h("div", { class: "timeline" });
  // History is returned newest-first; render top-down as newest → oldest.
  for (const s of snaps) {
    const kind = originKind(s.origin);
    const cls = `snap ${kind === "merge" ? "merge" : kind === "rollback" ? "rollback" : ""}`;
    const badges: Node[] = [];
    if (s.merge_parent) badges.push(h("span", { class: "badge merge" }, "merge"));
    if (kind === "rollback") badges.push(h("span", { class: "badge rollback" }, "rollback"));
    const snap = h(
      "div",
      { class: cls, title: s.id },
      h("div", { class: "snap-meta" },
        h("span", { class: "mono small" }, shortHash(s.id)),
        ...badges,
        h("span", { class: "snap-hash mono" }, "hash: " + shortHash(s.content_hash)),
        h("span", { class: "small muted" }, new Date(s.created_at).toLocaleString()),
      ),
      h("div", { class: "small muted" },
        s.parent ? `parent: ${shortHash(s.parent)}` : "root",
        s.merge_parent ? `  merge-parent: ${shortHash(s.merge_parent)}` : "",
        `  by: ${shortHash(s.created_by)}`,
      ),
    );
    timeline.appendChild(snap);
  }
  panel.appendChild(timeline);
  panel.appendChild(
    h("details", { style: "margin-top:12px" },
      h("summary", { class: "small muted" }, "HEAD build (raw JSON)"),
      h("pre", { class: "json-out" }, fmtJson(headBuild)),
    ),
  );
  return panel;
}

// ── Diff tab: server ↔ WASM parity ──────────────────────────────────────────

function snapshotSelect(label: string, snaps: SnapshotSummary[]): HTMLSelectElement {
  const sel = h("select") as HTMLSelectElement;
  sel.appendChild(h("option", { value: "" }, `— ${label} —`));
  for (const s of snaps) {
    sel.appendChild(
      h(
        "option",
        { value: s.id },
        `${shortHash(s.id)}  ${originKind(s.origin)}  ${new Date(s.created_at).toLocaleTimeString()}`,
      ),
    );
  }
  return sel;
}

function renderDiffTab(
  ctx: Ctx,
  characterId: string,
  snaps: SnapshotSummary[],
): HTMLElement {
  const panel = h("div", { class: "panel" });
  panel.appendChild(h("h3", null, "Diff — server vs. WASM parity"));
  panel.appendChild(
    h("p", { class: "small muted" },
      "Picks two snapshots and computes the field diff both server-side ",
      h("code", null, "GET /diff"),
      " and client-side via the WASM engine on the same two builds (§19 Phase 3).",
    ),
  );

  if (!ctx.wasm) {
    panel.appendChild(
      h("div", { class: "banner warn" }, "WASM engine unavailable — run `pnpm build:wasm`. Only the server diff will show."),
    );
  }

  const fromSel = snapshotSelect("from snapshot", snaps);
  const toSel = snapshotSelect("to snapshot", snaps);
  const out = h("div");
  const compute = h("button", { class: "btn primary" }, "Compute diff");
  compute.addEventListener("click", async () => {
    const from = fromSel.value;
    const to = toSel.value;
    if (!from || !to) {
      clear(out);
      out.appendChild(h("div", { class: "banner warn" }, "Select both a from and to snapshot."));
      return;
    }
    clear(out);
    out.appendChild(h("p", { class: "muted small" }, "Computing…"));
    try {
      const [serverDiff, fromSnap, toSnap] = await Promise.all([
        ctx.api.diffSnapshots(characterId, from, to),
        ctx.api.getSnapshot(characterId, from),
        ctx.api.getSnapshot(characterId, to),
      ]);
      let wasmDiffs: FieldDiff[] | null = null;
      if (ctx.wasm) {
        const r = ctx.wasm.diffBuilds(fromSnap.build, toSnap.build);
        if (r.error) throw new Error(`WASM diff: ${r.error}`);
        wasmDiffs = r.diffs ?? [];
      }
      clear(out);
      const match = wasmDiffs !== null && deepEqual(serverDiff.diffs, wasmDiffs);
      if (wasmDiffs !== null) {
        out.appendChild(
          h("div", { class: `banner ${match ? "warn" : "err"}` },
            match
              ? "✓ Parity: server and WASM diffs match exactly."
              : "✗ MISMATCH: server and WASM diffs differ — see below.",
          ),
        );
      }
      out.appendChild(h("h3", null, "Field diffs"));
      out.appendChild(renderDiffList(serverDiff.diffs));
      if (!match && wasmDiffs !== null) {
        out.appendChild(h("h3", null, "WASM diffs (mismatch)"));
        out.appendChild(renderDiffList(wasmDiffs));
      }
    } catch (err) {
      clear(out);
      out.appendChild(h("div", { class: "banner err" }, `Error: ${(err as Error).message}`));
    }
  });

  panel.append(
    h("div", { class: "row" }, fromSel, h("span", { class: "muted" }, "→"), toSel, compute),
    out,
  );
  return panel;
}

function renderDiffList(diffs: FieldDiff[]): HTMLElement {
  if (diffs.length === 0) {
    return h("p", { class: "muted" }, "No differences.");
  }
  const list = h("div", { class: "diff-list" });
  for (const d of diffs) {
    const vals = h("div", { class: "vals" });
    if (d.change === "modified") {
      vals.appendChild(h("span", { class: "before" }, fmtJson(d.before)));
      vals.appendChild(h("span", { class: "arrow" }, "→"));
      vals.appendChild(h("span", { class: "after" }, fmtJson(d.after)));
    } else if (d.change === "added") {
      vals.appendChild(h("span", { class: "after" }, fmtJson(d.after)));
    } else {
      vals.appendChild(h("span", { class: "before" }, fmtJson(d.before)));
    }
    list.appendChild(h("div", { class: "diff-row" }, h("div", { class: "path" }, d.path), h("div", { class: "chg" }, d.change), vals));
  }
  return list;
}

// ── Merge Lab: client-side conflict resolution ─────────────────────────────

interface MergeState {
  preview: MergePreview | null;
  ours: JsonValue;
  resolutions: Map<string, "ours" | "theirs" | "custom">;
  customValues: Map<string, string>;
}

function renderMergeTab(
  ctx: Ctx,
  characterId: string,
  snaps: SnapshotSummary[],
  headBuild: JsonValue,
): HTMLElement {
  const panel = h("div", { class: "panel" });
  panel.appendChild(h("h3", null, "Merge Lab — client-side conflict resolution"));
  panel.appendChild(
    h("p", { class: "small muted" },
      "Three-way merge (§11.3) computed entirely in the browser by the WASM engine. ",
      "Resolve each conflict per-field, then apply to produce the merged build.",
    ),
  );

  if (!ctx.wasm) {
    panel.appendChild(
      h("div", { class: "banner err" }, "WASM engine unavailable — run `pnpm build:wasm`. The Merge Lab needs the client-side engine."),
    );
    return panel;
  }

  const ancestorSel = snapshotSelect("ancestor", snaps);
  const oursSel = snapshotSelect("ours (HEAD)", snaps);
  const theirsSel = snapshotSelect("theirs", snaps);
  const ancestorBox = h("textarea", { placeholder: "ancestor build JSON" }) as HTMLTextAreaElement;
  const oursBox = h("textarea", { placeholder: "ours build JSON" }) as HTMLTextAreaElement;
  const theirsBox = h("textarea", { placeholder: "theirs build JSON (the incoming change)" }) as HTMLTextAreaElement;
  // Default ours to HEAD.
  oursBox.value = JSON.stringify(headBuild, null, 2);

  // Load a snapshot's build into a textarea via GET /v1/characters/:id/snapshots/:snap.
  const loadSnap = async (sel: HTMLSelectElement, box: HTMLTextAreaElement) => {
    if (!sel.value) return;
    box.value = "// loading…";
    try {
      const s = await ctx.api.getSnapshot(characterId, sel.value);
      box.value = JSON.stringify(s.build, null, 2);
    } catch (err) {
      box.value = `// error: ${(err as Error).message}`;
    }
  };

  const state: MergeState = {
    preview: null,
    ours: headBuild,
    resolutions: new Map(),
    customValues: new Map(),
  };

  const out = h("div");

  const compute = h("button", { class: "btn primary" }, "Compute merge preview");
  compute.addEventListener("click", () => {
    clear(out);
    let ancestor: JsonValue, ours: JsonValue, theirs: JsonValue;
    try {
      ancestor = JSON.parse(ancestorBox.value) as JsonValue;
      ours = JSON.parse(oursBox.value) as JsonValue;
      theirs = JSON.parse(theirsBox.value) as JsonValue;
    } catch (err) {
      out.appendChild(h("div", { class: "banner err" }, `Invalid JSON: ${(err as Error).message}`));
      return;
    }
    const r = ctx.wasm!.mergeBuilds(ancestor, ours, theirs);
    if (r.error) {
      out.appendChild(h("div", { class: "banner err" }, `WASM merge error: ${r.error}`));
      return;
    }
    state.preview = r.mergePreview ?? null;
    state.ours = ours;
    state.resolutions.clear();
    state.customValues.clear();
    renderMergeResult(ctx, state, out);
  });

  panel.append(
    h("div", { class: "row" },
      h("label", { class: "small muted" }, "ancestor", ancestorSel, h("button", { class: "btn small", onClick: () => loadSnap(ancestorSel, ancestorBox) }, "load")),
      h("label", { class: "small muted" }, "ours", oursSel, h("button", { class: "btn small", onClick: () => loadSnap(oursSel, oursBox) }, "load")),
      h("label", { class: "small muted" }, "theirs", theirsSel, h("button", { class: "btn small", onClick: () => loadSnap(theirsSel, theirsBox) }, "load")),
    ),
    h("div", { class: "row grow" },
      h("div", { style: "flex:1" }, h("div", { class: "small muted" }, "ancestor"), ancestorBox),
      h("div", { style: "flex:1" }, h("div", { class: "small muted" }, "ours"), oursBox),
      h("div", { style: "flex:1" }, h("div", { class: "small muted" }, "theirs"), theirsBox),
    ),
    h("div", { class: "row" }, compute),
    out,
  );
  return panel;
}

function renderMergeResult(ctx: Ctx, state: MergeState, out: HTMLElement): void {
  clear(out);
  const preview = state.preview;
  if (!preview) return;

  out.appendChild(
    h("div", { class: "row small" },
      h("span", { class: "badge ok" }, `${preview.autoMerged.length} auto-merged`),
      h("span", { class: "badge warn" }, `${preview.conflicts.length} conflicts`),
    ),
  );

  if (preview.autoMerged.length > 0) {
    out.appendChild(h("h3", null, "Auto-merged"));
    out.appendChild(renderDiffList(preview.autoMerged));
  }

  if (preview.conflicts.length > 0) {
    out.appendChild(h("h3", null, "Conflicts — resolve each"));
    for (const c of preview.conflicts) {
      out.appendChild(renderConflict(ctx, state, c, out));
    }
  }

  const apply = h("button", { class: "btn primary" }, "Apply merge → build");
  apply.addEventListener("click", () => applyMerge(ctx, state, out));
  out.appendChild(h("div", { class: "row", style: "margin-top:12px" }, apply));
}

function renderConflict(
  _ctx: Ctx,
  state: MergeState,
  c: Conflict,
  out: HTMLElement,
): HTMLElement {
  const setRes = (val: "ours" | "theirs" | "custom") => {
    state.resolutions.set(c.path, val);
  };
  const customBox = h("textarea", { class: "small", placeholder: "custom JSON value" }) as HTMLTextAreaElement;
  customBox.value = fmtJson(c.theirs);
  customBox.addEventListener("input", () => {
    state.customValues.set(c.path, customBox.value);
    setRes("custom");
    oursRadio.checked = false;
    theirsRadio.checked = false;
    customRadio.checked = true;
  });

  const oursRadio = h("input", { type: "radio", name: `r-${c.path}`, onChange: () => setRes("ours") }) as HTMLInputElement;
  const theirsRadio = h("input", { type: "radio", name: `r-${c.path}`, onChange: () => setRes("theirs") }) as HTMLInputElement;
  const customRadio = h("input", { type: "radio", name: `r-${c.path}`, onChange: () => setRes("custom") }) as HTMLInputElement;
  // Default to "theirs" for the common pull case.
  theirsRadio.checked = true;
  setRes("theirs");

  return h("div", { class: "conflict" },
    h("div", { class: "cpath" }, c.path),
    h("div", { class: "sides" },
      h("div", { class: "side" },
        h("label", null, "ancestor"),
        h("pre", null, fmtJson(c.ancestor)),
      ),
      h("div", { class: "side" },
        h("label", null, "ours"),
        h("pre", null, fmtJson(c.ours)),
      ),
      h("div", { class: "side" },
        h("label", null, "theirs"),
        h("pre", null, fmtJson(c.theirs)),
      ),
    ),
    h("div", { class: "res-options" },
      h("label", null, oursRadio, "ours"),
      h("label", null, theirsRadio, "theirs"),
      h("label", null, customRadio, "custom:"),
      customBox,
    ),
  );
  // `out` is passed so the apply button can be re-rendered; not used directly.
  void out;
}

function applyMerge(ctx: Ctx, state: MergeState, out: HTMLElement): void {
  const preview = state.preview;
  if (!preview) return;
  const resolutions: Resolution[] = [];
  for (const c of preview.conflicts) {
    const choice = state.resolutions.get(c.path) ?? "theirs";
    if (choice === "custom") {
      const raw = state.customValues.get(c.path) ?? "";
      let val: JsonValue;
      try {
        val = JSON.parse(raw) as JsonValue;
      } catch (err) {
        clear(out);
        out.appendChild(h("div", { class: "banner err" }, `Invalid custom JSON for ${c.path}: ${(err as Error).message}`));
        return;
      }
      resolutions.push({ path: c.path, resolution: val });
    } else {
      resolutions.push({ path: c.path, resolution: choice });
    }
  }
  const r = ctx.wasm!.applyMerge(state.ours, preview, resolutions);
  if (r.error) {
    clear(out);
    out.appendChild(h("div", { class: "banner err" }, `WASM apply_merge error: ${r.error}`));
    return;
  }
  const merged = r.merged!;
  const hashR = ctx.wasm!.contentHash(merged);
  const hash = hashR.contentHash ?? "(hash error)";

  clear(out);
  out.appendChild(h("div", { class: "banner warn" }, "✓ Merge applied client-side (WASM)."));
  out.appendChild(
    h("div", { class: "row small" },
      h("span", null, "merged content hash: "),
      h("span", { class: "mono hash" }, shortHash(hash)),
    ),
  );
  out.appendChild(h("h3", null, "Merged build"));
  out.appendChild(h("pre", { class: "json-out" }, fmtJson(merged)));

  const createBtn = h("button", { class: "btn primary" }, "Create new character from merged build");
  createBtn.addEventListener("click", async () => {
    createBtn.disabled = true;
    createBtn.textContent = "creating…";
    try {
      const resp = await ctx.api.createCharacter(merged, "pf2e", null);
      clear(out);
      out.appendChild(h("div", { class: "banner warn" }, `✓ Created character ${resp.character_id} (snapshot ${shortHash(resp.snapshot_id)}).`));
    } catch (err) {
      createBtn.disabled = false;
      createBtn.textContent = "Create new character from merged build";
      out.appendChild(h("div", { class: "banner err" }, `Create failed: ${(err as Error).message}`));
    }
  });
  out.appendChild(h("div", { class: "row", style: "margin-top:10px" }, createBtn));
}
