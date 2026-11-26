// Part 9b-1 acceptance tests for wb-client, run under Node against a running wb-server.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { ApiClientError, WbApp, WbClient, defaultLayer, layerChoices, layerDecl, legendFor, fieldKey, selectField } from '../src';
import { repoRoot, startServer, type RunningServer } from './harness';

let srv: RunningServer;
let app: WbApp;

beforeAll(async () => {
  srv = await startServer();
  app = await WbApp.connect(srv.wsUrl);
});
afterAll(async () => {
  app?.dispose();
  await srv?.stop();
});

describe('connect and load', () => {
  it('connects, checks versions and loads schema, world and mesh', () => {
    const s = app.store.getState();
    expect(s.connection).toBe('open');
    expect(app.client.hello.api_version).toBe(1);
    expect(s.schema!.fields.length).toBeGreaterThan(10);
    expect(s.world!.domain.kind).toBe('sphere');
    expect(s.world!.preset).toBe('custom'); // started with --param resolution=2, so it is no longer the pure preset
    const m = s.mesh!;
    expect(m.nCells).toBe(s.world!.domain.n_cells);
    expect(m.centroid.length).toBe(3 * m.nCells);
    expect(m.adjOffsets.length).toBe(m.nCells + 1);
    expect(m.adjOffsets[m.nCells]).toBe(m.adj.length);
    expect(m.meshId).toBe(s.world!.domain.mesh_id);
  });

  it('mesh adjacency is symmetric (the CSR arrays survived the wire intact)', () => {
    const m = app.store.getState().mesh!;
    const has = (a: number, b: number) => {
      for (let k = m.adjOffsets[a]; k < m.adjOffsets[a + 1]; k++) if (m.adj[k] === b) return true;
      return false;
    };
    for (let a = 0; a < m.nCells; a++) for (let k = m.adjOffsets[a]; k < m.adjOffsets[a + 1]; k++) expect(has(m.adj[k], a)).toBe(true);
  });

  it('rejects a server it cannot talk to', async () => {
    await expect(WbClient.connect('ws://127.0.0.1:1/ws')).rejects.toBeInstanceOf(ApiClientError);
  });
});

describe('load preset, solve, receive fields, switch layer', () => {
  it('loads a preset and sees the world change', async () => {
    await app.command({ type: 'load_preset', name: 'desert' });
    expect(app.store.getState().world!.preset).toBe('desert');
    expect(app.store.getState().world!.can_undo).toBe(true);
  });

  it('the elevation layer works before any solve; other fields are not available yet', async () => {
    const choices = layerChoices(app.store.getState().schema, app.store.getState().world);
    expect(defaultLayer(choices)).toBe('elevation');
    expect(choices.find((c) => c.field === 'temperature')!.available).toBe(false);
    await expect(app.client.getField('temperature')).rejects.toMatchObject({ code: 'not_solved' });
    const e = await app.ensureField('elevation');
    expect(e.values.length).toBe(app.store.getState().mesh!.nCells);
    expect(e.meta.colormap).toBe('terrain');
  });

  it('solves with progress events and then serves every solved field', async () => {
    const seen: number[] = [];
    const off = app.client.onEvent((e) => e.type === 'job_progress' && seen.push(e.fraction));
    expect(await app.solveAndWait()).toBe('completed');
    off();
    expect(seen.length).toBeGreaterThan(2);
    expect(Math.max(...seen)).toBeGreaterThan(0.9);
    await app.idle();
    const s = app.store.getState();
    expect(s.jobs.current).toMatchObject({ status: 'completed', fraction: 1 });
    expect(s.world!.solved).toBe(true);
    const t = await app.ensureField('temperature');
    expect(t.values.length).toBe(s.mesh!.nCells);
    expect(t.stale).toBe(false);
    expect(t.min).toBeGreaterThan(150);
    expect(t.max).toBeLessThan(400);
    expect(t.min).toBeLessThan(t.max);
  });

  it('switching layers sends ONLY get_field queries, and cached layers send nothing', async () => {
    await app.ensureField('temperature'); // make sure it is cached
    const log = app.client.sent;
    const mark = log.length;
    await app.ensureField('precipitation');
    await app.ensureField('biome');
    await app.ensureField('wind_speed');
    const fresh = log.slice(mark);
    expect(fresh.map((r) => r.type)).toEqual(['get_field', 'get_field', 'get_field']);
    expect(fresh.every((r) => r.kind === 'query')).toBe(true);
    const mark2 = log.length;
    await app.ensureField('temperature');
    await app.ensureField('precipitation');
    expect(log.length).toBe(mark2); // nothing at all: served from the store
  });

  it('builds layer declarations, ranges and legends from the schema alone', async () => {
    const { schema } = app.store.getState();
    const t = selectField(app.store.getState(), 'temperature')!;
    const decl = layerDecl(schema!, t);
    expect(decl).toMatchObject({ field: 'temperature', kind: 'scalar', colormap: 'temperature', range: [220, 320], opacity: 1 });
    const lg = legendFor(schema!, decl, t);
    expect(lg).toMatchObject({ type: 'gradient', unit: 'degC' });
    if (lg.type === 'gradient') {
      expect(lg.lo).toBeCloseTo(220 - 273.15);
      expect(lg.loLabel.startsWith('-53')).toBe(true);
    }
    const b = selectField(app.store.getState(), 'biome')!;
    const bl = legendFor(schema!, layerDecl(schema!, b), b);
    expect(bl.type).toBe('categories');
    if (bl.type === 'categories') {
      expect(bl.items.length).toBeGreaterThan(2);
      expect(bl.items.every((i) => i.color !== null)).toBe(true); // biome palette from the schema
    }
    const auto = selectField(app.store.getState(), 'wind_speed')!; // no fixed range: data min/max
    expect(layerDecl(schema!, auto).range).toEqual([auto.min, auto.max]);
  });
});

describe('events keep the store honest', () => {
  it('a parameter change marks solved fields stale and refetches the watched ones', async () => {
    await app.ensureField('temperature');
    await app.client.setParam('solar_constant', 1234);
    await app.refreshWorld();
    await app.idle();
    expect(app.store.getState().world!.stale).toBe(true);
    const t = selectField(app.store.getState(), 'temperature')!;
    expect(t.stale).toBe(true); // refetched after the field_changed event, now flagged stale
    await app.client.undo();
    await app.refreshWorld();
  });

  it('an elevation change refetches the watched elevation without being asked', async () => {
    const before = (await app.ensureField('elevation')).version;
    await app.command({ type: 'set_param', name: 'seed', value: 4242 });
    await app.idle();
    const after = selectField(app.store.getState(), 'elevation')!;
    expect(after.version).toBeGreaterThan(before);
  });

  it('a mesh change (resolution) delivers a new mesh through world_loaded', async () => {
    const old = app.store.getState().mesh!;
    const res = app.store.getState().world!.params.resolution;
    await app.command({ type: 'set_param', name: 'resolution', value: res + 1 });
    await app.idle();
    const s = app.store.getState();
    expect(s.mesh!.meshId).not.toBe(old.meshId);
    expect(s.mesh!.nCells).toBeGreaterThan(old.nCells);
    expect(s.world!.domain.mesh_id).toBe(s.mesh!.meshId);
    expect(selectField(s, 'elevation')!.values.length).toBe(s.mesh!.nCells);
    expect(Object.keys(s.fields).every((k) => s.fields[k].meshId === s.mesh!.meshId)).toBe(true);
    await app.command({ type: 'set_param', name: 'resolution', value: res });
    await app.idle();
    expect(app.store.getState().mesh!.meshId).toBe(old.meshId);
  });
});

describe('errors', () => {
  it('typed errors reject with their code and leave the connection usable', async () => {
    await expect(app.client.getField('no_such_field')).rejects.toMatchObject({ code: expect.stringMatching(/unknown_field|not_solved/) });
    await expect(app.client.setParam('seed', -5)).rejects.toMatchObject({ code: 'bad_param' });
    await expect(app.client.loadPreset('nope')).rejects.toMatchObject({ code: 'unknown_preset' });
    await expect(app.client.getCell(10_000_000)).rejects.toMatchObject({ code: 'bad_cell' });
    expect((await app.client.getWorldInfo()).api_version).toBe(1);
  });

  it('pending requests reject when the connection drops', async () => {
    const c = await WbClient.connect(srv.wsUrl);
    const p = c.getSchema();
    c.close();
    await expect(p).rejects.toBeInstanceOf(ApiClientError);
    await expect(c.getWorldInfo()).rejects.toMatchObject({ code: 'disconnected' });
  });
});

describe('contract', () => {
  const golden = (n: string) => JSON.parse(readFileSync(resolve(repoRoot, 'wb-app/tests/golden', n), 'utf8'));
  const keys = (o: object) => Object.keys(o).sort();

  it('live JSON has the same top-level keys as the 9a golden files', async () => {
    const live = await app.client.getSchema();
    const g = golden('schema.json');
    expect(keys(live)).toEqual(keys(g));
    const info = await app.client.getWorldInfo();
    expect(keys(info)).toEqual(keys(golden('world_info.json')));
    expect(keys(live.fields[0])).toEqual(keys(g.fields[0]));
    expect(keys(live.params[0])).toEqual(keys(g.params[0]));
  });

  it('event and ack shapes match the TypeScript types', async () => {
    const ack = await app.client.setParam('solar_constant', 1111);
    expect(Object.keys(ack).sort()).toEqual(['api_version', 'command', 'invalidated', 'job']);
    await app.client.undo();
  });
});
