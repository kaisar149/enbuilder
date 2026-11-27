// Part 9e: renderer-neutral overlay geometry, in MESH space. Pure TypeScript (no DOM, no Three.js), so it runs under Node and every viewport
// (2D map, globe, cube) can use it. The core answers are topology (cell ids); this turns them into points and directions.
// Nothing here names a field: it works on vectors (3 component arrays), outlines (cell pairs) and polylines (cell ids).
import type { MeshData, OutlineData, RiversData } from 'wb-client';
import { shapeKind, surfaceNormal, type ShapeKind } from '../viewport/geometry3d';

export type V3 = [number, number, number];
const cen = (m: MeshData, i: number): V3 => [m.centroid[3 * i], m.centroid[3 * i + 1], m.centroid[3 * i + 2]];

/** Typical distance between neighbouring cells (mean over a sample of the adjacency). */
export function cellSpacing(mesh: MeshData): number {
  let s = 0, k = 0;
  const step = Math.max(1, Math.floor(mesh.nCells / 400));
  for (let i = 0; i < mesh.nCells; i += step) {
    const a = cen(mesh, i);
    for (let q = mesh.adjOffsets[i]; q < mesh.adjOffsets[i + 1]; q++) {
      const b = cen(mesh, mesh.adj[q]);
      s += Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
      k++;
    }
  }
  return k ? s / k : 1;
}

// ---- thinning -----------------------------------------------------------------------------------------------------------------
const thinCache = new WeakMap<MeshData, Map<number, Uint32Array>>();

/**
 * Greedy thinning over the adjacency graph: walk the cells in order, keep a cell unless it is within `rings` hops of a kept one. Kept cells are
 * at least `rings + 1` hops apart, evenly spread, deterministic, and the same set is returned for the same mesh and `rings` (cached).
 * `rings` 0 keeps everything.
 */
export function thinCells(mesh: MeshData, rings: number): Uint32Array {
  const r = Math.max(0, Math.floor(rings));
  let per = thinCache.get(mesh);
  if (!per) { per = new Map(); thinCache.set(mesh, per); }
  const hit = per.get(r);
  if (hit) return hit;
  const n = mesh.nCells;
  const kept: number[] = [];
  if (r === 0) { for (let i = 0; i < n; i++) kept.push(i); }
  else {
    const blocked = new Uint8Array(n);
    const seen = new Int32Array(n).fill(-1);
    let frontier: number[] = [];
    for (let i = 0; i < n; i++) {
      if (blocked[i]) continue;
      kept.push(i);
      // block everything within r hops
      frontier = [i];
      seen[i] = i;
      for (let d = 0; d < r; d++) {
        const next: number[] = [];
        for (const c of frontier) {
          for (let q = mesh.adjOffsets[c]; q < mesh.adjOffsets[c + 1]; q++) {
            const nb = mesh.adj[q];
            if (seen[nb] !== i) { seen[nb] = i; blocked[nb] = 1; next.push(nb); }
          }
        }
        frontier = next;
      }
    }
  }
  const out = Uint32Array.from(kept);
  per.set(r, out);
  return out;
}

/** Smallest ring count whose thinned set has about `target` cells or fewer (estimate for a hexagonal mesh: 1 + 3 r (r+1) cells per kept cell). */
export function ringsForCount(nCells: number, target: number, maxRings = 14): number {
  for (let r = 0; r <= maxRings; r++) if (nCells / (1 + 3 * r * (r + 1)) <= target) return r;
  return maxRings;
}

/** Rings so that kept arrows are about `minSpacingPx` apart on screen, given the on-screen size of the map in pixels. */
export function ringsForPixels(nCells: number, mapPxW: number, mapPxH: number, minSpacingPx: number, maxRings = 14): number {
  const cellPx = Math.sqrt((mapPxW * mapPxH) / Math.max(1, nCells));
  return Math.max(0, Math.min(maxRings, Math.ceil(minSpacingPx / Math.max(cellPx, 1e-6)) - 1));
}

// ---- vectors ------------------------------------------------------------------------------------------------------------------
export type Components = [Float32Array, Float32Array, Float32Array];

export interface VectorStats {
  /** Largest magnitude, and the 95th percentile (arrow length is scaled to it so one gale does not shrink every other arrow). */
  max: number;
  p95: number;
}

export function vectorStats(c: Components): VectorStats {
  const n = c[0].length;
  const step = Math.max(1, Math.floor(n / 4000));
  const mags: number[] = [];
  let max = 0;
  for (let i = 0; i < n; i += step) {
    const m = Math.hypot(c[0][i], c[1][i], c[2][i]);
    if (Number.isFinite(m)) { mags.push(m); if (m > max) max = m; }
  }
  mags.sort((a, b) => a - b);
  return { max, p95: mags.length ? mags[Math.min(mags.length - 1, Math.floor(0.95 * mags.length))] : 0 };
}

export interface Glyph {
  cell: number;
  /** Unit direction in the tangent plane (mesh space); the normal part of the vector is removed. */
  dir: V3;
  /** Magnitude of the tangent part. */
  mag: number;
}

/** The vector of `cell` along the surface. Returns null when the value is missing or zero. */
export function tangentVector(kind: ShapeKind, centroid: V3, v: V3): { dir: V3; mag: number } | null {
  if (!Number.isFinite(v[0] + v[1] + v[2])) return null;
  const n = surfaceNormal(kind, centroid);
  const d = v[0] * n[0] + v[1] * n[1] + v[2] * n[2];
  const t: V3 = [v[0] - d * n[0], v[1] - d * n[1], v[2] - d * n[2]];
  const m = Math.hypot(t[0], t[1], t[2]);
  if (m < 1e-12) return null;
  return { dir: [t[0] / m, t[1] / m, t[2] / m], mag: m };
}

export function glyphsFor(mesh: MeshData, comps: Components, cells: ArrayLike<number>): Glyph[] {
  const kind = shapeKind(mesh);
  const out: Glyph[] = [];
  for (let k = 0; k < cells.length; k++) {
    const c = cells[k];
    const t = tangentVector(kind, cen(mesh, c), [comps[0][c], comps[1][c], comps[2][c]]);
    if (t) out.push({ cell: c, dir: t.dir, mag: t.mag });
  }
  return out;
}

/** Arrow length in pixels: proportional to the magnitude relative to the 95th percentile, between `minPx` (still visible) and `maxPx`. */
export function arrowLength(mag: number, ref: number, minPx: number, maxPx: number): number {
  if (!(ref > 0)) return minPx;
  return minPx + (maxPx - minPx) * Math.min(1, mag / ref);
}

// ---- outlines and polylines ----------------------------------------------------------------------------------------------------
/**
 * Outline edges as short segments in mesh space (6 floats each: x0 y0 z0 x1 y1 z1). The shared edge of cells a (inside) and b (outside) is perpendicular
 * to a->b through the midpoint, with the length of a regular polygon edge: D * tan(pi / degree). Needs only centroids and adjacency.
 */
export function outlineSegments(mesh: MeshData, outline: OutlineData): Float32Array {
  const kind = shapeKind(mesh);
  const out = new Float32Array(outline.nEdges * 6);
  for (let e = 0; e < outline.nEdges; e++) {
    const a = outline.pairs[2 * e], b = outline.pairs[2 * e + 1];
    const pa = cen(mesh, a), pb = cen(mesh, b);
    const d: V3 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
    const D = Math.hypot(d[0], d[1], d[2]) || 1;
    const m: V3 = [(pa[0] + pb[0]) / 2, (pa[1] + pb[1]) / 2, (pa[2] + pb[2]) / 2];
    const n = surfaceNormal(kind, kind === 'cube' ? pa : m);
    // t = n x d, normalised
    let t: V3 = [n[1] * d[2] - n[2] * d[1], n[2] * d[0] - n[0] * d[2], n[0] * d[1] - n[1] * d[0]];
    const tl = Math.hypot(t[0], t[1], t[2]) || 1;
    t = [t[0] / tl, t[1] / tl, t[2] / tl];
    const deg = Math.max(3, mesh.adjOffsets[a + 1] - mesh.adjOffsets[a]);
    const h = (D * Math.tan(Math.PI / deg)) / 2;
    out.set([m[0] - t[0] * h, m[1] - t[1] * h, m[2] - t[2] * h, m[0] + t[0] * h, m[1] + t[1] * h, m[2] + t[2] * h], 6 * e);
  }
  return out;
}

export interface Polylines {
  /** x y z per vertex (cell centroids). */
  points: Float32Array;
  offsets: Uint32Array;
  /** Relative width per vertex, 0..1 (log scale of discharge against the largest river). */
  width: Float32Array;
}

export function riverPolylines(mesh: MeshData, r: RiversData): Polylines {
  const points = new Float32Array(r.nPoints * 3);
  const width = new Float32Array(r.nPoints);
  let qmax = 0;
  for (let i = 0; i < r.nPoints; i++) qmax = Math.max(qmax, r.discharge[i]);
  const lmax = Math.log1p(qmax) || 1;
  for (let i = 0; i < r.nPoints; i++) {
    const c = r.cells[i];
    points[3 * i] = mesh.centroid[3 * c]; points[3 * i + 1] = mesh.centroid[3 * c + 1]; points[3 * i + 2] = mesh.centroid[3 * c + 2];
    width[i] = Math.log1p(Math.max(0, r.discharge[i])) / lmax;
  }
  return { points, offsets: r.offsets, width };
}

/** Pixel width of a river vertex: `minPx` for a trickle up to `maxPx` for the largest river. */
export const riverWidthPx = (rel: number, minPx: number, maxPx: number): number => minPx + (maxPx - minPx) * Math.max(0, Math.min(1, rel));

/** Indices (3 per triangle) of the triangles of the cells whose value is above `threshold`, for a fill overlay such as lakes. */
export function maskTriangles(triCell: Uint32Array, indices: Uint32Array, values: Float32Array, threshold: number): Uint32Array {
  const out: number[] = [];
  for (let t = 0; t < triCell.length; t++) {
    const v = values[triCell[t]];
    if (Number.isFinite(v) && v > threshold) out.push(indices[3 * t], indices[3 * t + 1], indices[3 * t + 2]);
  }
  return Uint32Array.from(out);
}
