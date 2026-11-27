// Synthetic meshes for tests: any size, no server needed. They follow the conventions of the real generators (sphere: cells on a sphere with
// icosahedral adjacency; cube: 6 n x n faces with cross-edge neighbours; plane: n x m square grid, z = 0).
import type { MeshData } from 'wb-client';

function finish(kind: string, c: number[], area: number[], nb: number[][]): MeshData {
  const n = area.length, offs = new Uint32Array(n + 1);
  let tot = 0;
  for (let i = 0; i < n; i++) { tot += nb[i].length; offs[i + 1] = tot; }
  const adj = new Uint32Array(tot);
  let k = 0;
  for (let i = 0; i < n; i++) for (const j of nb[i]) adj[k++] = j;
  return { meshId: `${kind}-${n}`, kind, nCells: n, centroid: Float32Array.from(c), area: Float32Array.from(area), adjOffsets: offs, adj };
}

/** Icosphere vertices as cells: 10 * 4^level + 2 cells (level 8 = 655362). */
export function sphereMesh(level: number, R = 1): MeshData {
  const t = (1 + Math.sqrt(5)) / 2;
  let v: number[][] = [[-1, t, 0], [1, t, 0], [-1, -t, 0], [1, -t, 0], [0, -1, t], [0, 1, t], [0, -1, -t], [0, 1, -t], [t, 0, -1], [t, 0, 1], [-t, 0, -1], [-t, 0, 1]]
    .map(([x, y, z]) => { const l = Math.hypot(x, y, z); return [x / l, y / l, z / l]; });
  let f: number[][] = [[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
  for (let l = 0; l < level; l++) {
    const cache = new Map<number, number>();
    const mid = (a: number, b: number) => {
      const key = a < b ? a * 4194304 + b : b * 4194304 + a;
      let m = cache.get(key);
      if (m === undefined) {
        const x = v[a][0] + v[b][0], y = v[a][1] + v[b][1], z = v[a][2] + v[b][2], q = Math.hypot(x, y, z);
        m = v.length; v.push([x / q, y / q, z / q]); cache.set(key, m);
      }
      return m;
    };
    const nf: number[][] = [];
    for (const [a, b, c] of f) { const ab = mid(a, b), bc = mid(b, c), ca = mid(c, a); nf.push([a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]); }
    f = nf;
  }
  const n = v.length, nb: Set<number>[] = Array.from({ length: n }, () => new Set());
  for (const [a, b, c] of f) { nb[a].add(b); nb[a].add(c); nb[b].add(a); nb[b].add(c); nb[c].add(a); nb[c].add(b); }
  const c: number[] = [];
  for (const p of v) c.push(p[0] * R, p[1] * R, p[2] * R);
  return finish('sphere', c, new Array(n).fill((4 * Math.PI * R * R) / n), nb.map((s) => [...s]));
}

const FACES: [number[], number[], number[]][] = [
  [[1, 0, 0], [0, 1, 0], [0, 0, 1]], [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
  [[0, 1, 0], [0, 0, 1], [1, 0, 0]], [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
  [[0, 0, 1], [1, 0, 0], [0, 1, 0]], [[0, 0, -1], [0, 1, 0], [1, 0, 0]],
];
const mix = (a: number[], x: number, b: number[], y: number, c: number[], z: number) => [0, 1, 2].map((k) => a[k] * x + b[k] * y + c[k] * z);
const dot = (a: number[], b: number[]) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

/** Cube with n x n cells per face (6 n^2 cells), half side hw. */
export function cubeMesh(n: number, hw = 1): MeshData {
  const h = 2 / n;
  const locate = (q: number[]) => {
    const a = q.map(Math.abs);
    const ax = a[0] >= a[1] && a[0] >= a[2] ? 0 : a[1] >= a[2] ? 1 : 2;
    const f = FACES.findIndex(([nr]) => nr[ax] === Math.sign(q[ax]) && nr.some((x) => x !== 0));
    const [, u, v] = FACES[f];
    const i = Math.min(n - 1, Math.max(0, Math.floor(((dot(q, u) + 1) * n) / 2))), j = Math.min(n - 1, Math.max(0, Math.floor(((dot(q, v) + 1) * n) / 2)));
    return f * n * n + j * n + i;
  };
  const c: number[] = [], nb: number[][] = [];
  for (let f = 0; f < 6; f++) {
    const [nr, u, v] = FACES[f];
    for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) {
      const s = -1 + (2 * i + 1) / n, t = -1 + (2 * j + 1) / n;
      const q = mix(nr, 1, u, s, v, t);
      c.push(q[0] * hw, q[1] * hw, q[2] * hw);
      const list: number[] = [];
      for (const [du, dv] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const s2 = s + du * h, t2 = t + dv * h;
        let pt: number[];
        if (Math.abs(s2) < 1 && Math.abs(t2) < 1) pt = mix(nr, 1, u, s2, v, t2);
        else if (s2 > 1) pt = mix(u, 1, nr, 1 - (s2 - 1), v, t);
        else if (s2 < -1) pt = mix(u, -1, nr, 1 - (-1 - s2), v, t);
        else if (t2 > 1) pt = mix(v, 1, nr, 1 - (t2 - 1), u, s);
        else pt = mix(v, -1, nr, 1 - (-1 - t2), u, s);
        list.push(locate(pt));
      }
      nb.push(list);
    }
  }
  return finish('cube', c, new Array(6 * n * n).fill(h * hw * h * hw), nb);
}

export function planeMesh(nx: number, ny: number, size = 1): MeshData {
  const c: number[] = [], nb: number[][] = [];
  for (let j = 0; j < ny; j++) for (let i = 0; i < nx; i++) {
    c.push((i + 0.5 - nx / 2) * size, (j + 0.5 - ny / 2) * size, 0);
    const l: number[] = [];
    for (const [di, dj] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) { const a = i + di, b = j + dj; if (a >= 0 && a < nx && b >= 0 && b < ny) l.push(b * nx + a); }
    nb.push(l);
  }
  return finish('plane', c, new Array(nx * ny).fill(size * size), nb);
}

/** Deterministic pseudo-elevation (metres) so relief tests are repeatable. */
export function fakeElevation(mesh: MeshData): Float32Array {
  const e = new Float32Array(mesh.nCells);
  for (let i = 0; i < e.length; i++) {
    const x = mesh.centroid[3 * i], y = mesh.centroid[3 * i + 1], z = mesh.centroid[3 * i + 2];
    e[i] = 3000 * Math.sin(3 * x + 1) * Math.cos(2 * y) + 2000 * Math.sin(4 * z) - 500;
  }
  return e;
}
