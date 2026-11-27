// Part 9f: the tool framework. Framework-free; no renderer, no DOM. Tools see the world only through `pick`ed cell ids (UI_ARCHITECTURE section 4):
// a viewport turns a pointer position into a cell, the EditController feeds that cell to the active tool, the tool grows a local stroke and,
// on release, hands back ONE command (batching: a drag over thousands of pixels is one `paint_stroke`, one undo step).
import type { Command, EditSchema, MeshData, OutputField, PaintLayerId, PaintOp, ToolDef } from './types';

// ---- brush ---------------------------------------------------------------------------------------------------------------------

export type FalloffName = 'smooth' | 'linear' | 'constant' | 'sharp' | (string & {});

export interface BrushSettings {
  /** Brush radius in km on the surface (the UI draws it as a ring). */
  radiusKm: number;
  /** 0..1. Scales the weights of additive strokes and is the `strength` of set / smooth / flatten / erase. */
  strength: number;
  falloff: FalloffName;
  /** Amount at full weight for the tool (m for elevation, K for temperature, a fraction for rainfall); null = the tool's own default. */
  amount: number | null;
  /** Biome id for the biome tool. */
  biome: number | null;
  /** The output field the lock / unlock tools act on. */
  lockField: OutputField;
  /** The paint layer the eraser acts on. */
  eraseLayer: PaintLayerId;
}

export const defaultBrush = (edit?: EditSchema): BrushSettings => ({
  radiusKm: edit?.brush.radius_default_km ?? 800,
  strength: edit?.brush.strength_default ?? 0.6,
  falloff: edit?.brush.falloff_default ?? 'smooth',
  amount: null,
  biome: null,
  lockField: 'elevation',
  eraseLayer: 'elevation',
});

/** Weight of a dab at normalised distance `t` = distance / radius in [0, 1]. 1 at the centre, 0 at the rim (except `constant`). */
export function falloffWeight(kind: FalloffName, t: number): number {
  const x = Math.min(1, Math.max(0, t));
  switch (kind) {
    case 'constant': return 1;
    case 'linear': return 1 - x;
    case 'sharp': return (1 - x) * (1 - x);
    case 'smooth':
    default: return 1 - x * x * (3 - 2 * x); // smoothstep
  }
}

// ---- geometry on the mesh ------------------------------------------------------------------------------------------------------

export interface MeshGeometry {
  /** Surface distance in metres between two cells (great-circle on a sphere, straight line elsewhere). */
  dist(a: number, b: number): number;
}

/** Distances only need centroids; on a sphere the chord is converted to an arc using the radius implied by the centroids. */
export function meshGeometry(mesh: MeshData): MeshGeometry {
  const c = mesh.centroid;
  const sphere = mesh.kind === 'sphere';
  let R = 0;
  if (sphere && mesh.nCells > 0) R = Math.hypot(c[0], c[1], c[2]);
  return {
    dist(a, b) {
      const dx = c[3 * a] - c[3 * b], dy = c[3 * a + 1] - c[3 * b + 1], dz = c[3 * a + 2] - c[3 * b + 2];
      const chord = Math.hypot(dx, dy, dz);
      return sphere && R > 0 ? 2 * R * Math.asin(Math.min(1, chord / (2 * R))) : chord;
    },
  };
}

export interface Dab {
  cells: number[];
  weights: number[];
}

/** All cells within `radiusM` of `centre`, found by walking the adjacency (works on any mesh), with falloff weights. The centre cell always counts. */
export function brushDab(mesh: MeshData, geom: MeshGeometry, centre: number, radiusM: number, falloff: FalloffName): Dab {
  const seen = new Set<number>([centre]);
  const queue = [centre];
  const cells: number[] = [];
  const weights: number[] = [];
  for (let q = 0; q < queue.length; q++) {
    const c = queue[q];
    const w = falloffWeight(falloff, radiusM > 0 ? geom.dist(centre, c) / radiusM : 0);
    if (w > 0) {
      cells.push(c);
      weights.push(w);
    }
    for (let k = mesh.adjOffsets[c]; k < mesh.adjOffsets[c + 1]; k++) {
      const nb = mesh.adj[k];
      if (!seen.has(nb) && geom.dist(centre, nb) <= radiusM) {
        seen.add(nb);
        queue.push(nb);
      }
    }
  }
  return { cells, weights };
}

/** Greedy walk over the adjacency from `a` towards `b`: the cells a fast drag passed over (excluding `a`, including `b`). Capped, so it always ends. */
export function cellPath(mesh: MeshData, geom: MeshGeometry, a: number, b: number, maxSteps = 512): number[] {
  const path: number[] = [];
  let cur = a;
  for (let i = 0; i < maxSteps && cur !== b; i++) {
    let best = cur;
    let bd = geom.dist(cur, b);
    for (let k = mesh.adjOffsets[cur]; k < mesh.adjOffsets[cur + 1]; k++) {
      const nb = mesh.adj[k];
      const d = geom.dist(nb, b);
      if (d < bd) { bd = d; best = nb; }
    }
    if (best === cur) break; // local minimum: stop, the caller dabs `b` itself
    cur = best;
    path.push(cur);
  }
  if (path[path.length - 1] !== b) path.push(b);
  return path;
}

// ---- stroke accumulation -------------------------------------------------------------------------------------------------------

/** The cells and weights of one stroke so far. A cell touched by several dabs keeps its largest weight (the same rule the core applies). */
export class StrokeBuffer {
  private w = new Map<number, number>();
  private lastCentre: number | null = null;
  dabs = 0;

  get last(): number | null { return this.lastCentre; }
  get size(): number { return this.w.size; }
  clear(): void { this.w.clear(); this.lastCentre = null; this.dabs = 0; }

  add(dab: Dab, centre: number): void {
    for (let i = 0; i < dab.cells.length; i++) {
      const c = dab.cells[i];
      const prev = this.w.get(c);
      if (prev === undefined || dab.weights[i] > prev) this.w.set(c, dab.weights[i]);
    }
    this.lastCentre = centre;
    this.dabs++;
  }

  /** Ascending cell ids with their weights (deterministic order). */
  toArrays(): { cells: Uint32Array; weights: Float32Array } {
    const cells = Uint32Array.from(this.w.keys()).sort();
    return { cells, weights: Float32Array.from(cells, (c) => this.w.get(c) as number) };
  }
}

// ---- tool interface and registry -----------------------------------------------------------------------------------------------

export interface ToolContext {
  mesh: MeshData;
  geom: MeshGeometry;
  brush: BrushSettings;
  stroke: StrokeBuffer;
  /** Called whenever the local stroke changed (the UI redraws its preview from `stroke`). */
  changed(): void;
}

export interface Tool {
  id: string;
  label: string;
  /** CSS cursor name; the UI maps it to its own cursor art if it wants. */
  cursor: string;
  def: ToolDef | null;
  /** Pointer down on `cell`. Starts the stroke. */
  onPick(ctx: ToolContext, cell: number): void;
  /** Pointer moved (button down) over `cell`. */
  onDrag(ctx: ToolContext, cell: number): void;
  /** Pointer released. Returns the ONE command that commits the stroke, or null when the stroke was empty. */
  onRelease(ctx: ToolContext): Command | null;
}

/** Spacing between dabs, as a fraction of the radius: keeps cost bounded however fast the pointer moves. */
export const DAB_SPACING = 0.25;

function dabAt(ctx: ToolContext, cell: number): void {
  const radiusM = ctx.brush.radiusKm * 1000;
  ctx.stroke.add(brushDab(ctx.mesh, ctx.geom, cell, radiusM, ctx.brush.falloff), cell);
}

/** Shared drag behaviour: fill the gap since the last dab with spaced dabs along the mesh. */
function dragTo(ctx: ToolContext, cell: number): void {
  const last = ctx.stroke.last;
  if (last === null) return dabAt(ctx, cell), ctx.changed();
  if (last === cell) return;
  const spacing = ctx.brush.radiusKm * 1000 * DAB_SPACING;
  let from = last;
  for (const c of cellPath(ctx.mesh, ctx.geom, last, cell)) {
    const d = ctx.geom.dist(from, c);
    if (d > 0 && (d >= spacing || c === cell)) {
      dabAt(ctx, c);
      from = c;
    }
  }
  ctx.changed();
}

function pickAt(ctx: ToolContext, cell: number): void {
  ctx.stroke.clear();
  dabAt(ctx, cell);
  ctx.changed();
}

/** A paint tool described entirely by its `ToolDef` (the schema says what it does; no per-tool code). */
export function paintTool(def: ToolDef): Tool {
  return {
    id: def.id, label: def.label, cursor: 'crosshair', def,
    onPick: pickAt,
    onDrag: dragTo,
    onRelease(ctx) {
      if (ctx.stroke.size === 0) return null;
      const { cells, weights } = ctx.stroke.toArrays();
      const strength = ctx.brush.strength;
      const layer = (def.kind === 'erase' ? ctx.brush.eraseLayer : def.layer) as PaintLayerId;
      const op = def.op as PaintOp;
      let value: number | null = null;
      let w: number[] = Array.from(weights);
      if (op === 'add') {
        // additive strokes: brush strength scales the weights, the amount (with the tool's sign) is the change at weight 1
        value = (def.sign ?? 1) * (ctx.brush.amount ?? def.amount?.default ?? 1);
        w = w.map((x) => x * strength);
      } else if (op === 'set' && def.needs_value) {
        if (ctx.brush.biome == null) return null;
        value = ctx.brush.biome;
      }
      return { type: 'paint_stroke', layer, op, cells: Array.from(cells), weights: w, value, strength: op === 'add' ? 1 : strength };
    },
  };
}

/** Lock / unlock tool: every cell the brush touched (weight > 0) is (un)locked on `brush.lockField`. */
export function lockTool(def: ToolDef): Tool {
  return {
    id: def.id, label: def.label, cursor: 'cell', def,
    onPick: pickAt,
    onDrag: dragTo,
    onRelease(ctx) {
      if (ctx.stroke.size === 0) return null;
      return { type: 'set_lock', field: ctx.brush.lockField, cells: Array.from(ctx.stroke.toArrays().cells), locked: def.locked !== false };
    },
  };
}

export class ToolRegistry {
  private tools = new Map<string, Tool>();

  register(tool: Tool): this {
    this.tools.set(tool.id, tool);
    return this;
  }

  get(id: string): Tool | undefined { return this.tools.get(id); }
  list(): Tool[] { return [...this.tools.values()]; }
  has(id: string): boolean { return this.tools.has(id); }

  /** Builds the registry from `schema.edit.tools`, so a tool added in Rust appears in the UI with no client code. */
  static fromSchema(edit: EditSchema): ToolRegistry {
    const r = new ToolRegistry();
    for (const d of edit.tools) r.register(d.kind === 'lock' ? lockTool(d) : paintTool(d));
    return r;
  }
}
