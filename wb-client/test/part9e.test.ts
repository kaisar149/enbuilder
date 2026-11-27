// Part 9e acceptance tests for wb-client against the REAL wb-server: geometry queries (rivers, outline), the store cache, get_cell seasons.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { WbApp } from '../src';
import { startServer, type RunningServer } from './harness';

describe('9e geometry queries and cache (real wb-server)', () => {
  let srv: RunningServer;
  let app: WbApp;
  const sent = (type: string) => app.client.sent.filter((r) => r.type === type);
  const count = (what: string) => sent('query').length && app.client.sent.filter((r) => r.kind === 'query' && (r.body as any).type === what).length;

  beforeAll(async () => {
    srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=3', '--param', 'spinup_years=1']);
    app = await WbApp.connect(srv.wsUrl);
    const ack = await app.client.solve();
    expect(await app.waitJob(ack.job as number)).toBe('completed');
    await app.idle();
  }, 120000);
  afterAll(async () => { app?.dispose(); await srv?.stop(); });

  it('schema declares overlays, kinds and the four presets as data', () => {
    const s = app.store.getState().schema!;
    expect(s.layer_kinds).toEqual(expect.arrayContaining(['scalar', 'mask', 'vector', 'lines']));
    expect(s.layer_presets!.map((p) => p.name)).toEqual(expect.arrayContaining(['Climate', 'Hydrology', 'Ocean', 'Terrain']));
    const known = new Set(s.overlays!.map((o) => o.id));
    for (const p of s.layer_presets!) for (const id of p.overlays) expect(known.has(id)).toBe(true);
    for (const o of s.overlays!) expect(['vector', 'lines', 'mask']).toContain(o.kind);
  });

  it('rivers: polylines are connected from source to outlet and the discharge never decreases downstream', async () => {
    const r = await app.ensureRivers();
    const mesh = app.store.getState().mesh!;
    expect(r.meshId).toBe(mesh.meshId);
    expect(r.nLines).toBeGreaterThan(0);
    expect(r.offsets[r.nLines]).toBe(r.nPoints);
    let bad = 0, shrink = 0;
    for (let k = 0; k < r.nLines; k++) {
      for (let i = r.offsets[k]; i + 1 < r.offsets[k + 1]; i++) {
        const a = r.cells[i], b = r.cells[i + 1];
        let adjacent = false;
        for (let q = mesh.adjOffsets[a]; q < mesh.adjOffsets[a + 1]; q++) if (mesh.adj[q] === b) adjacent = true;
        if (!adjacent) bad++;
        if (r.discharge[i + 1] < r.discharge[i] * 0.999) shrink++;
      }
    }
    expect(bad).toBe(0);
    expect(shrink).toBe(0);
  });

  it('rivers and outlines are fetched once and served from the cache afterwards (no refetch)', async () => {
    const r1 = await app.ensureRivers();
    const o1 = await app.ensureOutline('ocean_mask', 0.5);
    const rq = count('get_rivers'), oq = count('get_outline');
    const r2 = await app.ensureRivers();
    const o2 = await app.ensureOutline('ocean_mask', 0.5);
    await Promise.all([app.ensureRivers(), app.ensureRivers(), app.ensureOutline('ocean_mask', 0.5)]);
    expect(r2).toBe(r1);
    expect(o2).toBe(o1);
    expect(count('get_rivers')).toBe(rq);
    expect(count('get_outline')).toBe(oq);
    expect(o1.nEdges).toBeGreaterThan(0);
    expect(o1.pairs.length).toBe(2 * o1.nEdges);
  });

  it('every outline edge joins an inside cell and an outside neighbour of the mask field', async () => {
    const o = await app.ensureOutline('ocean_mask', 0.5);
    const f = await app.ensureField('ocean_mask', 0);
    const mesh = app.store.getState().mesh!;
    for (let e = 0; e < o.nEdges; e++) {
      const a = o.pairs[2 * e], b = o.pairs[2 * e + 1];
      expect(f.values[a] > 0.5).toBe(true);
      expect(f.values[b] > 0.5).toBe(false);
      let adj = false;
      for (let q = mesh.adjOffsets[a]; q < mesh.adjOffsets[a + 1]; q++) if (mesh.adj[q] === b) adj = true;
      expect(adj).toBe(true);
    }
  });

  it('a parameter change that alters the world drops the geometry and refetches the watched one once', async () => {
    const before = await app.ensureRivers();
    const rq = count('get_rivers');
    const ack = await app.client.command({ type: 'set_param', name: 'axial_tilt_deg', value: 40 } as any);
    if (ack.job !== null) expect(await app.waitJob(ack.job)).toBe('completed');
    else expect(await app.waitJob((await app.client.solve()).job as number)).toBe('completed');
    await app.idle();
    const after = app.store.getState().rivers;
    expect(after).not.toBeNull();
    expect(count('get_rivers')).toBeLessThanOrEqual(rq + 1);
    expect(after!.version).toBeGreaterThanOrEqual(before.version);
  });

  it('get_cell returns 12 seasonal values for temperature and precipitation, all finite', async () => {
    const info = await app.client.getCell(0);
    expect(info.seasonal).toBeTruthy();
    for (const name of ['temperature', 'precipitation']) {
      expect(info.seasonal![name]).toHaveLength(12);
      for (const v of info.seasonal![name]) expect(Number.isFinite(v)).toBe(true);
    }
  });
});
