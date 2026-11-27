// Part 9f acceptance tests for wb-client: tool geometry (pure) and the EditController against the REAL wb-server.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { EditController, StrokeBuffer, ToolRegistry, WbApp, brushDab, cellPath, falloffWeight, meshGeometry, needsRefresh } from '../src';
import { startServer, type RunningServer } from './harness';

describe('9f brush geometry and falloff (pure)', () => {
  it('falloff curves: 1 at the centre, 0 at the rim (constant stays 1), monotone', () => {
    for (const k of ['smooth', 'linear', 'sharp']) {
      expect(falloffWeight(k, 0)).toBeCloseTo(1);
      expect(falloffWeight(k, 1)).toBeCloseTo(0);
      let prev = 1;
      for (let t = 0.1; t <= 1; t += 0.1) { const w = falloffWeight(k, t); expect(w).toBeLessThanOrEqual(prev + 1e-12); prev = w; }
    }
    expect(falloffWeight('constant', 0.99)).toBe(1);
    expect(falloffWeight('linear', 0.5)).toBeCloseTo(0.5);
  });
});

describe('9f EditController (real wb-server)', () => {
  let srv: RunningServer;
  let app: WbApp;
  let ec: EditController;
  const count = (type: string) => app.client.sent.filter((r) => r.kind === 'command' && r.type === type).length;
  const elev = async () => (await app.client.getField('elevation')).values;
  const landCell = (z: Float32Array) => {
    const mesh = app.store.getState().mesh!;
    let best = -1, bs = -1;
    for (let i = 0; i < z.length; i++) {
      if (z[i] <= 0) continue;
      let s = 0;
      for (let k = mesh.adjOffsets[i]; k < mesh.adjOffsets[i + 1]; k++) if (z[mesh.adj[k]] > 0) s++;
      if (s > bs) { bs = s; best = i; }
    }
    return best;
  };

  beforeAll(async () => {
    srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=3', '--param', 'spinup_years=1']);
    app = await WbApp.connect(srv.wsUrl);
    expect(await app.solveAndWait()).toBe('completed');
    await app.idle();
    ec = new EditController(app);
  }, 120000);
  afterAll(async () => { app?.dispose(); await srv?.stop(); });

  it('the registry is built from schema.edit: every tool in the schema, nothing hard-coded', () => {
    const edit = app.store.getState().schema!.edit!;
    expect(edit).toBeTruthy();
    const reg = ToolRegistry.fromSchema(edit);
    expect(reg.list().map((t) => t.id)).toEqual(edit.tools.map((t) => t.id));
    expect(ec.tools.length).toBe(edit.tools.length);
  });

  it('brush dab covers a connected neighbourhood sized by the radius', () => {
    const mesh = app.store.getState().mesh!;
    const geom = meshGeometry(mesh);
    const small = brushDab(mesh, geom, 10, 600e3, 'smooth');
    const big = brushDab(mesh, geom, 10, 2000e3, 'smooth');
    expect(small.cells).toContain(10);
    expect(big.cells.length).toBeGreaterThan(small.cells.length);
    expect(Math.max(...big.weights)).toBeCloseTo(1);
    expect(big.weights[big.cells.indexOf(10)]).toBeCloseTo(1);
    for (const w of big.weights) expect(w).toBeGreaterThan(0);
  });

  it('cellPath fills the gap between two far apart picks with adjacent steps', () => {
    const mesh = app.store.getState().mesh!;
    const geom = meshGeometry(mesh);
    const path = cellPath(mesh, geom, 0, 300);
    expect(path[path.length - 1]).toBe(300);
    let prev = 0;
    for (const c of path.slice(0, -1)) {
      const nbs = Array.from(mesh.adj.slice(mesh.adjOffsets[prev], mesh.adjOffsets[prev + 1]));
      expect(nbs).toContain(c);
      prev = c;
    }
  });

  it('a long drag is ONE paint_stroke command (batching) and the stroke is one undo step', async () => {
    const z0 = await elev();
    const mesh = app.store.getState().mesh!;
    const c0 = landCell(z0);
    const before = count('paint_stroke');
    const undoBefore = app.store.getState().world!.history!.undo.length;
    ec.setTool('raise');
    ec.setBrush({ radiusKm: 900 });
    ec.pointerDown(c0);
    // drag across 40 picks along the mesh
    let c = c0;
    for (let i = 0; i < 40; i++) { c = mesh.adj[mesh.adjOffsets[c]]; ec.pointerMove(c); }
    expect(ec.state.preview!.cells.length).toBeGreaterThan(1);
    expect(count('paint_stroke')).toBe(before); // nothing sent while dragging
    const ack = await ec.pointerUp();
    expect(ack!.detail!.applied).toBeGreaterThan(1);
    expect(count('paint_stroke')).toBe(before + 1);
    await ec.idle();
    const w = app.store.getState().world!;
    expect(w.history!.undo.length).toBe(undoBefore + 1);
    expect(w.history!.undo.at(-1)!.kind).toBe('stroke');
    expect(w.edit!.paint.elevation).toBeGreaterThan(1);
    const z1 = await elev();
    expect(z1[c0]).toBeGreaterThan(z0[c0]);
  }, 60000);

  it('after a stroke the chain runs preview then refined, and the world is solved with the paint in it', async () => {
    await ec.idle();
    expect(ec.state.lastChain).toEqual(['preview', 'refined']);
    expect(app.store.getState().world!.edit!.quality).toBe('refined');
    expect(ec.state.phase).toBe('idle');
  });

  it('undo restores the exact elevation field and the stroke can be redone', async () => {
    const zPainted = await elev();
    await ec.undo();
    const zUndone = await elev();
    expect(app.store.getState().world!.edit!.paint.elevation).toBe(0);
    expect(Array.from(zUndone)).not.toEqual(Array.from(zPainted));
    await ec.redo();
    const zRedone = await elev();
    expect(Array.from(zRedone)).toEqual(Array.from(zPainted));
  }, 60000);

  it('paint and lock data decode and are cached per edit generation', async () => {
    const gen = app.store.getState().world!.edit!.generation;
    const p = await app.ensurePaint('elevation');
    expect(p.generation).toBe(gen);
    expect(p.mask.length).toBe(app.store.getState().mesh!.nCells);
    expect(Array.from(p.mask).filter((m) => m > 0).length).toBe(p.paintedCells);
    const before = app.client.sent.filter((r) => (r.body as any).type === 'get_paint').length;
    await app.ensurePaint('elevation');
    expect(app.client.sent.filter((r) => (r.body as any).type === 'get_paint').length).toBe(before); // cache hit
  });

  it('lock tool: locked cells are reported by get_locks and painting them is skipped', async () => {
    const z = await elev();
    const c = landCell(z);
    ec.setTool('lock');
    ec.setBrush({ radiusKm: 400, lockField: 'elevation' });
    ec.pointerDown(c);
    await ec.pointerUp();
    await ec.idle();
    const locks = await app.ensureLocks();
    expect(locks.cells.elevation).toContain(c);
    expect(app.store.getState().world!.edit!.locks.elevation).toBeGreaterThan(0);
    ec.setTool('raise');
    ec.pointerDown(c);
    const ack = await ec.pointerUp();
    expect(ack!.detail!.skipped_locked).toBeGreaterThan(0);
    const z2 = await elev();
    expect(z2[c]).toBe(z[c]);
  }, 60000);

  it('realism: 100 ignores the paint, 0 shows it (instant, no job)', async () => {
    await ec.idle();
    const jobsBefore = count('refresh') + count('solve');
    const painted = await app.client.getFieldView('elevation', 'painted');
    const computed = await app.client.getFieldView('elevation', 'computed');
    const locked = new Set(Array.from((await app.ensureLocks()).cells.elevation)); // locks hold their value whatever the slider says
    expect(locked.size).toBeGreaterThan(0);
    const same = (a: Float32Array, b: Float32Array, f = (x: number, y: number) => y) => {
      for (let i = 0; i < a.length; i++) if (!locked.has(i)) expect(a[i]).toBeCloseTo(f(painted.values[i], computed.values[i]), 2);
    };
    await ec.setRealism('elevation', 100);
    same((await app.client.getField('elevation')).values, computed.values, (_p, c) => c);
    await ec.setRealism('elevation', 0);
    same((await app.client.getField('elevation')).values, painted.values, (p) => p);
    await ec.setRealism('elevation', 50);
    same((await app.client.getField('elevation')).values, painted.values, (p, c) => (p + c) / 2);
    await ec.setRealism('elevation', 0);
    expect(count('refresh') + count('solve')).toBe(jobsBefore);
  }, 60000);

  it('dragging stays interactive: 300 pointer moves with a big brush cost well under a frame each (client side, no commands)', () => {
    const mesh = app.store.getState().mesh!;
    const sentBefore = app.client.sent.length;
    ec.setTool('raise');
    ec.setBrush({ radiusKm: 3000 });
    const t0 = performance.now();
    ec.pointerDown(0);
    let c = 0;
    for (let i = 0; i < 300; i++) { c = mesh.adj[mesh.adjOffsets[c] + (i % 3)]; ec.pointerMove(c); }
    const ms = performance.now() - t0;
    expect(ms / 300).toBeLessThan(5); // far below a 16 ms frame
    expect(app.client.sent.length).toBe(sentBefore);
    ec.cancelStroke();
    ec.setBrush({ radiusKm: 900 });
  });

  it('needsRefresh: only solver inputs trigger a refresh chain', () => {
    expect(needsRefresh({ type: 'paint_stroke', layer: 'elevation', op: 'add', cells: [], weights: [] })).toBe(true);
    expect(needsRefresh({ type: 'paint_stroke', layer: 'rainfall_bias', op: 'add', cells: [], weights: [] })).toBe(false);
    expect(needsRefresh({ type: 'set_lock', field: 'temperature', cells: [], locked: true })).toBe(true);
    expect(needsRefresh({ type: 'set_lock', field: 'biome', cells: [], locked: true })).toBe(false);
  });

  it('StrokeBuffer keeps the largest weight per cell, ascending ids', () => {
    const b = new StrokeBuffer();
    b.add({ cells: [5, 2], weights: [0.3, 0.9] }, 5);
    b.add({ cells: [5, 7], weights: [0.8, 0.1] }, 7);
    const { cells, weights } = b.toArrays();
    expect(Array.from(cells)).toEqual([2, 5, 7]);
    expect(Array.from(weights).map((x) => +x.toFixed(2))).toEqual([0.9, 0.8, 0.1]);
  });
});
