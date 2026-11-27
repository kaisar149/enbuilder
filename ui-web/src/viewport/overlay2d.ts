// Part 9e: overlay geometry in MAP space (u, v in 0..1) for the 2D viewport. Pure functions over the neutral mesh-space geometry of
// overlays/geometry.ts and the forward projection of projections.ts. No DOM, so they run under Node and are covered by unit tests.
import type { MeshData, OutlineData, RiversData } from 'wb-client';
import { cellSpacing, glyphsFor, outlineSegments, riverPolylines, type Components, type V3 } from '../overlays/geometry';
import type { Forward } from './projections';

/** Segments (u0 v0 u1 v1 per segment). Segments that jump across the map (date line, cube fold) are dropped. */
export function segments2D(mesh: MeshData, fwd: Forward, outline: OutlineData, maxJump = 0.2): Float32Array {
  const seg = outlineSegments(mesh, outline);
  const out: number[] = [];
  for (let e = 0; e < outline.nEdges; e++) {
    const a = fwd([seg[6 * e], seg[6 * e + 1], seg[6 * e + 2]]);
    if (!a) continue;
    const b = fwd([seg[6 * e + 3], seg[6 * e + 4], seg[6 * e + 5]], a);
    if (!b || Math.abs(a[0] - b[0]) > maxJump || Math.abs(a[1] - b[1]) > maxJump) continue;
    out.push(a[0], a[1], b[0], b[1]);
  }
  return Float32Array.from(out);
}

export interface Lines2D {
  /** u v per vertex. */
  points: Float32Array;
  /** Start of each polyline in vertex units, plus the end (n + 1 entries). Polylines are split where the map jumps. */
  offsets: Uint32Array;
  /** Relative width 0..1 per vertex. */
  width: Float32Array;
}

export function lines2D(mesh: MeshData, fwd: Forward, rivers: RiversData, maxJump = 0.2): Lines2D {
  const poly = riverPolylines(mesh, rivers);
  const pts: number[] = [];
  const wid: number[] = [];
  const offsets: number[] = [0];
  for (let k = 0; k + 1 < poly.offsets.length; k++) {
    let prev: [number, number] | null = null;
    let runStart = pts.length / 2;
    for (let i = poly.offsets[k]; i < poly.offsets[k + 1]; i++) {
      const p = fwd([poly.points[3 * i], poly.points[3 * i + 1], poly.points[3 * i + 2]], prev ?? undefined);
      if (!p) { prev = null; continue; }
      if (prev && (Math.abs(p[0] - prev[0]) > maxJump || Math.abs(p[1] - prev[1]) > maxJump)) {
        if (pts.length / 2 - runStart >= 2) offsets.push(pts.length / 2); else { pts.length = runStart * 2; wid.length = runStart; }
        runStart = pts.length / 2;
        prev = null;
        const q = fwd([poly.points[3 * i], poly.points[3 * i + 1], poly.points[3 * i + 2]]);
        if (!q) continue;
        pts.push(q[0], q[1]); wid.push(poly.width[i]); prev = q;
        continue;
      }
      pts.push(p[0], p[1]); wid.push(poly.width[i]); prev = p;
    }
    if (pts.length / 2 - runStart >= 2) offsets.push(pts.length / 2); else { pts.length = runStart * 2; wid.length = runStart; }
  }
  return { points: Float32Array.from(pts), offsets: Uint32Array.from(offsets), width: Float32Array.from(wid) };
}

export interface Arrows2D {
  cell: Uint32Array;
  /** Map position of the origin. */
  u: Float32Array;
  v: Float32Array;
  /** Unit screen direction (x right, y down) of the vector on THIS map. */
  dx: Float32Array;
  dy: Float32Array;
  mag: Float32Array;
}

/**
 * Arrows for `cells`: the direction is found by projecting the cell and a point a little way along the vector, so it is right on every
 * projection (cube cross, equirectangular stretching near the poles) without knowing which one is in use. `aspect` = map width / height.
 */
export function arrows2D(mesh: MeshData, fwd: Forward, aspect: number, comps: Components, cells: ArrayLike<number>): Arrows2D {
  const g = glyphsFor(mesh, comps, cells);
  const eps = cellSpacing(mesh) * 0.3;
  const u: number[] = [], v: number[] = [], dx: number[] = [], dy: number[] = [], mag: number[] = [], cell: number[] = [];
  for (const { cell: c, dir, mag: m } of g) {
    const o: V3 = [mesh.centroid[3 * c], mesh.centroid[3 * c + 1], mesh.centroid[3 * c + 2]];
    const a = fwd(o);
    if (!a) continue;
    const b = fwd([o[0] + dir[0] * eps, o[1] + dir[1] * eps, o[2] + dir[2] * eps], a);
    if (!b) continue;
    // to screen space: u spans `aspect` units of height
    const sx = (b[0] - a[0]) * aspect, sy = b[1] - a[1];
    const l = Math.hypot(sx, sy);
    if (l < 1e-12) continue;
    cell.push(c); u.push(a[0]); v.push(a[1]); dx.push(sx / l); dy.push(sy / l); mag.push(m);
  }
  return { cell: Uint32Array.from(cell), u: Float32Array.from(u), v: Float32Array.from(v), dx: Float32Array.from(dx), dy: Float32Array.from(dy), mag: Float32Array.from(mag) };
}

/** Screen-space flow of EVERY cell (2 floats: unit direction times magnitude relative to `ref`), for the animated streaks. Zero where undefined. */
export function flow2D(mesh: MeshData, fwd: Forward, aspect: number, comps: Components, ref: number): Float32Array {
  const all = new Uint32Array(mesh.nCells);
  for (let i = 0; i < all.length; i++) all[i] = i;
  const a = arrows2D(mesh, fwd, aspect, comps, all);
  const out = new Float32Array(2 * mesh.nCells);
  for (let k = 0; k < a.cell.length; k++) {
    const r = ref > 0 ? Math.min(1.5, a.mag[k] / ref) : 0;
    out[2 * a.cell[k]] = a.dx[k] * r;
    out[2 * a.cell[k] + 1] = a.dy[k] * r;
  }
  return out;
}
