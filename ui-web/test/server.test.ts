// Part 9b-2 acceptance tests against the REAL wb-server and wb-snap binaries (no mocks).
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { WbApp, layerChoices, layerDecl, selectField } from 'wb-client';
import { repoRoot, startServer, type RunningServer } from '../../wb-client/test/harness';
import { MapController, viewLayers } from '../src/state/controller';
import { buildPixelCells, makeProjection } from '../src/viewport/projections';
import { paintPixels } from '../src/viewport/raster';
import { decodePng } from './png';

const snapBin = resolve(repoRoot, 'target/release', process.platform === 'win32' ? 'wb-snap.exe' : 'wb-snap');
const tmp = mkdtempSync(join(tmpdir(), 'wb-ui-'));

/** Renders the same field headlessly with wb-snap and with the browser pipeline; returns the share of in-map pixels that differ. */
async function parity(app: WbApp, args: string[], field: string, width = 720, forceColormap?: string): Promise<{ mismatch: number; inMap: number }> {
  const out = join(tmp, `${field}-${Math.random().toString(36).slice(2)}.png`);
  execFileSync(snapBin, ['--field', field, '--out', out, '--width', String(width), '--no-legend', ...args], { stdio: 'pipe' });
  const png = decodePng(readFileSync(out));
  const s = app.store.getState();
  const f = await app.ensureField(field);
  const proj = makeProjection(s.mesh!, width);
  expect([proj.width, proj.height]).toEqual([png.width, png.height]);
  const cells = buildPixelCells(s.mesh!, proj);
  const rgba = new Uint8ClampedArray(cells.length * 4);
  const decl = layerDecl(s.schema!, f);
  if (forceColormap) decl.colormap = forceColormap;
  paintPixels(cells, f, decl, { biomes: s.schema!.biomes }, rgba);
  let bad = 0, inMap = 0;
  for (let i = 0; i < cells.length; i++) {
    if (cells[i] < 0) continue;
    inMap++;
    for (let k = 0; k < 3; k++) if (Math.abs(rgba[4 * i + k] - png.data[png.channels * i + k]) > 2) { bad++; break; }
  }
  if (process.env.WB_DEBUG) console.log(`parity ${field}: ${inMap} px in map, ${bad} differ`);
  return { mismatch: bad / inMap, inMap };
}

describe('layer switching and schema-driven layers', () => {
  let srv: RunningServer;
  let app: WbApp;
  let ctl: MapController;
  beforeAll(async () => {
    srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2']);
    app = await WbApp.connect(srv.wsUrl);
    ctl = new MapController(app);
  });
  afterAll(async () => { ctl?.dispose(); app?.dispose(); await srv?.stop(); });

  it('picks a default layer from the schema on connect and fetches it', async () => {
    await app.idle();
    const s = app.store.getState();
    const first = layerChoices(s.schema, s.world).find((c) => c.available)!;
    expect(ctl.ui.getState().layer).toBe(first.field);
    await app.ensureField(first.field);
    expect(viewLayers(app.store.getState(), ctl.ui.getState())!.layers[0].field).toBe(first.field);
  });

  it('unsolved fields cannot be selected, and the attempt sends nothing', async () => {
    const solvedOnly = layerChoices(app.store.getState().schema, app.store.getState().world).find((c) => !c.available)!;
    const mark = app.client.sent.length;
    await ctl.selectLayer(solvedOnly.field);
    expect(app.client.sent.length).toBe(mark);
    expect(ctl.ui.getState().layer).not.toBe(solvedOnly.field);
  });

  it('solve reports progress events and then makes more layers available', async () => {
    const seen: number[] = [];
    const off = app.store.subscribe(() => { const j = app.store.getState().jobs.current; if (j?.status === 'running') seen.push(j.fraction); });
    expect(await app.solveAndWait()).toBe('completed');
    await app.idle();
    off();
    expect(seen.length).toBeGreaterThan(0);
    expect(app.store.getState().jobs.current?.fraction).toBe(1);
    expect(layerChoices(app.store.getState().schema, app.store.getState().world).filter((c) => c.available).length).toBeGreaterThan(1);
  });

  it('switching layers sends only get_field queries, never a command', async () => {
    const choices = layerChoices(app.store.getState().schema, app.store.getState().world).filter((c) => c.available).slice(0, 4);
    expect(choices.length).toBeGreaterThanOrEqual(3);
    for (const c of choices) await ctl.selectLayer(c.field); // warm the cache
    await app.idle();
    const mark = app.client.sent.length;
    for (const c of [...choices].reverse()) await ctl.selectLayer(c.field);
    await app.idle();
    expect(app.client.sent.slice(mark).filter((r) => r.kind === 'command')).toEqual([]);
    // a layer that was never fetched costs exactly one get_field
    const fresh = layerChoices(app.store.getState().schema, app.store.getState().world).find((c) => c.available && !selectField(app.store.getState(), c.field));
    if (fresh) {
      const m2 = app.client.sent.length;
      await ctl.selectLayer(fresh.field);
      const sent = app.client.sent.slice(m2);
      expect(sent.map((r) => r.type)).toEqual(['get_field']);
    }
  });

  it('a click reads one cell with a read-only get_cell query', async () => {
    const mark = app.client.sent.length;
    await ctl.pick(5);
    expect(app.client.sent.slice(mark).map((r) => `${r.kind}:${r.type}`)).toEqual(['query:get_cell']);
    expect(ctl.ui.getState().cellInfo?.cell).toBe(5);
  });

  it('every map pixel colour equals the headless PNG (sphere, elevation and solved fields)', async () => {
    if (!existsSync(snapBin)) throw new Error(`wb-snap not built: ${snapBin} (cargo build --release -p wb-app)`);
    for (const field of ['elevation', 'temperature', 'biome']) {
      const r = await parity(app, ['--preset', 'earthlike', '--param', 'resolution=2'], field);
      expect(r.inMap, field).toBeGreaterThan(100000);
      expect(r.mismatch, `${field}: share of differing pixels`).toBeLessThan(0.01);
    }
  });

  it('negative control: the comparison does fail when the colours are wrong', async () => {
    const r = await parity(app, ['--preset', 'earthlike', '--param', 'resolution=2'], 'elevation', 720, 'speed');
    expect(r.mismatch).toBeGreaterThan(0.3);
  });
});

describe('projections on every shape', () => {
  for (const [name, shape] of [['cube', 1], ['plane', 2]] as const) {
    it(`${name}: browser raster matches the headless PNG`, async () => {
      const srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2', '--param', `shape=${shape}`]);
      const app = await WbApp.connect(srv.wsUrl);
      try {
        expect(app.store.getState().mesh!.kind).toBe(name);
        const r = await parity(app, ['--preset', 'earthlike', '--param', 'resolution=2', '--param', `shape=${shape}`], 'elevation');
        expect(r.inMap).toBeGreaterThan(10000);
        expect(r.mismatch).toBeLessThan(0.01);
      } finally { app.dispose(); await srv.stop(); }
    });
  }
});
