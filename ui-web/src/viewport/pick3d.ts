// Ray casting against the world (pure). No triangle test: the world is a height field over a simple base surface, so a ray is intersected with
// the base shape (sphere, cube, plane) and, when relief is on, marched through the relief shell above it. The cell under the hit point is found
// with the same greedy adjacency walk as the 2D map. Cost per pick: a handful of ray steps plus short graph walks, independent of mesh size.
import type { MeshData } from 'wb-client';
import { heightAbove, toSurface, type CellGeometry } from './geometry3d';
import { locateCell, bruteCell } from './projections';

type V3 = [number, number, number];
const SHELL_STEPS = 96;

function raySphere(o: V3, d: V3, r: number): number | null {
  const b = o[0] * d[0] + o[1] * d[1] + o[2] * d[2];
  const c = o[0] * o[0] + o[1] * o[1] + o[2] * o[2] - r * r;
  const disc = b * b - c;
  if (disc < 0) return null;
  const t = -b - Math.sqrt(disc);
  return t >= 0 ? t : null;
}

function rayBox(o: V3, d: V3, h: number): number | null {
  let t0 = 0, t1 = Infinity;
  for (let k = 0; k < 3; k++) {
    if (Math.abs(d[k]) < 1e-12) { if (Math.abs(o[k]) > h) return null; continue; }
    let a = (-h - o[k]) / d[k], b = (h - o[k]) / d[k];
    if (a > b) [a, b] = [b, a];
    t0 = Math.max(t0, a); t1 = Math.min(t1, b);
    if (t0 > t1) return null;
  }
  return t0;
}

/** Distance along the ray to the base shape inflated by the lift distance (the undisplaced surface when it is 0), or null. */
function hitBase(g: CellGeometry, o: V3, d: V3, lift: number): number | null {
  if (g.kind === 'sphere') return raySphere(o, d, g.radius + lift);
  if (g.kind === 'cube') return rayBox(o, d, g.radius + lift);
  if (Math.abs(d[2]) < 1e-12) return null;
  const t = (g.z0 + lift - o[2]) / d[2];
  return t >= 0 ? t : null;
}

export interface PickHit { cell: number; point: V3; t: number }

/**
 * @param relief per-cell relief heights (mesh units, >= 0) or null
 * @param hint a cell near the last hit, to start the graph walk from (optional)
 */
export function pickRay(mesh: MeshData, g: CellGeometry, relief: Float32Array | null, reliefMax: number, o: V3, d: V3, hint = -1): PickHit | null {
  const at = (t: number): V3 => [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
  let last = hint;
  const cellAt = (p: V3): number => {
    const s = toSurface(g, p);
    if (last < 0) last = bruteCell(mesh, s);
    last = locateCell(mesh, s, last);
    return last;
  };
  const inside = (p: V3, cell: number): boolean => {
    // a point on the base surface is on the world only when the nearest cell centre is close (plane/disc edges, off-map hits)
    const s = toSurface(g, p);
    const dx = mesh.centroid[3 * cell] - s[0], dy = mesh.centroid[3 * cell + 1] - s[1], dz = mesh.centroid[3 * cell + 2] - s[2];
    return Math.hypot(dx, dy, dz) < 1.2 * g.spacing;
  };
  const tBase = hitBase(g, o, d, 0);
  if (!relief || reliefMax <= 0) {
    if (tBase === null) return null;
    const p = at(tBase);
    const cell = cellAt(p);
    return inside(p, cell) ? { cell, point: p, t: tBase } : null;
  }
  const tTop = hitBase(g, o, d, reliefMax);
  if (tTop === null) return null;
  // The ray enters the shell at tTop and reaches the base surface at tBase (or leaves the shell sideways, e.g. grazing the limb).
  const tEnd = tBase ?? tTop + (g.kind === 'plane' ? 0 : 2 * reliefMax + 2 * g.spacing);
  const below = (t: number): number => {
    const p = at(t);
    const c = cellAt(p);
    return heightAbove(g, p) <= relief[c] + 1e-12 && inside(p, c) ? c : -1;
  };
  const step = (tEnd - tTop) / SHELL_STEPS;
  if (!(step > 0)) {
    if (tBase === null) return null;
    const p = at(tBase), c = cellAt(p);
    return inside(p, c) ? { cell: c, point: p, t: tBase } : null;
  }
  let prev = tTop;
  for (let k = 0; k <= SHELL_STEPS; k++) {
    const t = tTop + k * step;
    const c = below(t);
    if (c >= 0) {
      // refine between the last free sample and this one
      let lo = prev, hi = t, hit = c;
      for (let it = 0; it < 14; it++) {
        const mid = (lo + hi) / 2;
        const cm = below(mid);
        if (cm >= 0) { hi = mid; hit = cm; } else lo = mid;
      }
      return { cell: hit, point: at(hi), t: hi };
    }
    prev = t;
  }
  return null;
}
