// Typed REST client for the Nexus Sheet API (§13). Every request carries the
// `Authorization: Bearer nsk_live_…` header (§17). The base URL is configurable
// so the UI can talk to a remote API in dev (CORS is permissive on the server).

import type {
  CreateCharacterResponse,
  DiffResponse,
  GetCharacterResponse,
  HistoryResponse,
  JsonValue,
  ListCharactersResponse,
  SnapshotResponse,
} from "./types";

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
    public body?: JsonValue,
  ) {
    super(message);
  }
}

export class ApiClient {
  constructor(public baseUrl: string, public apiKey: string) {}

  private async req<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.apiKey}`,
    };
    if (body !== undefined) headers["content-type"] = "application/json";
    const res = await fetch(this.baseUrl + path, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : null,
    });
    const text = await res.text();
    const json = text ? (JSON.parse(text) as JsonValue) : undefined;
    if (!res.ok) {
      const msg =
        (json && typeof json === "object" && "message" in json
          ? String((json as { message: unknown }).message)
          : res.statusText) || `HTTP ${res.status}`;
      throw new ApiError(res.status, msg, json);
    }
    return json as T;
  }

  listCharacters(): Promise<ListCharactersResponse> {
    return this.req<ListCharactersResponse>("GET", "/v1/characters");
  }

  getCharacter(id: string): Promise<GetCharacterResponse> {
    return this.req<GetCharacterResponse>("GET", `/v1/characters/${id}`);
  }

  getHistory(id: string): Promise<HistoryResponse> {
    return this.req<HistoryResponse>("GET", `/v1/characters/${id}/history`);
  }

  getSnapshot(characterId: string, snapId: string): Promise<SnapshotResponse> {
    return this.req<SnapshotResponse>(
      "GET",
      `/v1/characters/${characterId}/snapshots/${snapId}`,
    );
  }

  diffSnapshots(
    characterId: string,
    from: string,
    to: string,
  ): Promise<DiffResponse> {
    return this.req<DiffResponse>(
      "GET",
      `/v1/characters/${characterId}/diff?from=${from}&to=${to}`,
    );
  }

  createCharacter(
    build: JsonValue,
    system: string,
    displayName: string | null,
  ): Promise<CreateCharacterResponse> {
    const body: Record<string, unknown> = { build, system };
    if (displayName !== null) body.display_name = displayName;
    return this.req<CreateCharacterResponse>("POST", "/v1/characters", body);
  }
}
