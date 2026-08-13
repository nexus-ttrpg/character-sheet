// Entry point — wires the topbar (API base URL + key, persisted in localStorage),
// loads the WASM engine, and routes between the character list and detail views.
//
// The app is a minimal SPA: a hash route (`#/` or `#/characters/<id>`) selects
// which view renders into `#app`. No framework — just direct DOM manipulation
// via the `h()` helper and the view renderers in views.ts.

import { ApiClient } from "./api";
import { loadWasm, type WasmEngine } from "./wasm";
import { renderCharacterList, renderCharacterDetail, type Ctx } from "./views";
import { clear } from "./dom";

const LS_BASE_URL = "nexus.baseUrl";
const LS_API_KEY = "nexus.apiKey";

const baseUrlInput = document.getElementById("base-url") as HTMLInputElement;
const apiKeyInput = document.getElementById("api-key") as HTMLInputElement;
const connForm = document.getElementById("conn-form") as HTMLFormElement;
const connStatus = document.getElementById("conn-status") as HTMLSpanElement;
const app = document.getElementById("app")!;

let wasm: WasmEngine | null = null;
let api: ApiClient | null = null;

function setConnStatus(text: string, ok: boolean): void {
  connStatus.textContent = text;
  connStatus.className = `conn-status ${ok ? "ok" : "err"}`;
}

function buildCtx(): Ctx {
  return {
    api: api!,
    wasm,
    onOpenCharacter: (id: string) => {
      window.location.hash = `#/characters/${id}`;
    },
    onBack: () => {
      window.location.hash = "#/";
    },
  };
}

async function renderRoute(): Promise<void> {
  if (!api) return;
  clear(app);
  const hash = window.location.hash;
  const match = hash.match(/^#\/characters\/(.+)$/);
  if (match) {
    const id = decodeURIComponent(match[1]!);
    app.appendChild(document.createTextNode("Loading…"));
    try {
      const el = await renderCharacterDetail(buildCtx(), id);
      clear(app);
      app.appendChild(el);
    } catch (err) {
      clear(app);
      app.appendChild(
        document.createTextNode(`Error: ${(err as Error).message}`),
      );
    }
  } else {
    try {
      const el = await renderCharacterList(buildCtx());
      clear(app);
      app.appendChild(el);
    } catch (err) {
      clear(app);
      app.appendChild(
        document.createTextNode(`Error: ${(err as Error).message}`),
      );
    }
  }
}

connForm.addEventListener("submit", (e) => {
  e.preventDefault();
  const baseUrl = baseUrlInput.value.trim().replace(/\/+$/, "");
  const apiKey = apiKeyInput.value.trim();
  if (!baseUrl || !apiKey) {
    setConnStatus("Enter base URL and API key.", false);
    return;
  }
  localStorage.setItem(LS_BASE_URL, baseUrl);
  localStorage.setItem(LS_API_KEY, apiKey);
  api = new ApiClient(baseUrl, apiKey);
  setConnStatus("connected", true);
  void renderRoute();
});

window.addEventListener("hashchange", () => void renderRoute());

async function init(): Promise<void> {
  // Restore persisted credentials.
  baseUrlInput.value = localStorage.getItem(LS_BASE_URL) ?? "";
  apiKeyInput.value = localStorage.getItem(LS_API_KEY) ?? "";

  // Load WASM engine in the background (non-blocking — views degrade without it).
  void loadWasm().then((eng) => {
    wasm = eng;
  });

  // Auto-connect if credentials are already saved.
  if (baseUrlInput.value && apiKeyInput.value) {
    api = new ApiClient(baseUrlInput.value.trim(), apiKeyInput.value.trim());
    setConnStatus("connected", true);
    void renderRoute();
  } else {
    app.appendChild(
      document.createTextNode("Enter your API base URL and key above, then click Connect."),
    );
  }
}

void init();
