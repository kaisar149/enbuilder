// Contract types for API version 1 (mirrors wb-app/src/api.rs, schema.rs and wb-server/src/protocol.rs).
// Hand-written; `test/contract.test.ts` checks them against the golden files and a live server so they cannot drift silently.
export const API_VERSION = 1;
export const WIRE_VERSION = 1;

export type Command =
  | { type: 'load_preset'; name: string }
  | { type: 'set_param'; name: string; value: number }
  | { type: 'solve' }
  | { type: 'cancel' }
  | { type: 'undo' }
  | { type: 'redo' }
  /** Part 9c: background job solving `count` alternative seeds (coarse = one year of spin-up). Changes no parameter. */
  | { type: 'variations'; count: number; coarse?: boolean }
  /** Part 9f: one brush stroke on a paint layer (weights in [0,1] per cell; radius and falloff are applied by the client). One undo step. */
  | { type: 'paint_stroke'; layer: PaintLayerId; op: PaintOp; cells: number[]; weights: number[]; value?: number | null; strength?: number }
  /** Part 9f: lock (at the current value) or unlock cells of an output field. */
  | { type: 'set_lock'; field: OutputField; cells: number[]; locked: boolean }
  /** Part 9f: realism 0..100 for one output field or `all`. Instant (re-blend, no solve). */
  | { type: 'set_realism'; field: OutputField | 'all'; value: number }
  /** Part 9f: background re-solve after edits. */
  | { type: 'refresh'; mode: RefreshMode };

export type OutputField = 'elevation' | 'temperature' | 'precipitation' | 'biome';
export type PaintLayerId = 'elevation' | 'temperature_bias' | 'rainfall_bias' | 'biome_override';
export type PaintOp = 'add' | 'set' | 'smooth' | 'flatten' | 'erase';
export type RefreshMode = 'preview' | 'refined' | 'full';
export type FieldViewName = 'blended' | 'computed' | 'painted';

export type Query =
  | { type: 'get_schema' }
  | { type: 'get_world_info' }
  | { type: 'get_mesh' }
  | { type: 'get_field'; name: string; layer?: number }
  | { type: 'get_cell'; cell: number }
  /** Part 9e: river polylines as cell ids (needs a solve). */
  | { type: 'get_rivers' }
  /** Part 9e: edges between cells with `field > threshold` and their neighbours outside (any field). */
  | { type: 'get_outline'; field: string; threshold?: number }
  | { type: 'get_variations' }
  | { type: 'get_variation'; index: number; name: string }
  /** Part 9f */
  | { type: 'get_field_view'; name: string; layer?: number; view?: FieldViewName }
  | { type: 'get_paint'; layer: PaintLayerId }
  | { type: 'get_locks' }
  /** Part 9g */
  | { type: 'get_versions' }
  | { type: 'get_diff'; field: string; from_version: number; to_version: number; min_rel?: number }
  | { type: 'get_diff_summary'; from_version: number; to_version: number; min_rel?: number }
  | { type: 'explain'; cell: number; field: string; from_version?: number; to_version?: number; min_rel?: number };

export interface Ack {
  api_version: number;
  command: string;
  /** Solved fields that are now stale. */
  invalidated: string[];
  /** Set by `solve` and `refresh`. */
  job: number | null;
  /** Part 9f: `paint_stroke` -> { applied, skipped_locked }; `set_lock` -> detail of the lock change. */
  detail?: { applied?: number; skipped_locked?: number; [k: string]: unknown };
}

export type JobStatus = 'completed' | 'cancelled' | 'failed';

export type WbEvent =
  | { type: 'job_progress'; job: number; fraction: number; stage: string }
  | { type: 'job_done'; job: number; status: JobStatus }
  | { type: 'field_changed'; names: string[]; version: number }
  | { type: 'world_loaded'; mesh_id: string; n_cells: number }
  | { type: 'error'; message: string };

export interface ApiErrorBody {
  code: string;
  message: string;
}

// ---- schema -------------------------------------------------------------------------------------------------------------------

export type FieldKind = 'scalar' | 'vector' | 'mask' | 'category' | 'lines';

export interface ParamDef {
  name: string;
  group: string;
  unit: string;
  min: number;
  max: number;
  default: number;
  widget: 'slider' | 'integer' | 'select' | string;
  scope: 'climate' | 'elevation' | 'mesh' | string;
  description: string;
  options?: string[] | null;
  /** Part 9c: true for parameters that define the world's identity (shape, resolution, radius, seed). Ignored by the preset-dirty check. */
  identity?: boolean;
}

export interface GroupDef {
  name: string;
  title: string;
  /** Advanced groups start collapsed. */
  advanced: boolean;
}

export interface PresetDef {
  name: string;
  description: string;
  /** Parameter overrides the preset applies (names not listed keep their default). */
  params?: Record<string, number>;
  [k: string]: unknown;
}

export interface FieldMeta {
  name: string;
  kind: FieldKind;
  unit: string;
  display: { scale: number; offset: number; unit: string };
  colormap: string;
  /** Fixed colour range in SI units, or null = use the data min/max. */
  range: [number, number] | null;
  description: string;
  /** Curated fields come first in layer lists. */
  featured: boolean;
  /** Category fields only: `biomes` = schema biome table, `hash` = deterministic colour per id. */
  palette?: 'biomes' | 'hash';
  produced_by?: string[];
}

export interface BiomeDef {
  id: number;
  name: string;
  color: [number, number, number];
}

export interface Schema {
  api_version: number;
  params: ParamDef[];
  presets: PresetDef[];
  modules: { name: string; inputs: string[]; outputs: string[]; anomaly_hooks: string[]; params: { name: string; default: number }[] }[];
  loops: { name: string; members: string[] }[];
  fields: FieldMeta[];
  biomes: BiomeDef[];
  colormaps: string[];
  /** Part 9c: display order and titles of the parameter groups. */
  groups?: GroupDef[];
  /** Part 9c: stored season layers: `variants[field]` is a field with `count` layers. */
  seasons?: { count: number; variants: Record<string, string> };
  /** Part 9e: overlay declarations (vector / lines / mask), layer presets and the layer kinds a renderer must handle. */
  overlays?: OverlayDef[];
  layer_presets?: LayerPresetDef[];
  layer_kinds?: string[];
  /** Part 9f */
  edit?: EditSchema;
}

export type OverlayKind = 'vector' | 'lines' | 'mask';

export interface OverlayDef {
  id: string;
  label: string;
  kind: OverlayKind;
  /** vector, mask: the field it reads. */
  field?: string;
  /** lines: the geometry query that produces it (`rivers`). */
  source?: string;
  /** lines: field that gives the line width (per vertex, from the geometry answer). */
  weight_field?: string;
  /** mask: cells with `field > threshold` are inside. */
  threshold?: number;
  mode?: 'fill' | 'outline';
  /** Theme token suffix: the colour is `--overlay-<token>`. */
  token: string;
  animated?: boolean;
  unit_hint?: string;
}

export interface LayerPresetDef {
  id: string;
  name: string;
  description: string;
  /** Scalar base field. */
  base: string;
  /** Overlay ids, bottom to top. */
  overlays: string[];
}

// ---- world info ---------------------------------------------------------------------------------------------------------------

export interface WorldInfo {
  api_version: number;
  preset: string;
  params: Record<string, number>;
  domain: { kind: 'sphere' | 'cube' | 'plane' | string; mesh_id: string; n_cells: number; total_area_m2: number };
  solved: boolean;
  stale: boolean;
  fields: { name: string; layers: number; version: number; stale: boolean }[];
  job: { id: number; running: boolean; /** Part 9f */ mode?: 'full' | 'preview' | 'refined' | 'variations' } | null;
  /** Part 9f */
  edit?: EditInfo;
  can_undo: boolean;
  can_redo: boolean;
  /** Part 9c. `undo` is oldest first; `redo` lists the next step to re-apply first. */
  history?: { undo: HistoryEntry[]; redo: HistoryEntry[] };
}

export interface HistoryEntry {
  label: string;
  preset: string;
  changes: { name: string; old: number; new: number }[];
  /** Part 9f: what kind of step this is (absent in old servers = params). */
  kind?: 'params' | 'stroke' | 'lock' | 'realism';
  layer?: string;
  field?: string;
  cells?: number;
}

/** Part 9f: the `edit` block of world_info. */
export interface EditInfo {
  /** Bumps on every paint, lock or realism change; caches of paint and lock data are keyed by it. */
  generation: number;
  realism: Record<OutputField, number>;
  locks: Record<OutputField, number>;
  paint: Record<PaintLayerId, number>;
  /** Quality of the solved state the fields come from (null = never solved). */
  quality: 'preview' | 'refined' | 'full' | null;
  last_refresh: { mode: RefreshMode; seasons_run: number; ms: number } | null;
}

/** Part 9f: paint layer (value + coverage mask, f32 per cell). */
export interface PaintData {
  meshId: string;
  layer: PaintLayerId;
  output: OutputField;
  generation: number;
  paintedCells: number;
  value: Float32Array;
  mask: Float32Array;
}

/** Part 9f: locked cell ids per output field (ascending). */
export interface LocksData {
  meshId: string;
  generation: number;
  cells: Record<OutputField, Uint32Array>;
}

/** Part 9f: `schema.edit`, everything the tool panel needs (tools, brush limits, realism, refresh modes) as data. */
export interface ToolDef {
  id: string;
  label: string;
  hint: string;
  kind: 'paint' | 'erase' | 'lock';
  layer?: PaintLayerId | 'any';
  op?: PaintOp;
  sign?: 1 | -1;
  needs_value?: boolean;
  value_from?: 'biomes';
  needs_field?: boolean;
  locked?: boolean;
  amount?: { default: number; min: number; max: number; unit: string } | null;
}

export interface EditSchema {
  tools: ToolDef[];
  brush: { radius_min_km: number; radius_max_km: number; radius_default_km: number; strength_default: number; falloffs: string[]; falloff_default: string };
  layers: { id: PaintLayerId; label: string; output: OutputField; unit: string; min: number; max: number | null }[];
  lockable: OutputField[];
  realism: { fields: OutputField[]; min: number; max: number; default: number };
  refresh_modes: { id: RefreshMode; label: string; description: string; target_ms?: number; target_ms_max?: number }[];
}

export interface VariationsInfo {
  mesh_id: string;
  base_seed: number | null;
  coarse: boolean | null;
  /** Fields kept per variation (elevation, temperature, precipitation, biome). */
  fields: string[];
  items: { index: number; seed: number }[];
}

// ---- mesh and fields ----------------------------------------------------------------------------------------------------------

export interface MeshHeader {
  mesh_id: string;
  kind: string;
  n_cells: number;
  endianness: 'little';
  layout: { name: string; dtype: string; count: number; offset: number }[];
  bytes: number;
}

export interface MeshData {
  meshId: string;
  kind: string;
  nCells: number;
  /** x,y,z per cell (3 * nCells). */
  centroid: Float32Array;
  area: Float32Array;
  /** CSR adjacency: neighbours of cell i are adj[adjOffsets[i] .. adjOffsets[i+1]]. */
  adjOffsets: Uint32Array;
  adj: Uint32Array;
}

export interface FieldHeader extends FieldMeta {
  mesh_id: string;
  version: number;
  stale: boolean;
  layer: number;
  layers: number;
  min: number;
  max: number;
  dtype: 'f32';
  count: number;
}

export interface FieldData {
  name: string;
  layer: number;
  layers: number;
  meshId: string;
  version: number;
  stale: boolean;
  /** min/max of the finite values (SI units). */
  min: number;
  max: number;
  meta: FieldMeta;
  values: Float32Array;
}

/** A field of one variation (carries the seed that produced it). */
export type VariationField = FieldData & { seed: number; variation: number };

export interface CellInfo {
  cell: number;
  centroid: [number, number, number];
  area_m2: number;
  values: Record<string, number>;
  biome_name: string | null;
  neighbors: number[];
  stale: boolean;
  lat_deg?: number;
  lon_deg?: number;
  /** Part 9e: twelve season values per seasonal field (temperature, precipitation, soil_moisture); only after a solve. */
  seasonal?: Record<string, number[]>;
  /** Part 9f: per output field, how much of this value is painted (mask), whether it is locked, and the realism. */
  edit_state?: Record<OutputField, { painted: number; locked: boolean; realism: number }>;
}

/** River polylines (topology only). Line k is `cells[offsets[k] .. offsets[k+1]]`; `discharge` is per vertex (m3/s). */
export interface RiversData {
  meshId: string;
  version: number;
  stale: boolean;
  nLines: number;
  nPoints: number;
  offsets: Uint32Array;
  cells: Uint32Array;
  discharge: Float32Array;
}

/** Outline edges: edge k joins `pairs[2k]` (inside) and `pairs[2k+1]` (outside neighbour). */
export interface OutlineData {
  meshId: string;
  field: string;
  threshold: number;
  version: number;
  stale: boolean;
  nEdges: number;
  pairs: Uint32Array;
}

// ---- layer declarations (UI_ARCHITECTURE section 3) ---------------------------------------------------------------------------

export interface LayerDecl {
  id: string;
  field: string;
  kind: FieldKind;
  colormap: string;
  /** Colour range in SI units. Always resolved (schema range, else data min/max). */
  range: [number, number];
  opacity: number;
  palette?: 'biomes' | 'hash';
}

// ---- wire ---------------------------------------------------------------------------------------------------------------------

export interface Hello {
  wire_version: number;
  api_version: number;
  server: string;
}

// ---- Part 9g: versions, diff, explainer ---------------------------------------------------------------------------------------

export interface VersionInfo { version: number; label: string; quality: 'preview' | 'refined' | 'full'; generation: number; n_cells: number; edit_cells: number }
export interface VersionsData { mesh_id: string; current: number | null; max_stored: number; versions: VersionInfo[] }

/** `get_diff` answer. Arrays are per cell. `front` is the reveal time in [0, 1] of a changed cell, -1 for an unchanged cell. */
export interface DiffData {
  field: string;
  meshId: string;
  from: number;
  to: number;
  nCells: number;
  kind: string;
  noChange: boolean;
  /** 'edit' = distance from the edited cells, 'peak' = from the cell that changed most, 'none' = nothing changed. */
  frontKind: 'edit' | 'peak' | 'none';
  minRel: number;
  hiddenCells: number;
  changedCellsExact: number;
  summary: { changed_cells: number; peak_abs: number; mean_abs: number; changed_area: number; global_mean_shift: number; reach_km: number; edit_cells: number };
  meta: { unit: string; display: { scale: number; offset: number; unit: string }; colormap: string; range: [number, number] | null; palette?: 'biomes' | 'hash' };
  delta: Float32Array;
  before: Float32Array;
  after: Float32Array;
  front: Float32Array;
  distKm: Float32Array;
}

export interface DiffSummaryRow { field: string; kind: string; unit: string; display: { scale: number; offset: number; unit: string }; featured: boolean; layers: number; changed_cells: number; peak_abs: number; mean_abs: number; reach_km: number }
export interface DiffSummary {
  mesh_id: string; from_version: number; to_version: number; n_cells: number; no_change: boolean; min_rel: number; fields_changed: number; fields_changed_exact: number;
  changed_cells_any_field: number; edit_cells: number; reach_km: number; edit_fields: { field: string; cells: number }[]; fields: DiffSummaryRow[];
}

export interface ExplainStep {
  index: number; field: string; cell: number; old: number; new: number; delta: number; unit: string; display: { scale: number; offset: number; unit: string }; kind: string;
  module: string | null; hop: 'local' | 'global' | 'origin' | 'end'; dist_km: number; contributors: string[]; modules: string[]; anomalies: string[];
  also: { field: string; cell: number }[]; via_feedback: string[]; source_cells: number[];
}
export interface Explanation {
  mesh_id: string; from_version: number; to_version: number; field: string; cell: number; min_rel: number; changed: boolean; found_origin: boolean; has_global_hop: boolean;
  origin: string[]; stopped: string | null; steps: ExplainStep[]; text: string; note: string;
}
