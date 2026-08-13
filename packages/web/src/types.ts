// Wire types matching the REST DTOs (crates/api/src/dto.rs). The canonical
// build is carried as raw JSON (`JsonValue`) — the UI never reshapes it, only
// displays it and hands it to the WASM engine.

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface CharacterSummary {
  id: string;
  system: string;
  display_name: string;
  content_hash: string;
  head_snapshot_id: string | null;
  updated_at: string;
}

export interface ListCharactersResponse {
  characters: CharacterSummary[];
  next_cursor: string | null;
}

export interface GetCharacterResponse {
  id: string;
  system: string;
  display_name: string;
  content_hash: string;
  head_snapshot_id: string | null;
  build: JsonValue;
  created_at: string;
  updated_at: string;
}

export interface SnapshotSummary {
  id: string;
  parent: string | null;
  merge_parent: string | null;
  content_hash: string;
  origin: JsonValue;
  created_at: string;
  created_by: string;
}

export interface HistoryResponse {
  snapshots: SnapshotSummary[];
  next_cursor: string | null;
}

export interface SnapshotResponse {
  id: string;
  character_id: string;
  parent: string | null;
  merge_parent: string | null;
  content_hash: string;
  origin: JsonValue;
  created_at: string;
  created_by: string;
  build: JsonValue;
}

// A single field diff in the wire format (sync_engine::json::field_diff_to_json).
export interface FieldDiff {
  path: string;
  change: "added" | "removed" | "modified";
  before?: JsonValue;
  after?: JsonValue;
}

export interface DiffResponse {
  diffs: FieldDiff[];
}

export interface Conflict {
  path: string;
  ancestor: JsonValue;
  ours: JsonValue;
  theirs: JsonValue;
}

export interface MergePreview {
  autoMerged: FieldDiff[];
  conflicts: Conflict[];
}

// Result of WASM diff(): { diffs: [...] } | { error: "..." }
export interface WasmDiffResult {
  diffs?: FieldDiff[];
  error?: string;
}

export interface WasmMergeResult {
  mergePreview?: MergePreview;
  error?: string;
}

export interface WasmApplyResult {
  merged?: JsonValue;
  error?: string;
}

export interface WasmHashResult {
  contentHash?: string;
  error?: string;
}

export interface CreateCharacterResponse {
  character_id: string;
  snapshot_id: string;
  content_hash: string;
}
