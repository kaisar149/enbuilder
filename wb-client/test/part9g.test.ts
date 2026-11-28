// Part 9g acceptance tests for wb-client: playhead / compare models (pure) and the DiffController against the REAL wb-server.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { DiffController, WbApp, advancePlayhead, compareValues, coreLinkLines, explainRows, frameDelta, revealedCount, stepLine, symmetricRange, sweepCoord } from '../src';
import type { DiffData, MeshData } from '../src';
import { startServer, type RunningServer } from './harness';

const fakeDiff = (): DiffData => ({
  field: 'x', meshId: 'm', from: 1, to: 2, nCells: 5, kind: 'scalar', noChange: false, frontKind: 'edit', minRel: 0.01, hiddenCells: 0, changedCellsExact: 3,
  summary: { changed_cells: 3, peak_abs: 4, mean_abs: 2, changed_area: 0, global_mean_shift: 0, reach_km: 1, edit_cells: 1 },
  meta: { unit: '', display: { scale: 1, offset: 0, unit: '' }, colormap: 'viridis', range: null },
  delta: Float32Array.from([0, 4, -2, 0, 1]), before: Float32Array.from([1, 1, 1, 1, 1]), after: Float32Array.from([1, 5, -1, 1, 2]),
  front: Float32Array.from([-1, 0, 0.5, -1, 1]), distKm: Float32Array.from([-1, 0, 5, -1, 9]),
});

describe('9g playhead and compare models (pure)', () => {
  it('frame shows exactly the cells with front <= t, t = 1 shows all changes, t = 0 only the first', () => {
    const d = fakeDiff();
    expect([...frameDelta(d, 0)]).toEqual([0, 4, 0, 0, 0]);
    expect([...frameDelta(d, 0.5)]).toEqual([0, 4, -2, 0, 0]);
    expect([...frameDelta(d, 1)]).toEqual([...d.delta]);
    expect(revealedCount(d, 0.4)).toBe(1);
    expect(revealedCount(d, 1)).toBe(3);
  });
  it('the playhead is monotone, ends at 1, and is a plain number (so it survives a viewport swap)', () => {
    let t = 0, last = -1;
    for (let i = 0; i < 100 && t < 1; i++) { const r = advancePlayhead(t, 50, 2000); expect(r.t).toBeGreaterThanOrEqual(last); last = t = r.t; }
    expect(t).toBe(1);
    expect(advancePlayhead(1, 50, 2000).done).toBe(true);
  });
  it('range is symmetric around zero and the slider scales it', () => {
    const d = fakeDiff();
    expect(symmetricRange(d)).toEqual([-4, 4]);
    expect(symmetricRange(d, 0.25)).toEqual([-1, 1]);
    expect(symmetricRange({ ...d, summary: { ...d.summary, peak_abs: 0 } })).toEqual([-1, 1]);
  });
  it('swipe: cells left of the handle show after, the rest before; flip modes return the stored arrays', () => {
    const d = fakeDiff();
    const mesh: MeshData = { meshId: 'm', kind: 'plane', nCells: 5, centroid: Float32Array.from([0, 0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0, 4, 0, 0]), area: new Float32Array(5), adjOffsets: new Uint32Array(6), adj: new Uint32Array(0) };
    const u = sweepCoord(mesh);
    expect([...u]).toEqual([0, 0.25, 0.5, 0.75, 1]);
    expect([...compareValues(d, 'swipe', u, 0.5)]).toEqual([1, 5, 1, 1, 1].map((v, i) => (u[i] < 0.5 ? d.after[i] : d.before[i])));
    expect(compareValues(d, 'before')).toBe(d.before);
    expect(compareValues(d, 'after')).toBe(d.after);
  });
});

describe('9g DiffController (real wb-server)', () => {
  let srv: RunningServer;
  let app: WbApp;
  let dc: DiffController;
  let edited = -1;

  beforeAll(async () => {
    srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2', '--param', 'spinup_years=1']);
    app = await WbApp.connect(srv.wsUrl);
    dc = new DiffController(app);
    expect(await app.solveAndWait()).toBe('completed');
    await app.idle();
    // a ridge on the land cell with most land neighbours, then a refined refresh
    const z = (await app.client.getField('elevation')).values;
    const mesh = app.store.getState().mesh!;
    let bs = -1;
    for (let i = 0; i < z.length; i++) {
      if (z[i] <= 0) continue;
      let s = 0;
      for (let k = mesh.adjOffsets[i]; k < mesh.adjOffsets[i + 1]; k++) if (z[mesh.adj[k]] > 0) s++;
      if (s > bs) { bs = s; edited = i; }
    }
    const cells = [edited, ...Array.from(mesh.adj.slice(mesh.adjOffsets[edited], mesh.adjOffsets[edited + 1]))];
    await app.command({ type: 'paint_stroke', layer: 'elevation', op: 'add', cells, weights: cells.map((_, i) => (i === 0 ? 1 : 0.6)), value: 3000, strength: 1 });
    expect(await app.refreshAndWait('refined')).toBe('completed');
    await app.idle();
    await dc.refreshVersions();
  }, 120000);
  afterAll(async () => { dc?.dispose(); app?.dispose(); await srv?.stop(); });

  it('lists versions and defaults the pair to (previous, latest)', () => {
    const s = dc.state;
    expect(s.versions!.versions.length).toBeGreaterThanOrEqual(2);
    const ids = s.versions!.versions.map((v) => v.version);
    expect(s.to).toBe(ids[ids.length - 1]);
    expect(s.from).toBe(ids[ids.length - 2]);
  });

  it('diff arrays decode through the layout table; delta = after - before; the edited cell reveals first', async () => {
    const d = (await dc.loadDiff('elevation'))!;
    expect(d.field).toBe('elevation');
    expect(d.nCells).toBe(app.store.getState().mesh!.nCells);
    expect(d.noChange).toBe(false);
    for (let i = 0; i < d.nCells; i++) expect(Math.abs(d.after[i] - d.before[i] - d.delta[i])).toBeLessThan(1e-2);
    expect(d.front[edited]).toBeLessThanOrEqual(0.05);
    expect(d.distKm[edited]).toBe(0);
    expect(revealedCount(d, 1)).toBe(d.summary.changed_cells);
    // cached: the second call does not hit the server again
    const n = app.client.sent.filter((r) => r.type === 'get_diff').length;
    await dc.loadDiff('elevation');
    expect(app.client.sent.filter((r) => r.type === 'get_diff').length).toBe(n);
  });

  it('a version against itself is "no change"', async () => {
    const v = dc.state.to!;
    dc.select(v, v);
    const d = (await dc.loadDiff('temperature'))!;
    expect(d.noChange).toBe(true);
    expect(revealedCount(d, 1)).toBe(0);
    const sum = (await dc.loadSummary())!;
    expect(sum.no_change).toBe(true);
    await dc.refreshVersions();
    const ids = dc.state.versions!.versions.map((x) => x.version);
    dc.followLatest();
    await new Promise((r) => setTimeout(r, 200));
  });

  it('explanation rows are the core chain, one row per step, in the core line format', async () => {
    const d = (await dc.loadDiff('precipitation'))!;
    const far = [...d.delta.keys()].filter((i) => d.delta[i] !== 0).sort((a, b) => d.distKm[b] - d.distKm[a])[0];
    const ex = (await dc.explain(far, 'precipitation'))!;
    expect(ex.changed).toBe(true);
    const rows = explainRows(ex);
    expect(rows.length).toBe(ex.steps.length);
    const core = coreLinkLines(ex.text);
    expect(core.length).toBe(rows.length);
    rows.forEach((r, i) => { expect(core[i].startsWith(r.line)).toBe(true); expect(r.highlight[0]).toBe(ex.steps[i].cell); });
    expect(stepLine(ex.steps[0])).toBe(rows[0].line);
    expect(ex.steps[0].field).toBe('precipitation');
  });

  it('the list follows new solves (a third version appears after another refresh)', async () => {
    const before = dc.state.versions!.versions.length;
    expect(await app.refreshAndWait('preview')).toBe('completed');
    await app.idle();
    for (let i = 0; i < 40 && dc.state.versions!.versions.length === before; i++) await new Promise((r) => setTimeout(r, 50));
    expect(dc.state.versions!.versions.length).toBeGreaterThan(before);
  });
});
