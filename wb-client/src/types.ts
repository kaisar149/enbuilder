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
  | { type: 'variations'; count: number; coarse?: boolean };

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
  | { type: 'get_variation'; index: number; name: string };

export interface Ack {
  api_version: number;
  command: string;
  /** Solved fields that are now stale. */
  invalidated: string[];
  /** Set by `solve`. */
  job: number | null;
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
  job: { id: number; running: boolean } | null;
  can_undo: boolean;
  can_redo: boolean;
  /** Part 9c. `undo` is oldest first; `redo` lists the next step to re-apply first. */
  history?: { undo: HistoryEntry[]; redo: HistoryEntry[] };
}

export interface HistoryEntry {
  label: string;
  preset: string;
  changes: { name: string; old: number; new: number }[];
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
