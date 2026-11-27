// Part 9e acceptance tests (headless, no browser, no server): stack model, overlay geometry, 2D arrows, rivers, inspector, presets, particles.
import { describe, expect, it } from 'vitest';
import type { CellInfo, OverlayDef, RiversData, Schema } from 'wb-client';
import { buildOverlayItems, dataNeeds, overlayAvailable } from '../src/layers/items';
import { inspectorModel } from '../src/layers/inspector';
import { drawnOverlays, emptyStack, isDrawn, matchesPreset, moveBy, moveTo, order, setBlend, setOpacity, setVisible, stackFromPreset, toggleSolo } from '../src/layers/stack';
import { arrowLength, cellSpacing, glyphsFor, maskTriangles, outlineSegments, ringsForCount, riverPolylines, thinCells, vectorStats } from '../src/overlays/geometry';
import { arrows2D, flow2D, lines2D, segments2D } from '../src/viewport/overlay2d';
import { makeParticles, seededRandom, stepParticles } from '../src/viewport/particles';
import { makeForward } from '../src/viewport/projections';
import { cubeMesh, fakeElevation, planeMesh, sphereMesh } from './synth';

const overlays: OverlayDef[] = [
  { id: 'wind', label: 'Wind', kind: 'vector', field: 'wind_velocity', token: 'wind', animated: true },
  { id: 'currents', label: 'Currents', kind: 'vector', field: 'ocean_current', token: 'current', animated: true },
  { id: 'rivers', label: 'Rivers', kind: 'lines', source: 'rivers', weight_field: 'discharge', token: 'river' },
  { id: 'lakes', label: 'Lakes', kind: 'mask', field: 'lake_mask', threshold: 0.5, mode: 'fill', token: 'lake' },
  { id: 'coast', label: 'Coast', kind: 'mask', field: 'ocean_mask', threshold: 0.5, mode: 'outline', token: 'coast' },
] as OverlayDef[];

const schema = {
  api_version: 1,
  params: [], presets: [],
  fields: [
    { name: 'temperature', kind: 'scalar', unit: 'K', display: { scale: 1, offset: -273.15, unit: '°C' }, colormap: 'x', range: null, description: '', featured: true, produced_by: ['energy'] },
    { name: 'precipitation', kind: 'scalar', unit: 'm/s', display: { scale: 1000, offset: 0, unit: 'mm' }, colormap: 'x', range: null, description: '', featured: true, produced_by: ['moisture'] },
    { name: 'elevation', kind: 'scalar', unit: 'm', display: { scale: 1, offset: 0, unit: 'm' }, colormap: 'x', range: null, description: '', featured: true, produced_by: ['tectonics'] },
  ],
  modules: [{ name: 'tectonics' }, { name: 'energy' }, { name: 'moisture' }],
  overlays,
  layer_presets: [
    { id: 'climate', name: 'Climate', description: '', base: 'precipitation', overlays: ['wind'] },
    { id: 'hydrology', name: 'Hydrology', description: '', base: 'elevation', overlays: ['lakes', 'rivers', 'coast'] },
    { id: 'ocean', name: 'Ocean', description: '', base: 'temperature', overlays: ['currents', 'coast'] },
    { id: 'terrain', name: 'Terrain', description: '', base: 'elevation', overlays: ['coast'] },
  ],
} as unknown as Schema;

describe('9e layer stack model', () => {
  it('presets switch the whole stack in one step and are recognised', () => {
    const h = stackFromPreset(schema, 'hydrology');
    expect(drawnOverlays(h, schema).map((d) => d.def.id)).toEqual(['lakes', 'rivers', 'coast']);
    expect(h.preset).toBe('hydrology');
    const c = stackFromPreset(schema, 'climate');
    expect(drawnOverlays(c, schema).map((d) => d.def.id)).toEqual(['wind']);
    expect(matchesPreset(c, schema, 'climate')).toBe(true);
    expect(matchesPreset(c, schema, 'hydrology')).toBe(false);
    expect(matchesPreset(setVisible(c, 'wind', false), schema, 'climate')).toBe(false);
    expect(stackFromPreset(schema, 'nope').entries.every((e) => !e.visible)).toBe(true);
  });

  it('reordering changes the draw order only, never the entries', () => {
    const s = stackFromPreset(schema, 'hydrology');
    const before = new Map(s.entries.map((e) => [e.id, JSON.stringify({ ...e })]));
    const t = moveTo(s, 'rivers', 0);
    expect(order(t)[0]).toBe('rivers');
    expect([...order(t)].sort()).toEqual([...order(s)].sort());
    for (const e of t.entries) expect(JSON.stringify(e)).toBe(before.get(e.id));
    expect(drawnOverlays(t, schema)[0].def.id).toBe('rivers');
    expect(s.entries.map((e) => e.id)).not.toEqual(t.entries.map((e) => e.id)); // original not mutated in place
    expect(order(moveBy(moveBy(s, 'lakes', 1), 'lakes', -1))).toEqual(order(s));
    expect(moveTo(s, 'missing', 0)).toBe(s);
  });

  it('opacity clamps, blend and solo work, base can be hidden but not reordered', () => {
    let s = stackFromPreset(schema, 'hydrology');
    s = setOpacity(s, 'rivers', 7);
    expect(s.entries.find((e) => e.id === 'rivers')!.opacity).toBe(1);
    s = setOpacity(s, 'rivers', -1);
    expect(s.entries.find((e) => e.id === 'rivers')!.opacity).toBe(0);
    s = setBlend(s, 'lakes', 'multiply');
    expect(s.entries.find((e) => e.id === 'lakes')!.blend).toBe('multiply');
    s = toggleSolo(s, 'rivers');
    expect(drawnOverlays(s, schema).map((d) => d.def.id)).toEqual(['rivers']);
    expect(isDrawn(s, 'base')).toBe(false);
    s = toggleSolo(s, 'rivers');
    expect(s.solo).toBeNull();
    expect(isDrawn(setVisible(s, 'base', false), 'base')).toBe(false);
    expect(order(setVisible(s, 'base', false))).toEqual(order(s));
    expect(emptyStack(null).entries).toEqual([]);
  });

  it('data needs follow the drawn overlays only, and availability follows the declaration', () => {
    const avail = (f: string) => f !== 'ocean_current';
    const n = dataNeeds(stackFromPreset(schema, 'hydrology'), schema, avail);
    expect(n.rivers).toBe(true);
    expect(n.outlines).toEqual([{ field: 'ocean_mask', threshold: 0.5 }]);
    expect(n.fields).toEqual([{ name: 'lake_mask', layer: 0 }]);
    const c = dataNeeds(stackFromPreset(schema, 'climate'), schema, avail);
    expect(c.fields.map((f) => f.layer)).toEqual([0, 1, 2]);
    expect(dataNeeds(stackFromPreset(schema, 'ocean'), schema, avail).fields).toEqual([]); // unsolved current is skipped
    expect(overlayAvailable(overlays[1], avail)).toBe(false);
  });
});

describe('9e arrows match the field', () => {
  it('uniform wind on a plane gives uniform arrows pointing the same way as the vector', () => {
    const mesh = planeMesh(20, 10);
    const n = mesh.nCells;
    const comps: [Float32Array, Float32Array, Float32Array] = [new Float32Array(n).fill(5), new Float32Array(n).fill(0), new Float32Array(n)];
    const a = arrows2D(mesh, makeForward(mesh), 2, comps, thinCells(mesh, 1));
    expect(a.cell.length).toBeGreaterThan(10);
    for (let k = 0; k < a.cell.length; k++) {
      expect(a.dx[k]).toBeCloseTo(1, 5); // +x in the mesh is to the right on screen
      expect(a.dy[k]).toBeCloseTo(0, 5);
      expect(a.mag[k]).toBeCloseTo(5, 5);
    }
    // +y in the mesh points up the map, which is -y on screen
    comps[0].fill(0); comps[1].fill(3);
    const b = arrows2D(mesh, makeForward(mesh), 2, comps, thinCells(mesh, 1));
    for (let k = 0; k < b.cell.length; k++) { expect(b.dx[k]).toBeCloseTo(0, 5); expect(b.dy[k]).toBeCloseTo(-1, 5); }
  });

  it('on a sphere an eastward vector is a right-pointing arrow everywhere; a northward one points up', () => {
    const mesh = sphereMesh(3);
    const n = mesh.nCells;
    const east: [Float32Array, Float32Array, Float32Array] = [new Float32Array(n), new Float32Array(n), new Float32Array(n)];
    const north: [Float32Array, Float32Array, Float32Array] = [new Float32Array(n), new Float32Array(n), new Float32Array(n)];
    for (let i = 0; i < n; i++) {
      const x = mesh.centroid[3 * i], y = mesh.centroid[3 * i + 1], z = mesh.centroid[3 * i + 2];
      const r = Math.hypot(x, y);
      if (r < 0.2) continue; // near the poles east is ill-defined on the equirectangular map
      east[0][i] = -y / r; east[1][i] = x / r;
      north[0][i] = (-x * z) / r; north[1][i] = (-y * z) / r; north[2][i] = r;
    }
    const fwd = makeForward(mesh);
    const cells = thinCells(mesh, 1);
    const e = arrows2D(mesh, fwd, 2, east, cells);
    expect(e.cell.length).toBeGreaterThan(5);
    for (let k = 0; k < e.cell.length; k++) { expect(e.dx[k]).toBeGreaterThan(0.99); expect(Math.abs(e.dy[k])).toBeLessThan(0.1); }
    const nn = arrows2D(mesh, fwd, 2, north, cells);
    for (let k = 0; k < nn.cell.length; k++) { expect(nn.dy[k]).toBeLessThan(-0.99); expect(Math.abs(nn.dx[k])).toBeLessThan(0.1); }
  });

  it('a vector with a normal component is projected onto the surface (and a purely normal one gives no arrow)', () => {
    const mesh = sphereMesh(2);
    const n = mesh.nCells;
    const radial: [Float32Array, Float32Array, Float32Array] = [new Float32Array(n), new Float32Array(n), new Float32Array(n)];
    for (let i = 0; i < n; i++) { radial[0][i] = mesh.centroid[3 * i]; radial[1][i] = mesh.centroid[3 * i + 1]; radial[2][i] = mesh.centroid[3 * i + 2]; }
    expect(glyphsFor(mesh, radial, [0, 1, 2, 3])).toHaveLength(0);
    const g = glyphsFor(mesh, [new Float32Array(n).fill(NaN), new Float32Array(n), new Float32Array(n)], [0, 1]);
    expect(g).toHaveLength(0); // missing values are skipped, not drawn as garbage
  });

  it('works on the cube cross too, and flow table is zero only where undefined', () => {
    const mesh = cubeMesh(6);
    const n = mesh.nCells;
    const c: [Float32Array, Float32Array, Float32Array] = [new Float32Array(n).fill(1), new Float32Array(n).fill(0.5), new Float32Array(n).fill(0.2)];
    const fwd = makeForward(mesh);
    const a = arrows2D(mesh, fwd, 4 / 3, c, thinCells(mesh, 0));
    expect(a.cell.length).toBeGreaterThan(n / 2);
    for (let k = 0; k < a.cell.length; k++) expect(Math.hypot(a.dx[k], a.dy[k])).toBeCloseTo(1, 4);
    const f = flow2D(mesh, fwd, 4 / 3, c, 1);
    expect(f).toHaveLength(2 * n);
  });

  it('thinning keeps cells apart, is deterministic, and zoom picks fewer rings for fewer arrows', () => {
    const mesh = sphereMesh(4);
    const t1 = thinCells(mesh, 2);
    expect(thinCells(mesh, 2)).toBe(t1); // cached, same set
    expect(t1.length).toBeLessThan(mesh.nCells / 5);
    const kept = new Set(t1);
    for (const c of t1) for (let q = mesh.adjOffsets[c]; q < mesh.adjOffsets[c + 1]; q++) expect(kept.has(mesh.adj[q])).toBe(false);
    expect(thinCells(mesh, 0)).toHaveLength(mesh.nCells);
    expect(ringsForCount(mesh.nCells, 100)).toBeGreaterThan(ringsForCount(mesh.nCells, 1000));
    expect(cellSpacing(mesh)).toBeGreaterThan(0);
  });

  it('arrow length grows with magnitude between the pixel bounds', () => {
    expect(arrowLength(0, 10, 6, 30)).toBe(6);
    expect(arrowLength(10, 10, 6, 30)).toBe(30);
    expect(arrowLength(100, 10, 6, 30)).toBe(30);
    expect(arrowLength(5, 10, 6, 30)).toBeCloseTo(18);
    expect(arrowLength(5, 0, 6, 30)).toBe(6);
    const s = vectorStats([Float32Array.from([3, 0]), Float32Array.from([4, 0]), new Float32Array(2)]);
    expect(s.max).toBe(5);
  });
});

/** A river from a source to a lake-free outlet along a row of a plane mesh, plus a tributary joining it. */
function toyRivers(mesh: ReturnType<typeof planeMesh>, nx: number): RiversData {
  const row = (j: number, i0: number, i1: number) => { const o: number[] = []; for (let i = i0; i <= i1; i++) o.push(j * nx + i); return o; };
  const main = row(4, 2, 17);
  const trib = [2 * nx + 10, 3 * nx + 10, main[8]];
  const lines = [main, trib];
  const offsets = [0]; const cells: number[] = []; const q: number[] = [];
  for (const l of lines) { for (let k = 0; k < l.length; k++) { cells.push(l[k]); q.push(l === main ? 10 + 50 * k : 5 + k); } offsets.push(cells.length); }
  return { meshId: mesh.meshId, version: 1, stale: false, nLines: 2, nPoints: cells.length, offsets: Uint32Array.from(offsets), cells: Uint32Array.from(cells), discharge: Float32Array.from(q) };
}

describe('9e rivers', () => {
  const nx = 20, ny = 10;
  const mesh = planeMesh(nx, ny);
  const rivers = toyRivers(mesh, nx);

  it('polylines are connected from source to outlet (consecutive vertices are neighbouring cells) with width growing downstream', () => {
    const poly = riverPolylines(mesh, rivers);
    for (let k = 0; k + 1 < poly.offsets.length; k++) {
      for (let i = poly.offsets[k]; i + 1 < poly.offsets[k + 1]; i++) {
        const a = rivers.cells[i], b = rivers.cells[i + 1];
        const nb = [...mesh.adj.slice(mesh.adjOffsets[a], mesh.adjOffsets[a + 1])];
        expect(nb).toContain(b);
        expect(poly.width[i + 1]).toBeGreaterThanOrEqual(poly.width[i] - 1e-9);
      }
    }
    expect(Math.max(...poly.width)).toBeCloseTo(1, 6);
    expect(Math.min(...poly.width)).toBeGreaterThan(0);
  });

  it('the 2D lines keep every polyline connected (one map-space step is one cell step) and are not split on a plane', () => {
    const l = lines2D(mesh, makeForward(mesh), rivers);
    expect(l.offsets.length).toBe(3);
    const step = Math.max(1 / (nx - 1), 1 / (ny - 1)) + 1e-6; // plane map spans the centroid extent
    for (let k = 0; k + 1 < l.offsets.length; k++) {
      for (let i = l.offsets[k]; i + 1 < l.offsets[k + 1]; i++) {
        const d = Math.hypot(l.points[2 * i] - l.points[2 * i + 2], l.points[2 * i + 1] - l.points[2 * i + 3]);
        expect(d).toBeLessThanOrEqual(step);
      }
    }
  });

  it('lines crossing the date line are split instead of drawn across the whole map', () => {
    const sph = sphereMesh(3);
    // walk around the equator: pick cells by longitude order
    const eq = [...Array(sph.nCells).keys()].filter((i) => Math.abs(sph.centroid[3 * i + 2]) < 0.2).sort((a, b) => Math.atan2(sph.centroid[3 * a + 1], sph.centroid[3 * a]) - Math.atan2(sph.centroid[3 * b + 1], sph.centroid[3 * b]));
    const r: RiversData = { meshId: sph.meshId, version: 1, stale: false, nLines: 1, nPoints: eq.length, offsets: Uint32Array.from([0, eq.length]), cells: Uint32Array.from(eq), discharge: new Float32Array(eq.length).fill(1) };
    const l = lines2D(sph, makeForward(sph), r, 0.5);
    for (let k = 0; k + 1 < l.offsets.length; k++) for (let i = l.offsets[k]; i + 1 < l.offsets[k + 1]; i++) expect(Math.abs(l.points[2 * i] - l.points[2 * i + 2])).toBeLessThanOrEqual(0.5);
  });

  it('rivers are drawn above the elevation layer: the base is never an overlay and the lines item comes after the base in every preset', () => {
    // the scalar base (elevation) is drawn first by the viewport; overlays come after, in stack order
    const s = stackFromPreset(schema, 'hydrology');
    const items = buildOverlayItems({ fields: {}, rivers, outlines: {} }, schema, s, () => '#00f');
    expect(items.map((i) => i.kind)).toEqual(['lines']);
    expect(drawnOverlays(s, schema).some((d) => d.def.field === 'elevation')).toBe(false);
    // and the stack only ever reorders overlays among themselves, so rivers cannot fall below the base
    expect(order(moveTo(s, 'rivers', 0)).includes('base')).toBe(false);
  });

  it('outline segments and mask triangles follow the cell pairs', () => {
    const el = fakeElevation(mesh);
    const inside = new Float32Array(mesh.nCells);
    for (let i = 0; i < inside.length; i++) inside[i] = el[i] > 0 ? 1 : 0;
    const pairs: number[] = [];
    for (let a = 0; a < mesh.nCells; a++) if (inside[a]) for (let q = mesh.adjOffsets[a]; q < mesh.adjOffsets[a + 1]; q++) if (!inside[mesh.adj[q]]) pairs.push(a, mesh.adj[q]);
    const o = { meshId: mesh.meshId, field: 'ocean_mask', threshold: 0.5, version: 1, stale: false, nEdges: pairs.length / 2, pairs: Uint32Array.from(pairs) };
    const seg = outlineSegments(mesh, o);
    expect(seg).toHaveLength(6 * o.nEdges);
    for (let e = 0; e < o.nEdges; e++) {
      const a = o.pairs[2 * e], b = o.pairs[2 * e + 1];
      const mx = (seg[6 * e] + seg[6 * e + 3]) / 2, my = (seg[6 * e + 1] + seg[6 * e + 4]) / 2;
      expect(mx).toBeCloseTo((mesh.centroid[3 * a] + mesh.centroid[3 * b]) / 2, 5);
      expect(my).toBeCloseTo((mesh.centroid[3 * a + 1] + mesh.centroid[3 * b + 1]) / 2, 5);
    }
    expect(segments2D(mesh, makeForward(mesh), o).length).toBe(4 * o.nEdges);
    const tri = maskTriangles(Uint32Array.from([0, 1]), Uint32Array.from([0, 1, 2, 3, 4, 5]), Float32Array.from([0, 1]), 0.5);
    expect([...tri]).toEqual([3, 4, 5]);
  });
});

describe('9e inspector shows what get_cell returned', () => {
  const info = (cell: number, t: number, extra: Partial<CellInfo> = {}): CellInfo => ({
    cell, lat_deg: 12.5, lon_deg: -40, biome_name: 'Savanna', stale: false,
    values: { temperature: t, precipitation: 3e-8, elevation: 412, mystery: 7 },
    seasonal: { temperature: Array.from({ length: 12 }, (_, m) => t + m) },
    ...extra,
  } as unknown as CellInfo);

  it('every value is the raw number from get_cell; text only applies the schema display scaling', () => {
    const c = info(5, 290);
    const m = inspectorModel(schema, c);
    const rows = m.groups.flatMap((g) => g.rows);
    expect(rows.map((r) => r.name).sort()).toEqual(Object.keys(c.values).sort());
    for (const r of rows) expect(r.raw).toBe(c.values[r.name]);
    expect(rows.find((r) => r.name === 'temperature')!.text).toBe('16.85 °C');
    expect(rows.find((r) => r.name === 'mystery')!.text).toBe('7.00'); // unknown to the schema: shown as sent
    expect(m.groups.map((g) => g.id)).toEqual(['tectonics', 'energy', 'moisture', 'other']); // pipeline order, unknown last
    expect(m.subtitle).toBe('Savanna');
  });

  it('seasonal values are carried unchanged for the 12 seasons', () => {
    const c = info(5, 290);
    const row = inspectorModel(schema, c).groups.flatMap((g) => g.rows).find((r) => r.name === 'temperature')!;
    expect(row.seasons!.raw).toEqual(c.seasonal!.temperature);
    expect(row.seasons!.text).toHaveLength(12);
    expect(row.seasons!.min).toBe('16.85');
    expect(row.seasons!.max).toBe('27.85');
  });

  it('a pinned cell adds a side-by-side column with the difference', () => {
    const a = info(5, 290), b = info(9, 280);
    const m = inspectorModel(schema, a, b);
    expect(m.pinnedTitle).toContain('Cell 9');
    const t = m.groups.flatMap((g) => g.rows).find((r) => r.name === 'temperature')!;
    expect(t.other!.raw).toBe(280);
    expect(t.other!.delta).toBe('+10.00');
    expect(inspectorModel(schema, a).groups.flatMap((g) => g.rows).every((r) => r.other === undefined)).toBe(true);
  });

  it('works without a schema and flags stale data', () => {
    const m = inspectorModel(null, info(1, 1, { stale: true }));
    expect(m.stale).toBe(true);
    expect(m.groups).toHaveLength(1);
  });
});

describe('9e animated streaks', () => {
  it('particles follow the flow field and respawn when they leave the map', () => {
    const n = 200;
    const rand = seededRandom(7);
    const p = makeParticles(n, 4, rand);
    const flow = new Float32Array(2 * 4);
    for (let c = 0; c < 4; c++) flow[2 * c] = 1; // everything flows right
    const start = Float32Array.from({ length: n }, (_, i) => p.hist[i * 8]);
    stepParticles(p, flow, () => 0, 0.01, { speed: 0.5, life: 100, aspect: 2, wrap: false }, rand);
    let moved = 0;
    for (let i = 0; i < n; i++) {
      const u = p.hist[i * 8];
      if (u > start[i]) { expect(u - start[i]).toBeCloseTo(0.005, 5); moved++; } // exactly speed * dt, and the previous point is the trail
    }
    expect(moved).toBeGreaterThan(n * 0.9);
    // run until nearly all have left: they respawn inside the map, never outside
    for (let s = 0; s < 400; s++) stepParticles(p, flow, () => 0, 0.05, { speed: 0.5, life: 100, aspect: 2, wrap: false }, rand);
    for (let i = 0; i < n; i++) { expect(p.hist[i * 8]).toBeGreaterThanOrEqual(0); expect(p.hist[i * 8]).toBeLessThan(1); }
  });

  it('wrap keeps particles in the map; cells outside the map and old age respawn; same seed gives the same first frame', () => {
    const rand = seededRandom(1);
    const p = makeParticles(50, 3, rand);
    const flow = Float32Array.from([1, 0]);
    for (let s = 0; s < 100; s++) stepParticles(p, flow, () => 0, 0.1, { speed: 1, life: 1000, aspect: 1, wrap: true }, rand);
    for (let i = 0; i < 50; i++) { expect(p.hist[i * 6]).toBeGreaterThanOrEqual(0); expect(p.hist[i * 6]).toBeLessThan(1); }
    const q = makeParticles(50, 3, seededRandom(9)), r = makeParticles(50, 3, seededRandom(9));
    expect([...q.hist]).toEqual([...r.hist]);
    const before = [...q.age];
    stepParticles(q, flow, () => -1, 0.1, { speed: 1, life: 10, aspect: 1, wrap: true }, seededRandom(3));
    expect(q.age.every((a) => a === 0)).toBe(true); // all respawned (age reset)
    expect(before.some((a) => a > 0)).toBe(true);
  });
});
