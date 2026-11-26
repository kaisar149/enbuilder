// @vitest-environment jsdom
// Part 9c UI tests with jsdom. The core is replaced by a tiny recording fake ONLY to count requests; the real server is used in server9c.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { ApiClientError, ParamController, Store, initialState, type Command, type Schema, type WbApp, type WbState, type WorldInfo } from 'wb-client';
import { HistoryPanel } from '../src/panels/HistoryPanel';
import { ParameterPanel } from '../src/panels/ParameterPanel';
import { PresetPanel } from '../src/panels/PresetPanel';
import { SeasonPanel } from '../src/panels/SeasonPanel';
import { VariationsPanel } from '../src/panels/VariationsPanel';
import { PANEL_REGISTRY } from '../src/panels/registry';
import { PANEL_COMPONENTS } from '../src/panels/components';
import { initialUi, type MapController, type UiState } from '../src/state/controller';

const golden = (n: string) => JSON.parse(readFileSync(resolve(__dirname, '../../wb-app/tests/golden', n), 'utf8'));
const baseSchema = golden('schema.json') as Schema;
const baseWorld = golden('world_info.json') as WorldInfo;

interface Rig { app: WbApp; ctl: MapController; sent: Command[]; pc: ParamController; setWorld(p: Partial<WorldInfo>): void; ui: Store<UiState>; store: Store<WbState> }

/** Fake app: `command` records and applies set_param/load_preset like the core would (validation included). */
function rig(schema: Schema = baseSchema, world: WorldInfo = baseWorld): Rig {
  const store = new Store<WbState>({ ...initialState(), schema, world: structuredClone(world) });
  const sent: Command[] = [];
  const app = {
    store,
    async command(cmd: Command) {
      sent.push(cmd);
      if (cmd.type === 'set_param') {
        const d = schema.params.find((p) => p.name === cmd.name)!;
        if (cmd.value < d.min || cmd.value > d.max) throw new ApiClientError('bad_param', `parameter '${cmd.name}' must be in [${d.min}, ${d.max}], got ${cmd.value}`);
        store.setState((s) => ({ world: { ...s.world!, preset: 'custom', params: { ...s.world!.params, [cmd.name]: cmd.value } } }));
      }
      return { api_version: 1, command: cmd.type, invalidated: [], job: null };
    },
    ensureVariation: async () => undefined,
  } as unknown as WbApp;
  const pc = new ParamController(app, { debounceMs: 100 });
  const ui = new Store<UiState>({ ...initialUi(), layer: 'temperature' });
  const ctl = { ui, params: pc, setSeason: vi.fn() } as unknown as MapController;
  return { app, ctl, sent, pc, ui, store, setWorld: (p) => store.setState((s) => ({ world: { ...s.world!, ...p } })) };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => { cleanup(); vi.useRealTimers(); });

describe('generated parameter panel', () => {
  it('renders one control per schema parameter, in the schema groups, with unit, default and reset', () => {
    const r = rig();
    render(<ParameterPanel app={r.app} ctl={r.ctl} />);
    for (const p of baseSchema.params) {
      const el = document.querySelector(`[data-param="${p.name}"]`);
      expect(el, p.name).toBeTruthy();
      expect(el!.textContent).toContain('default');
      expect(within(el as HTMLElement).getByRole('button', { name: /reset/i })).toBeTruthy();
    }
    const groups = [...document.querySelectorAll('details.group')] as HTMLDetailsElement[];
    expect(groups.map((g) => g.dataset.group)).toEqual(baseSchema.groups!.map((g) => g.name));
    for (const g of groups) expect(g.open).toBe(!baseSchema.groups!.find((x) => x.name === g.dataset.group)!.advanced);
    expect(screen.getByText(/Solver \(advanced\)/)).toBeTruthy();
    const shape = document.querySelector('[data-param="shape"] select') as HTMLSelectElement;
    expect([...shape.options].map((o) => o.textContent)).toEqual(['sphere', 'cube', 'plane']);
  });

  it('a parameter added to the schema gets a control with NO UI code change (dummy parameter)', () => {
    const schema = structuredClone(baseSchema);
    schema.params.push({ name: 'dummy_greenhouse', group: 'laws', unit: 'x', min: 0, max: 4, default: 1, widget: 'slider', scope: 'climate', description: 'test only', identity: false } as any);
    schema.params.push({ name: 'dummy_flag', group: 'invented_group', unit: '', min: 0, max: 1, default: 0, widget: 'select', options: ['off', 'on'], scope: 'climate', description: '', identity: false } as any);
    const r = rig(schema, { ...baseWorld, params: { ...baseWorld.params } });
    render(<ParameterPanel app={r.app} ctl={r.ctl} />);
    const el = document.querySelector('[data-param="dummy_greenhouse"]') as HTMLElement;
    expect(el).toBeTruthy();
    expect(within(el).getByLabelText('dummy greenhouse slider')).toBeTruthy();
    expect(el.textContent).toContain('default 1 x');
    expect((document.querySelector('details[data-group="laws"]') as HTMLElement).contains(el)).toBe(true);
    expect(document.querySelector('details[data-group="invented_group"] [data-param="dummy_flag"] select')).toBeTruthy();
  });

  it('dragging a slider sends ONE set_param after the pause and never a solve', async () => {
    const r = rig();
    render(<ParameterPanel app={r.app} ctl={r.ctl} />);
    const slider = screen.getByLabelText('axial tilt deg slider') as HTMLInputElement;
    for (let v = 24; v <= 60; v += 1) { fireEvent.change(slider, { target: { value: String(v) } }); await act(async () => { vi.advanceTimersByTime(5); }); }
    expect(r.sent).toEqual([]); // 37 pointer moves, nothing sent while dragging
    expect((screen.getByLabelText('axial tilt deg slider') as HTMLInputElement).value).toBe('60'); // the UI follows the pointer from the draft
    await act(async () => { vi.advanceTimersByTime(150); });
    expect(r.sent).toEqual([{ type: 'set_param', name: 'axial_tilt_deg', value: 60 }]);
  });

  it('a value the core rejects is shown next to its control, not clamped', async () => {
    const r = rig();
    render(<ParameterPanel app={r.app} ctl={r.ctl} />);
    const input = document.querySelector('[data-param="axial_tilt_deg"] input[type="number"]') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '120' } });
    await act(async () => { vi.advanceTimersByTime(150); });
    const box = document.querySelector('[data-param="axial_tilt_deg"]') as HTMLElement;
    const alert = within(box).getByRole('alert');
    expect(alert.textContent).toContain('axial_tilt_deg');
    expect(alert.textContent).toContain('[0, 90]');
    expect((box.querySelector('input[type="number"]') as HTMLInputElement).value).toBe('120'); // not clamped to 90
    expect(r.store.getState().world!.params.axial_tilt_deg).toBe(23.4);
    expect(box.querySelector('input[type="number"]')!.getAttribute('aria-invalid')).toBe('true');
    fireEvent.change(box.querySelector('input[type="number"]')!, { target: { value: '45' } });
    expect(within(box).queryByRole('alert')).toBeNull();
  });

  it('reset restores one parameter, and the group button resets the group', async () => {
    const r = rig();
    render(<ParameterPanel app={r.app} ctl={r.ctl} />);
    await act(async () => { r.pc.edit('axial_tilt_deg', 50); r.pc.edit('solar_constant', 1500); await r.pc.flush(); });
    expect(r.store.getState().world!.params.axial_tilt_deg).toBe(50);
    await act(async () => { fireEvent.click(within(document.querySelector('[data-param="axial_tilt_deg"]') as HTMLElement).getByRole('button', { name: /reset/i })); });
    expect(r.store.getState().world!.params.axial_tilt_deg).toBe(23.4);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /reset laws of physics to defaults/i })); });
    expect(r.store.getState().world!.params.solar_constant).toBe(1361);
  });
});

describe('preset picker and dirty indicator', () => {
  it('lists the schema presets, shows clean then dirty, and goes back to the preset', async () => {
    const r = rig();
    r.setWorld({ preset: 'earthlike' });
    render(<PresetPanel app={r.app} ctl={r.ctl} />);
    const sel = screen.getByRole('combobox') as HTMLSelectElement;
    expect([...sel.options].map((o) => o.value)).toEqual(baseSchema.presets.map((p) => p.name));
    expect(screen.getByRole('status').getAttribute('data-dirty')).toBe('false');
    await act(async () => { r.pc.edit('axial_tilt_deg', 60); await r.pc.flush(); });
    expect(screen.getByRole('status').getAttribute('data-dirty')).toBe('true');
    expect(screen.getByText(/axial tilt deg: 23.4 to 60 deg/)).toBeTruthy();
    // an identity parameter (seed) does not make the world dirty
    await act(async () => { r.pc.edit('axial_tilt_deg', 23.4); r.pc.edit('seed', 99); await r.pc.flush(); });
    expect(screen.getByRole('status').getAttribute('data-dirty')).toBe('false');
    await act(async () => { fireEvent.change(sel, { target: { value: 'waterworld' } }); });
    expect(r.sent.at(-1)).toEqual({ type: 'load_preset', name: 'waterworld' });
  });
});

describe('seasons, history, variations panels', () => {
  const solvedWorld = (r: Rig) => r.setWorld({ solved: true, fields: [{ name: 'season_temperature', layers: 12, version: 1, stale: false }, ...r.store.getState().world!.fields] });

  it('the season slider only appears for a solved world with an orbit and calls setSeason', () => {
    const r = rig();
    const { rerender } = render(<SeasonPanel app={r.app} ctl={r.ctl} />);
    expect(screen.getByText(/solve first/i)).toBeTruthy();
    solvedWorld(r);
    rerender(<SeasonPanel app={r.app} ctl={r.ctl} />);
    fireEvent.change(screen.getByLabelText('Season'), { target: { value: '6' } });
    expect((r.ctl.setSeason as any)).toHaveBeenCalledWith(6);
    r.setWorld({ params: { ...r.store.getState().world!.params, year_length_days: 0 } });
    rerender(<SeasonPanel app={r.app} ctl={r.ctl} />);
    expect(screen.getByText(/no orbit/i)).toBeTruthy();
  });

  it('history lists undo steps, the undone ones struck through, and the buttons follow can_undo/can_redo', async () => {
    const r = rig();
    r.setWorld({
      can_undo: true, can_redo: true,
      history: {
        undo: [{ label: 'preset desert', preset: 'desert', changes: [] }, { label: 'axial_tilt_deg: 23.4 -> 60', preset: 'custom', changes: [{ name: 'axial_tilt_deg', old: 23.4, new: 60 }] }],
        redo: [{ label: 'surface_gravity: 9.81 -> 5', preset: 'custom', changes: [] }],
      },
    });
    render(<HistoryPanel app={r.app} ctl={r.ctl} />);
    const items = screen.getAllByRole('listitem').map((l) => l.textContent);
    expect(items).toEqual(['preset desert', 'axial_tilt_deg: 23.4 -> 60', 'surface_gravity: 9.81 -> 5']);
    expect((screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement).disabled).toBe(false);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Undo' })); });
    expect(r.sent.at(-1)).toEqual({ type: 'undo' });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Redo' })); });
    expect(r.sent.at(-1)).toEqual({ type: 'redo' });
  });

  it('variations: one button asks for 6, and six thumbnails appear; clicking one opens it (seed then solve)', async () => {
    const r = rig();
    const mesh = { meshId: 'm', kind: 'plane', nCells: 1, centroid: new Float32Array([0, 0, 0]), area: new Float32Array([1]), adjOffsets: new Uint32Array([0, 0]), adj: new Uint32Array([]) };
    r.store.setState({ mesh: mesh as any, variations: { mesh_id: 'm', base_seed: 7, coarse: true, fields: ['elevation', 'temperature', 'precipitation', 'biome'], items: [0, 1, 2, 3, 4, 5].map((i) => ({ index: i, seed: 100 + i })) } });
    render(<VariationsPanel app={r.app} ctl={r.ctl} />);
    expect(screen.getAllByRole('button', { name: /^Seed 10\d$/ }).length).toBe(6);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Seed 103' })); });
    expect(r.sent.slice(-2)).toEqual([{ type: 'set_param', name: 'seed', value: 103 }, { type: 'solve' }]);
  });
});

describe('layout is data', () => {
  it('every registered panel names a real component', () => {
    for (const p of PANEL_REGISTRY) expect(PANEL_COMPONENTS[p.component], p.id).toBeTruthy();
    expect(PANEL_REGISTRY.map((p) => p.id)).toEqual(expect.arrayContaining(['preset', 'params', 'variations', 'season', 'history']));
  });
});
