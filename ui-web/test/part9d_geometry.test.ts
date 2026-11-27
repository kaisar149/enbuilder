import { describe, expect, it } from 'vitest';
import { buildCellGeometry, cellRelief, displaceCorners, cornerColors, surfaceNormal } from '../src/viewport/geometry3d';
import { cubeMesh, fakeElevation, planeMesh, sphereMesh } from './synth';

const key = (b: Float32Array, j: number, q: number) => `${Math.round(b[3 * j] * q)},${Math.round(b[3 * j + 1] * q)},${Math.round(b[3 * j + 2] * q)}`;

function polygonEdgeCounts(g: ReturnType<typeof buildCellGeometry>, q: number) {
  const counts = new Map<string, number>();
  for (let i = 0; i < g.nCells; i++) {
    const a = g.cornerStart[i], b = g.cornerStart[i + 1];
    for (let j = a; j < b; j++) {
      const k = j + 1 < b ? j + 1 : a;
      const p = key(g.base, j, q), r = key(g.base, k, q);
      const e = p < r ? `${p}|${r}` : `${r}|${p}`;
      counts.set(e, (counts.get(e) ?? 0) + 1);
    }
  }
  return counts;
}

function area(g: ReturnType<typeof buildCellGeometry>) {
  let s = 0;
  for (let t = 0; t < g.triCell.length; t++) {
    const [a, b, c] = [g.indices[3 * t], g.indices[3 * t + 1], g.indices[3 * t + 2]];
    const u = [0, 1, 2].map((k) => g.base[3 * b + k] - g.base[3 * a + k]), v = [0, 1, 2].map((k) => g.base[3 * c + k] - g.base[3 * a + k]);
    s += 0.5 * Math.hypot(u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]);
  }
  return s;
}

function outwardFraction(g: ReturnType<typeof buildCellGeometry>) {
  let ok = 0;
  for (let t = 0; t < g.triCell.length; t++) {
    const [a, b, c] = [g.indices[3 * t], g.indices[3 * t + 1], g.indices[3 * t + 2]];
    const u = [0, 1, 2].map((k) => g.base[3 * b + k] - g.base[3 * a + k]), v = [0, 1, 2].map((k) => g.base[3 * c + k] - g.base[3 * a + k]);
    const n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    const cx = [0, 1, 2].map((k) => (g.base[3 * a + k] + g.base[3 * b + k] + g.base[3 * c + k]) / 3) as [number, number, number];
    const out = g.kind === 'plane' ? [0, 0, 1] : surfaceNormal(g.kind, cx);
    if (n[0] * out[0] + n[1] * out[1] + n[2] * out[2] > 0) ok++;
  }
  return ok / g.triCell.length;
}

describe('cell polygons', () => {
  const cases: [string, ReturnType<typeof sphereMesh>, number][] = [
    ['sphere', sphereMesh(4), 4 * Math.PI],
    ['cube', cubeMesh(12), 24],
    ['plane', planeMesh(20, 10), 200],
  ];
  for (const [name, mesh, surface] of cases) {
    it(`${name}: polygons tile the surface, face outward, and are watertight`, () => {
      const g = buildCellGeometry(mesh);
      expect(area(g) / surface).toBeGreaterThan(0.97);
      expect(area(g) / surface).toBeLessThan(1.03);
      expect(outwardFraction(g)).toBe(1);
      const counts = polygonEdgeCounts(g, 1e4 / g.radius);
      let one = 0, two = 0, other = 0;
      for (const c of counts.values()) { if (c === 1) one++; else if (c === 2) two++; else other++; }
      expect(other).toBe(0);
      if (name === 'plane') expect(one).toBe(2 * (20 + 10)); else expect(one).toBe(0);
      expect(two).toBeGreaterThan(0);
    });
  }

  it('relief keeps neighbouring polygons joined (corners at one place get one height)', () => {
    for (const mesh of [sphereMesh(3), cubeMesh(8), planeMesh(12, 9)]) {
      const g = buildCellGeometry(mesh);
      const relief = cellRelief(fakeElevation(mesh), 1, 1.0)!;
      const out = new Float32Array(3 * g.nCorners);
      displaceCorners(g, relief, out);
      const seen = new Map<string, [number, number, number]>();
      let bad = 0;
      for (let j = 0; j < g.nCorners; j++) {
        const k = key(g.base, j, 1e4 / g.radius);
        const p: [number, number, number] = [out[3 * j], out[3 * j + 1], out[3 * j + 2]];
        const q = seen.get(k);
        if (!q) seen.set(k, p);
        else if (Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]) > 1e-3 * g.radius) bad++;
      }
      expect(bad).toBe(0);
    }
  });

  it('corner colours repeat the per-cell colour (same colours as the 2D map)', () => {
    const mesh = sphereMesh(2);
    const g = buildCellGeometry(mesh);
    const cell = new Uint8ClampedArray(3 * mesh.nCells);
    for (let i = 0; i < cell.length; i++) cell[i] = (i * 37) % 256;
    const cc = cornerColors(g, cell);
    for (let i = 0; i < mesh.nCells; i++) for (let j = g.cornerStart[i]; j < g.cornerStart[i + 1]; j++) expect([cc[3 * j], cc[3 * j + 1], cc[3 * j + 2]]).toEqual([cell[3 * i], cell[3 * i + 1], cell[3 * i + 2]]);
  });
});
