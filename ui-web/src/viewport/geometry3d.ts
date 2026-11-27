// Mesh -> triangles for the 3D viewports. Pure TypeScript (no Three.js, no DOM), so it runs in Node tests and any renderer can use it.
//
// Every cell becomes a flat-coloured polygon (so a cell has exactly the colour it has in the 2D map). The polygon corners are the circumcentres of
// (cell, neighbour k, neighbour k+1) taken in angular order around the cell: for a hex/pent sphere that is the Voronoi hexagon, for the square
// grids of the cube and the plane it is the square. Missing neighbours (plane edge) are mirrored through the cell centre. A cube fold is
// "unfolded" into the face plane of the cell so cells on both sides of an edge meet exactly on the edge line. Nothing here reads the field names.
//
// Relief: a corner is lifted by the mean elevation of the cells that meet there (found as the cells equidistant from the corner), so neighbouring
// polygons stay joined (no cracks) while their colours stay flat.
import type { MeshData } from 'wb-client';

export type ShapeKind = 'sphere' | 'cube' | 'plane';

export function shapeKind(mesh: MeshData): ShapeKind {
  return mesh.kind === 'cube' ? 'cube' : mesh.kind === 'plane' ? 'plane' : 'sphere';
}

export interface CellGeometry {
  kind: ShapeKind;
  nCells: number;
  /** Bounding radius R: sphere radius, cube half side, plane half of the longer side. Camera lengths are multiples of it. */
  radius: number;
  /** Plane only: z of the surface and the half extents. Sphere/cube: z0 = 0. */
  z0: number;
  half: [number, number];
  /** Typical centre distance between neighbours. */
  spacing: number;
  /** Corners of cell i are cornerStart[i] .. cornerStart[i+1]-1, counter-clockwise seen from outside. */
  cornerStart: Uint32Array;
  /** 3 floats per corner, on the undisplaced surface. */
  base: Float32Array;
  /** 4 cell ids per corner (own cell first, -1 padded): the cells that meet at the corner. */
  incident: Int32Array;
  /** Triangle fan per cell (3 indices per triangle). */
  indices: Uint32Array;
  /** Cell of every triangle (for software rendering and tests). */
  triCell: Uint32Array;
  nCorners: number;
}

type V3 = [number, number, number];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a: V3, s: number): V3 => [a[0] * s, a[1] * s, a[2] * s];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const len = (a: V3) => Math.hypot(a[0], a[1], a[2]);
const norm = (a: V3): V3 => { const l = len(a) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

/** Outward surface normal used to order a cell's neighbours (sphere: radial, cube: the face axis, plane: +z). */
export function surfaceNormal(kind: ShapeKind, c: V3): V3 {
  if (kind === 'plane') return [0, 0, 1];
  if (kind === 'cube') {
    const ax = Math.abs(c[0]), ay = Math.abs(c[1]), az = Math.abs(c[2]);
    if (ax >= ay && ax >= az) return [Math.sign(c[0]) || 1, 0, 0];
    return ay >= az ? [0, Math.sign(c[1]) || 1, 0] : [0, 0, Math.sign(c[2]) || 1];
  }
  return norm(c);
}

/** Height of point `p` above the undisplaced surface (same units as the mesh). */
export function heightAbove(g: Pick<CellGeometry, 'kind' | 'radius' | 'z0'>, p: V3): number {
  if (g.kind === 'sphere') return len(p) - g.radius;
  if (g.kind === 'cube') return Math.max(Math.abs(p[0]), Math.abs(p[1]), Math.abs(p[2])) - g.radius;
  return p[2] - g.z0;
}

/** Projects `p` onto the undisplaced surface. */
export function toSurface(g: Pick<CellGeometry, 'kind' | 'radius' | 'z0'>, p: V3): V3 {
  if (g.kind === 'sphere') return mul(p, g.radius / (len(p) || 1));
  if (g.kind === 'cube') return mul(p, g.radius / (Math.max(Math.abs(p[0]), Math.abs(p[1]), Math.abs(p[2])) || 1));
  return [p[0], p[1], g.z0];
}

/** Direction in which relief lifts a corner at `p` (radial for sphere and cube, so folds stay joined; +z for the plane). */
export function liftDir(kind: ShapeKind, p: V3): V3 { return kind === 'plane' ? [0, 0, 1] : norm(p); }

function circumcentre(p0: V3, p1: V3, p2: V3): V3 | null {
  const a = sub(p1, p0), b = sub(p2, p0);
  const axb = cross(a, b);
  const d = 2 * dot(axb, axb);
  if (d < 1e-24) return null;
  const t = cross(sub(mul(b, dot(a, a)), mul(a, dot(b, b))), axb);
  return add(p0, mul(t, 1 / d));
}

export function buildCellGeometry(mesh: MeshData): CellGeometry {
  const kind = shapeKind(mesh);
  const n = mesh.nCells, C = mesh.centroid;
  const cen = (i: number): V3 => [C[3 * i], C[3 * i + 1], C[3 * i + 2]];
  let area = 0;
  for (let i = 0; i < n; i++) area += mesh.area[i];
  const spacing = Math.sqrt(area / Math.max(n, 1));

  let radius = 0, z0 = 0, hx = 0, hy = 0, rSum = 0;
  for (let i = 0; i < n; i++) {
    const x = C[3 * i], y = C[3 * i + 1], z = C[3 * i + 2];
    rSum += Math.hypot(x, y, z);
    radius = Math.max(radius, Math.abs(x), Math.abs(y), Math.abs(z));
    hx = Math.max(hx, Math.abs(x)); hy = Math.max(hy, Math.abs(y));
  }
  if (kind === 'sphere') radius = rSum / Math.max(n, 1);
  if (kind === 'plane') { z0 = n ? C[2] : 0; hx += spacing / 2; hy += spacing / 2; radius = Math.max(hx, hy); }
  const shape: Pick<CellGeometry, 'kind' | 'radius' | 'z0'> = { kind, radius, z0 };

  let cap = mesh.adj.length + n + 16;
  let base = new Float32Array(3 * cap), inc = new Int32Array(4 * cap);
  let idx = new Uint32Array(3 * cap), triCell = new Uint32Array(cap);
  let nc = 0, nt = 0;
  const cornerStart = new Uint32Array(n + 1);
  const grow = () => {
    cap = Math.ceil(cap * 1.5);
    const b = new Float32Array(3 * cap); b.set(base); base = b;
    const q = new Int32Array(4 * cap); q.set(inc); inc = q;
    const t = new Uint32Array(3 * cap); t.set(idx); idx = t;
    const tc = new Uint32Array(cap); tc.set(triCell); triCell = tc;
  };

  type Nb = { id: number; p: V3; ang: number };
  for (let i = 0; i < n; i++) {
    cornerStart[i] = nc;
    const c = cen(i);
    const u = surfaceNormal(kind, c);
    const ref: V3 = Math.abs(u[2]) > 0.9 ? [1, 0, 0] : [0, 0, 1];
    const e1 = norm(sub(ref, mul(u, dot(ref, u))));
    const e2 = cross(u, e1);
    const ring: Nb[] = [];
    for (let k = mesh.adjOffsets[i]; k < mesh.adjOffsets[i + 1]; k++) {
      const j = mesh.adj[k];
      if (j === i) continue;
      const pj = cen(j);
      const w = sub(pj, c);
      const wt = sub(w, mul(u, dot(w, u)));
      const lt = len(wt);
      if (lt < 1e-12 * (radius || 1)) continue;
      // cube: unfold a neighbour across an edge into the face plane (surface distance = along + across)
      const p = kind === 'cube' ? add(c, mul(wt, (lt + Math.abs(dot(w, u))) / lt)) : pj;
      ring.push({ id: j, p, ang: Math.atan2(dot(wt, e2), dot(wt, e1)) });
    }
    if (ring.length < 2) { cornerStart[i + 1] = nc; continue; }
    ring.sort((a, b) => a.ang - b.ang);
    // mirror the neighbours through the centre where a side has none (edge of a plane or disc)
    let minGap = Infinity, maxGap = 0;
    for (let k = 0; k < ring.length; k++) {
      const g = (k + 1 < ring.length ? ring[k + 1].ang : ring[0].ang + 2 * Math.PI) - ring[k].ang;
      minGap = Math.min(minGap, g); maxGap = Math.max(maxGap, g);
    }
    if (maxGap > 1.5 * minGap) {
      const extra: Nb[] = [];
      for (const r of ring) {
        let a = r.ang + Math.PI;
        if (a > Math.PI) a -= 2 * Math.PI;
        const near = ring.some((o) => { let d = Math.abs(o.ang - a); if (d > Math.PI) d = 2 * Math.PI - d; return d < 0.35 * minGap; });
        if (!near) extra.push({ id: -1, p: sub(mul(c, 2), r.p), ang: a });
      }
      ring.push(...extra);
      ring.sort((a, b) => a.ang - b.ang);
    }
    const m = ring.length;
    const first = nc;
    for (let k = 0; k < m; k++) {
      const a = ring[k], b = ring[(k + 1) % m];
      let v = circumcentre(c, a.p, b.p);
      if (!v) v = mul(add(c, add(a.p, b.p)), 1 / 3);
      if (kind === 'sphere') v = mul(v, radius / (len(v) || 1));
      if (nc >= cap) grow();
      base[3 * nc] = v[0]; base[3 * nc + 1] = v[1]; base[3 * nc + 2] = v[2];
      // cells meeting here: this one, the two neighbours, and any cell (a neighbour's neighbour) equidistant from the corner
      const set = [i, a.id, b.id].filter((x) => x >= 0);
      const d0 = len(sub(c, v));
      const tol = 0.1 * spacing;
      for (const s of [a.id, b.id]) {
        if (s < 0) continue;
        for (let q = mesh.adjOffsets[s]; q < mesh.adjOffsets[s + 1] && set.length < 4; q++) {
          const x = mesh.adj[q];
          if (set.includes(x)) continue;
          if (Math.abs(len(sub(cen(x), v)) - d0) < tol) set.push(x);
        }
      }
      for (let s = 0; s < 4; s++) inc[4 * nc + s] = s < set.length ? set[s] : -1;
      nc++;
    }
    for (let k = 1; k + 1 < m; k++) {
      if (nt >= cap) grow();
      idx[3 * nt] = first; idx[3 * nt + 1] = first + k; idx[3 * nt + 2] = first + k + 1;
      triCell[nt] = i;
      nt++;
    }
  }
  cornerStart[n] = nc;
  return {
    kind, nCells: n, radius, z0, half: [hx, hy], spacing, cornerStart,
    base: base.slice(0, 3 * nc), incident: inc.slice(0, 4 * nc), indices: idx.slice(0, 3 * nt), triCell: triCell.slice(0, nt), nCorners: nc,
  };
}

/**
 * Relief height per cell in mesh units: max(elevation, 0) * exaggeration * unitsPerMetre. Sea stays at the base surface (the sea is flat);
 * pass `null` for no relief. Never negative, so the displaced surface is always at or above the base surface (the picker relies on that).
 */
export function cellRelief(elevation: Float32Array | null, exaggeration: number, unitsPerMetre: number): Float32Array | null {
  if (!elevation || exaggeration <= 0) return null;
  const out = new Float32Array(elevation.length);
  const k = exaggeration * unitsPerMetre;
  for (let i = 0; i < out.length; i++) { const e = elevation[i]; out[i] = Number.isFinite(e) && e > 0 ? e * k : 0; }
  return out;
}

/** Writes displaced corner positions into `out` (3 per corner). `relief` null = the base surface. */
export function displaceCorners(g: CellGeometry, relief: Float32Array | null, out: Float32Array): void {
  const { base, incident, nCorners, kind } = g;
  for (let j = 0; j < nCorners; j++) {
    let x = base[3 * j], y = base[3 * j + 1], z = base[3 * j + 2];
    if (relief) {
      let s = 0, k = 0;
      for (let q = 0; q < 4; q++) { const c = incident[4 * j + q]; if (c >= 0) { s += relief[c]; k++; } }
      const d = k ? s / k : 0;
      if (d > 0) {
        if (kind === 'plane') z += d;
        else { const l = Math.hypot(x, y, z) || 1; const f = d / l; x += x * f; y += y * f; z += z * f; }
      }
    }
    out[3 * j] = x; out[3 * j + 1] = y; out[3 * j + 2] = z;
  }
}

/** One RGB triple per corner from one RGB triple per cell (the same `cellColors` the 2D map uses). */
export function cornerColors(g: CellGeometry, cell: Uint8ClampedArray, out?: Uint8Array): Uint8Array {
  const res = out ?? new Uint8Array(3 * g.nCorners);
  for (let i = 0; i < g.nCells; i++) {
    const r = cell[3 * i], gg = cell[3 * i + 1], b = cell[3 * i + 2];
    for (let j = g.cornerStart[i]; j < g.cornerStart[i + 1]; j++) { res[3 * j] = r; res[3 * j + 1] = gg; res[3 * j + 2] = b; }
  }
  return res;
}

/** World position of a cell's relief top (centroid lifted), used by focus and tests. */
export function cellTop(g: CellGeometry, mesh: MeshData, cell: number, relief: Float32Array | null): V3 {
  const c: V3 = [mesh.centroid[3 * cell], mesh.centroid[3 * cell + 1], mesh.centroid[3 * cell + 2]];
  const d = relief ? relief[cell] : 0;
  return d > 0 ? add(c, mul(liftDir(g.kind, c), d)) : c;
}
