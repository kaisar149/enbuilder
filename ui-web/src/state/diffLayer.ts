// Part 9g: the map layer of the diff view. A diff is drawn through the ordinary Viewport interface (`setLayers`): a scalar layer whose values are
// the change revealed so far, with the diverging colormap centred on zero. Nothing here knows a field name or a renderer.
import { frameDelta, compareValues, layerDecl, metaOf, symmetricRange, sweepCoord, type CompareMode, type DiffData, type FieldData, type FieldKind, type FieldMeta, type LayerDecl, type MeshData, type Schema } from 'wb-client';

export interface DiffUiState {
  /** Diff view on: the map shows the change between the two chosen versions. */
  on: boolean;
  /** Playhead of the propagation animation in [0, 1]. UI-only; it lives here, not in a viewport, so a 2D/3D switch keeps it. */
  t: number;
  playing: boolean;
  mode: CompareMode;
  /** Swipe handle in [0, 1]. */
  swipe: number;
  /** Colour range slider: fraction of the largest change that maps to full colour. */
  rangeFrac: number;
  /** Field the user chose to diff (null = follow the map layer when it changed, else the first changed field). */
  field: string | null;
  /** Hide changes below 1 % of the largest change of a field (numerical noise). */
  hideSmall: boolean;
  /** Cells highlighted by a clicked explainer step. */
  highlight: number[];
}

export const initialDiffUi = (): DiffUiState => ({ on: false, t: 1, playing: false, mode: 'diff', swipe: 0.5, rangeFrac: 1, field: null, hideSmall: true, highlight: [] });

const sweeps = new WeakMap<MeshData, Float32Array>();
const sweepOf = (m: MeshData) => { let s = sweeps.get(m); if (!s) { s = sweepCoord(m); sweeps.set(m, s); } return s; };

/** Layer declaration plus synthetic field data for the diff, or null when the diff is not on / not loaded. */
export function diffLayer(schema: Schema, mesh: MeshData | null, d: DiffData | null, u: DiffUiState): { layers: LayerDecl[]; fields: Record<string, FieldData> } | null {
  if (!u.on || !d || !mesh || d.nCells !== mesh.nCells) return null;
  // fields outside the curated schema list still diff: the answer carries the metadata
  const meta: FieldMeta = metaOf(schema, d.field) ?? { name: d.field, kind: d.kind as FieldKind, unit: d.meta.unit, display: d.meta.display, colormap: d.meta.colormap, range: d.meta.range, description: '', featured: false, ...(d.meta.palette ? { palette: d.meta.palette } : {}) };
  const category = meta.kind === 'category';
  if (u.mode === 'diff') {
    const delta = frameDelta(d, u.t);
    const values = category ? delta.map((v) => (v !== 0 ? 1 : 0)) : delta;
    const range: [number, number] = category ? [-1, 1] : symmetricRange(d, u.rangeFrac);
    const id = `diff:${d.field}`;
    const decl: LayerDecl = { id, field: d.field, kind: 'scalar', colormap: 'diverging', range, opacity: 1 };
    const field: FieldData = { name: d.field, layer: 0, layers: 1, meshId: d.meshId, version: d.to, stale: false, min: range[0], max: range[1], meta: { ...meta, kind: 'scalar', colormap: 'diverging' }, values };
    return { layers: [decl], fields: { [id]: field } };
  }
  const values = compareValues(d, u.mode, sweepOf(mesh), u.swipe);
  let lo = Infinity, hi = -Infinity;
  for (let i = 0; i < d.nCells; i++) { const a = d.before[i], b = d.after[i]; lo = Math.min(lo, a, b); hi = Math.max(hi, a, b); }
  const field: FieldData = { name: d.field, layer: 0, layers: 1, meshId: d.meshId, version: d.to, stale: false, min: lo, max: hi, meta, values };
  const decl = layerDecl(schema, field, 1);
  return { layers: [{ ...decl, id: `diffcmp:${decl.id}` }], fields: { [`diffcmp:${decl.id}`]: field } };
}
