// Part 9f: what the tools draw on the map, expressed as ordinary overlay items (kinds `mask` outline / fill), so every viewport that already
// draws overlays draws tools too. Pure functions: no DOM, no store. Colours come from the theme tokens --overlay-brush / -stroke / -lock.
import { brushDab, meshGeometry, type BrushSettings, type LocksData, type MeshData, type OutlineData } from 'wb-client';
import type { OverlayItem } from '../viewport/types';

/** Outline (inside, outside neighbour) pairs around a set of cells: the same data shape `get_outline` answers. */
export function outlineOfCells(mesh: MeshData, cells: ArrayLike<number>, tag = 'brush'): OutlineData {
  const inside = new Set<number>();
  for (let i = 0; i < cells.length; i++) inside.add(cells[i]);
  const pairs: number[] = [];
  for (const c of inside) {
    for (let k = mesh.adjOffsets[c]; k < mesh.adjOffsets[c + 1]; k++) {
      const nb = mesh.adj[k];
      if (!inside.has(nb)) pairs.push(c, nb);
    }
  }
  return { meshId: mesh.meshId, field: tag, threshold: 0.5, version: 0, stale: false, nEdges: pairs.length / 2, pairs: Uint32Array.from(pairs) };
}

/** Per-cell array with `weights` at `cells` and 0 elsewhere. */
export function cellValues(mesh: MeshData, cells: ArrayLike<number>, weights: ArrayLike<number> | number = 1): Float32Array {
  const v = new Float32Array(mesh.nCells);
  for (let i = 0; i < cells.length; i++) v[cells[i]] = typeof weights === 'number' ? weights : weights[i];
  return v;
}

export interface ToolOverlayInput {
  mesh: MeshData;
  /** Brush ring: where the pointer is, and the brush (null = no ring, e.g. pointer outside the map or tools off). */
  ring: { cell: number; brush: BrushSettings } | null;
  /** The uncommitted stroke. */
  stroke: { cells: Uint32Array; weights: Float32Array } | null;
  /** Locked cells to hatch (already narrowed to the field the UI wants to show). */
  locked: Uint32Array | null;
  resolveColor: (token: string) => string;
}

const base = (id: string, label: string, color: string, opacity: number, pattern?: 'solid' | 'hatch') => ({ id, label, color, opacity, blend: 'normal' as const, ...(pattern ? { pattern } : {}) });

/** Items bottom to top: lock hatch, stroke preview, brush ring. */
export function toolOverlayItems(i: ToolOverlayInput): OverlayItem[] {
  const items: OverlayItem[] = [];
  if (i.locked && i.locked.length) {
    items.push({ ...base('tool-lock', 'Locked cells', i.resolveColor('lock'), 0.85, 'hatch'), kind: 'mask', mode: 'fill', values: cellValues(i.mesh, i.locked), threshold: 0 });
  }
  if (i.stroke && i.stroke.cells.length) {
    items.push({ ...base('tool-stroke', 'Stroke', i.resolveColor('stroke'), 0.5), kind: 'mask', mode: 'fill', values: cellValues(i.mesh, i.stroke.cells, i.stroke.weights), threshold: 0 });
  }
  if (i.ring) {
    const dab = brushDab(i.mesh, meshGeometry(i.mesh), i.ring.cell, i.ring.brush.radiusKm * 1000, i.ring.brush.falloff);
    items.push({ ...base('tool-ring', 'Brush', i.resolveColor('brush'), 1), kind: 'mask', mode: 'outline', outline: outlineOfCells(i.mesh, dab.cells) });
  }
  return items;
}

/** Which locked cells to hatch: those of the output field the shown layer is (the lock data is keyed by field name), else every locked cell. */
export function lockedForLayer(locks: LocksData | null, layer: string | null): Uint32Array | null {
  if (!locks) return null;
  if (layer && Object.prototype.hasOwnProperty.call(locks.cells, layer)) return (locks.cells as Record<string, Uint32Array>)[layer];
  const all = new Set<number>();
  for (const arr of Object.values(locks.cells)) for (const c of arr) all.add(c);
  return Uint32Array.from(all);
}

/** Part 9g: the cells of a clicked explainer step, drawn as an ordinary fill mask (outline ring would hide single cells at low zoom). */
export function highlightItem(mesh: MeshData, cells: ArrayLike<number>, color: string): OverlayItem {
  return { ...base('explain-highlight', 'Explainer step', color, 0.7), kind: 'mask', mode: 'fill', values: cellValues(mesh, cells), threshold: 0 };
}
