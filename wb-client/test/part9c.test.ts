// Part 9c acceptance tests for wb-client: pure logic (no server) and the real wb-server over a WebSocket.
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import {
  ParamController, WbApp, debounce, fmtParam, historyRedo, historyUndo, paramGroups, presetStatus, presetTarget, randomSeed,
  seasonLabel, seasonsAvailable, seasonVariant, type Schema, type WorldInfo,
} from '../src';
import { startServer, type RunningServer } from './harness';

const def = (name: string, group: string, extra: object = {}) => ({
  name, group, unit: '', min: 0, max: 10, default: 1, widget: 'slider', scope: 'climate', description: '', identity: false, ...extra,
});
const fakeSchema = (): Schema => ({
  api_version: 1,
  params: [def('a', 'laws'), def('seed', 'world', { widget: 'integer', max: 1000, identity: true }), def('b', 'solver')] as any,
  presets: [{ name: 'base', description: '', params: {} }, { name: 'hot', description: '', params: { a: 5 } }],
  modules: [], loops: [], fields: [], biomes: [], colormaps: [],
  groups: [{ name: 'laws', title: 'Laws', advanced: false }, { name: 'world', title: 'World', advanced: false }, { name: 'solver', title: 'Solver', advanced: true }],
  seasons: { count: 12, variants: { temperature: 'season_temperature' } },
});

describe('pure helpers', () => {
  it('debounce sends one call after the last of many edits, and supports flush and cancel', () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = debounce(fn, 100);
    for (let i = 0; i < 50; i++) { d(i); vi.advanceTimersByTime(10); }
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith(49);
    d(1); d.flush();
    expect(fn).toHaveBeenCalledTimes(2);
    d(2); d.cancel(); vi.advanceTimersByTime(500);
    expect(fn).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it('a parameter added to the schema appears in the groups with no client change (known group and unknown group)', () => {
    const s = fakeSchema();
    s.params.push(def('dummy_known', 'laws') as any, def('dummy_new', 'brand_new_group') as any);
    const g = paramGroups(s);
    expect(g.map((x) => x.group)).toEqual(['laws', 'world', 'solver', 'brand_new_group']);
    expect(g[0].params.map((p) => p.name)).toEqual(['a', 'dummy_known']);
    expect(g[2].advanced).toBe(true);
    expect(g[3]).toMatchObject({ title: 'Brand new group', advanced: false });
  });

  it('preset status ignores identity parameters and lists real differences', () => {
    const s = fakeSchema();
    expect(presetTarget(s, 'hot')).toEqual({ a: 5, seed: 1, b: 1 });
    expect(presetStatus(s, { a: 5, seed: 1, b: 1 }, 'hot').dirty).toBe(false);
    expect(presetStatus(s, { a: 5, seed: 999, b: 1 }, 'hot').dirty).toBe(false);
    const st = presetStatus(s, { a: 6, seed: 1, b: 1 }, 'custom', 'hot');
    expect(st).toEqual({ base: 'hot', dirty: true, diffs: [{ name: 'a', current: 6, expected: 5 }] });
    expect(presetStatus(s, { a: 5, seed: 1, b: 1 }, 'custom').base).toBe('hot'); // closest preset when nothing else is known
  });

  it('formats parameters and seasons', () => {
    expect(fmtParam(def('x', 'g', { min: 0, max: 90 }) as any, 23.4)).toBe('23.4');
    expect(fmtParam(def('x', 'g', { min: 0, max: 5000 }) as any, 365.25)).toBe('365');
    expect(seasonLabel(0, 12, 360)).toBe('Season 1 of 12 (day 15)');
    expect(randomSeed(fakeSchema(), () => 0.5)).toBe(500);
    expect(seasonVariant(fakeSchema(), 'temperature')).toBe('season_temperature');
    expect(seasonVariant(fakeSchema(), 'wind')).toBeNull();
  });

  it('seasons are only offered when the world has an orbit and the layers were solved', () => {
    const s = fakeSchema();
    const w = { params: { year_length_days: 365 }, fields: [{ name: 'season_temperature', layers: 12 }] } as unknown as WorldInfo;
    expect(seasonsAvailable(s, w, 'temperature')).toBe(true);
    expect(seasonsAvailable(s, { ...w, params: { year_length_days: 0 } } as any, 'temperature')).toBe(false);
    expect(seasonsAvailable(s, { ...w, fields: [] } as any, 'temperature')).toBe(false);
    expect(seasonsAvailable(s, w, 'precipitation')).toBe(false);
  });
});

describe('against the real wb-server', () => {
  let srv: RunningServer;
  let app: WbApp;
  let pc: ParamController;
  const sent = (type: string) => app.client.sent.filter((r) => r.type === type);
  const world = () => app.store.getState().world!;

  beforeAll(async () => {
    srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2', '--param', 'spinup_years=1']);
    app = await WbApp.connect(srv.wsUrl);
    pc = new ParamController(app, { debounceMs: 40 });
  });
  afterAll(async () => {
    pc?.dispose();
    app?.dispose();
    await srv?.stop();
  });

  it('schema has groups, identity flags and seasons from the core', () => {
    const schema = app.store.getState().schema!;
    const g = paramGroups(schema);
    expect(g.map((x) => x.group)).toEqual(['laws', 'world', 'solver']);
    expect(g.find((x) => x.group === 'solver')!.advanced).toBe(true);
    expect(schema.params.filter((p) => p.identity).map((p) => p.name)).toEqual(['shape', 'resolution', 'radius_km', 'seed']);
    expect(schema.seasons!.count).toBe(12);
  });

  it('a slider drag sends ONE set_param and no solve', async () => {
    const before = sent('set_param').length;
    for (let v = 20; v <= 60; v += 2) pc.edit('axial_tilt_deg', v); // 21 pointer moves
    expect(sent('set_param').length).toBe(before); // nothing yet
    await new Promise((r) => setTimeout(r, 150));
    await pc.flush();
    const mine = sent('set_param').slice(before);
    expect(mine.length).toBe(1);
    expect((mine[0].body as any).value).toBe(60);
    expect(sent('solve').length).toBe(0);
    expect(world().params.axial_tilt_deg).toBe(60);
    expect(pc.ui.getState().drafts).toEqual({});
  });

  it('an invalid value is rejected by the core and shown, not clamped', async () => {
    pc.edit('axial_tilt_deg', 120);
    expect(await pc.commit('axial_tilt_deg')).toBe(false);
    expect(pc.ui.getState().errors.axial_tilt_deg).toContain('axial_tilt_deg');
    expect(pc.ui.getState().drafts.axial_tilt_deg).toBe(120); // still what the user typed
    expect(pc.value('axial_tilt_deg')).toBe(120);
    expect(world().params.axial_tilt_deg).toBe(60); // unchanged in the core
    expect(app.store.getState().lastError).toBeNull();
    pc.edit('axial_tilt_deg', 45); // fixing the value clears the message
    expect(pc.ui.getState().errors.axial_tilt_deg).toBeUndefined();
    await pc.commit('axial_tilt_deg');
    expect(world().params.axial_tilt_deg).toBe(45);
    pc.edit('axial_tilt_deg', Number.NaN);
    expect(pc.ui.getState().errors.axial_tilt_deg).toContain('number');
    pc.discard();
  });

  it('presets, dirty indicator, undo and redo across several changes', async () => {
    await pc.applyPreset('earthlike');
    expect(presetStatus(app.store.getState().schema, world().params, world().preset, pc.ui.getState().basePreset).dirty).toBe(false);
    pc.edit('axial_tilt_deg', 60); await pc.commit('axial_tilt_deg');
    pc.edit('surface_gravity', 5); await pc.commit('surface_gravity');
    let st = presetStatus(app.store.getState().schema, world().params, world().preset, pc.ui.getState().basePreset);
    expect(st.base).toBe('earthlike');
    expect(st.diffs.map((d) => d.name).sort()).toEqual(['axial_tilt_deg', 'surface_gravity']);
    pc.edit('seed', 123); await pc.commit('seed'); // identity: not dirty
    expect(presetStatus(app.store.getState().schema, world().params, world().preset, 'earthlike').diffs.length).toBe(2);
    await pc.applyPreset('waterworld');
    expect(world().params.ocean_fraction).toBe(0.97);
    expect(world().params.axial_tilt_deg).toBe(23.4); // a preset resets everything it does not mention
    const labels = historyUndo(world()).map((h) => h.label);
    expect(labels.slice(-4)).toEqual(['axial_tilt_deg: 23.4 -> 60', 'surface_gravity: 9.81 -> 5', 'seed: 7 -> 123', 'preset waterworld']);
    await pc.undo(); // preset
    expect(world().params.ocean_fraction).toBe(0.7);
    expect(world().params.surface_gravity).toBe(5);
    await pc.undo(); // seed
    await pc.undo(); // gravity
    expect(world().params.surface_gravity).toBe(9.81);
    expect(world().params.axial_tilt_deg).toBe(60);
    expect(historyRedo(world())[0].label).toBe('surface_gravity: 9.81 -> 5');
    await pc.redo();
    expect(world().params.surface_gravity).toBe(5);
    await pc.redo();
    expect(world().params.seed).toBe(123);
    await pc.redo(); // the preset again: it resets what it does not mention (gravity, seed)
    expect(world().params.surface_gravity).toBe(9.81);
    expect(world().params.seed).toBe(7);
    expect(world().params.ocean_fraction).toBe(0.97);
    expect(world().can_redo).toBe(false);
    await pc.applyPreset('earthlike');
  });

  it('reset restores the default of one parameter and of a whole group', async () => {
    pc.edit('solar_constant', 1500); await pc.commit('solar_constant');
    pc.edit('axial_tilt_deg', 50); await pc.commit('axial_tilt_deg');
    await pc.reset('solar_constant');
    expect(world().params.solar_constant).toBe(1361);
    expect(world().params.axial_tilt_deg).toBe(50);
    await pc.resetGroup('laws');
    expect(world().params.axial_tilt_deg).toBe(23.4);
  });

  it('solve() flushes pending edits first, then solves once', async () => {
    pc.edit('axial_tilt_deg', 40);
    const before = app.client.sent.length;
    await pc.solve();
    const mine = app.client.sent.slice(before).filter((r) => r.kind === 'command').map((r) => r.type);
    expect(mine).toEqual(['set_param', 'solve']);
    expect(await app.waitJob(app.store.getState().jobs.current!.id)).toBe('completed');
    await app.idle();
  });

  it('season layers switch the displayed values without a new solve', async () => {
    const schema = app.store.getState().schema!;
    expect(seasonsAvailable(schema, world(), 'temperature')).toBe(true);
    const solves = sent('solve').length;
    const a = await app.ensureField('season_temperature', 0);
    const b = await app.ensureField('season_temperature', 6);
    expect(a.layers).toBe(12);
    expect(Array.from(a.values)).not.toEqual(Array.from(b.values));
    expect(sent('solve').length).toBe(solves);
  });

  it('six variations differ, are reproducible from their seed, and a mesh change drops them', async () => {
    await pc.applyPreset('earthlike');
    pc.edit('spinup_years', 1); await pc.commit('spinup_years');
    expect(await app.generateVariations(6)).toBe('completed');
    const v = app.store.getState().variations!;
    expect(v.items.length).toBe(6);
    expect(new Set(v.items.map((i) => i.seed)).size).toBe(6);
    const thumbs = await Promise.all(v.items.map((i) => app.ensureVariation(i.index, 'biome')));
    const distinct = new Set(thumbs.map((t) => Array.from(t.values).join(',')));
    expect(distinct.size).toBeGreaterThan(1);
    expect(thumbs[3].seed).toBe(v.items[3].seed);
    // open variation 3: its seed is set and the solve gives exactly the thumbnail
    const seed = await pc.openVariation(3);
    expect(world().params.seed).toBe(seed);
    expect(await app.waitJob(app.store.getState().jobs.current!.id)).toBe('completed');
    await app.idle();
    const solved = await app.ensureField('biome');
    expect(Array.from(solved.values)).toEqual(Array.from(thumbs[3].values));
    // a mesh parameter change drops the batch on both sides
    pc.edit('resolution', 1); await pc.commit('resolution');
    expect(app.store.getState().variations).toBeNull();
    pc.edit('resolution', 2); await pc.commit('resolution');
  });
});
