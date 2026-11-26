// Projection strategies: pixel -> 3D point on the mesh (mirror of wb-app/src/raster.rs `projection`), and the inverse "which pixel is this 3D point" is never needed:
// the viewport asks "which cell is under this pixel" via `buildPixelCells`.
import type { MeshData } from 'wb-client';

export interface Projection {
  kind: 'equirectangular' | 'cube_cross' | 'plane';
  width: number;
  height: number;
  /** Pixel (integer coordinates) -> point in mesh space, or null outside the map (the empty corners of the cube cross). */
  point(x: number, y: number): [number, number, number] | null;
}

export function makeProjection(mesh: MeshData, width: number): Projection {
  const c = mesh.centroid;
  const n = mesh.nCells;
  switch (mesh.kind) {
    case 'cube': {
      let half = 0;
      for (let i = 0; i < c.length; i++) half = Math.max(half, Math.abs(c[i]));
      const s = Math.max(Math.floor(width / 4), 2);
      return {
        kind: 'cube_cross', width: s * 4, height: s * 3,
        point(x, y) {
          const fc = Math.floor(x / s), fr = Math.floor(y / s);
          const a = ((x % s) + 0.5) / s * 2 - 1;
          const b = 1 - ((y % s) + 0.5) / s * 2;
          let p: [number, number, number];
          if (fr === 1 && fc === 0) p = [-1, -a, b];
          else if (fr === 1 && fc === 1) p = [a, -1, b];
          else if (fr === 1 && fc === 2) p = [1, a, b];
          else if (fr === 1 && fc === 3) p = [-a, 1, b];
          else if (fr === 0 && fc === 1) p = [a, b, 1];
          else if (fr === 2 && fc === 1) p = [a, -b, -1];
          else return null;
          return [p[0] * half, p[1] * half, p[2] * half];
        },
      };
    }
    case 'plane': {
      let x0 = Infinity, x1 = -Infinity, y0 = Infinity, y1 = -Infinity;
      for (let i = 0; i < n; i++) {
        x0 = Math.min(x0, c[3 * i]); x1 = Math.max(x1, c[3 * i]);
        y0 = Math.min(y0, c[3 * i + 1]); y1 = Math.max(y1, c[3 * i + 1]);
      }
      const ex = Math.max(x1 - x0, 1e-9), ey = Math.max(y1 - y0, 1e-9), z = c[2];
      const w = width, h = Math.max(Math.round((width * ey) / ex), 1);
      return { kind: 'plane', width: w, height: h, point: (px, py) => [x0 + ((px + 0.5) / w) * ex, y1 - ((py + 0.5) / h) * ey, z] };
    }
    default: {
      // sphere: equirectangular
      let r = 0;
      for (let i = 0; i < n; i++) r += Math.hypot(c[3 * i], c[3 * i + 1], c[3 * i + 2]);
      r /= Math.max(n, 1);
      const w = width, h = Math.max(Math.floor(width / 2), 1);
      return {
        kind: 'equirectangular', width: w, height: h,
        point(x, y) {
          const lon = (((x + 0.5) / w) * 2 - 1) * Math.PI;
          const lat = (0.5 - (y + 0.5) / h) * Math.PI;
          return [r * Math.cos(lat) * Math.cos(lon), r * Math.cos(lat) * Math.sin(lon), r * Math.sin(lat)];
        },
      };
    }
  }
}

function dist2(c: Float32Array, i: number, p: [number, number, number]): number {
  const dx = c[3 * i] - p[0], dy = c[3 * i + 1] - p[1], dz = c[3 * i + 2] - p[2];
  return dx * dx + dy * dy + dz * dz;
}

export function bruteCell(mesh: MeshData, p: [number, number, number]): number {
  let best = 0, bd = Infinity;
  for (let i = 0; i < mesh.nCells; i++) {
    const d = dist2(mesh.centroid, i, p);
    if (d < bd) { bd = d; best = i; }
  }
  return best;
}

/** Greedy descent over the adjacency graph from `start` (same tie rules as Rust `locate`). */
export function locateCell(mesh: MeshData, p: [number, number, number], start: number): number {
  let cur = start;
  let best = dist2(mesh.centroid, cur, p);
  for (let iter = 0; iter < mesh.nCells; iter++) {
    let moved = false;
    const end = mesh.adjOffsets[cur + 1];
    for (let k = mesh.adjOffsets[cur]; k < end; k++) {
      const nb = mesh.adj[k];
      const dd = dist2(mesh.centroid, nb, p);
      if (dd < best) { best = dd; cur = nb; moved = true; }
    }
    if (!moved) break;
  }
  return cur;
}

/** Cell under every pixel, row-major; -1 outside the map. Computed once per mesh + projection, then every layer switch is a table lookup. */
export function buildPixelCells(mesh: MeshData, proj: Projection): Int32Array {
  const out = new Int32Array(proj.width * proj.height).fill(-1);
  let area = 0;
  for (let i = 0; i < mesh.nCells; i++) area += mesh.area[i];
  const spacing = Math.sqrt(area / mesh.nCells);
  const near2 = (3 * spacing) ** 2;
  let last = -1;
  for (let y = 0; y < proj.height; y++) {
    for (let x = 0; x < proj.width; x++) {
      const p = proj.point(x, y);
      if (!p) continue;
      const start = last >= 0 && dist2(mesh.centroid, last, p) < near2 ? last : bruteCell(mesh, p);
      const cell = locateCell(mesh, p, start);
      last = cell;
      out[y * proj.width + x] = cell;
    }
  }
  return out;
}
