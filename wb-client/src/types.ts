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
  | { type: 'redo' };

export type Query =
  | { type: 'get_schema' }
  | { type: 'get_world_info' }
  | { type: 'get_mesh' }
  | { type: 'get_field'; name: string; layer?: number }
  | { type: 'get_cell'; cell: number };

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
}

export interface PresetDef {
  name: string;
  description: string;
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
