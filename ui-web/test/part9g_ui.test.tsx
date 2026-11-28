// @vitest-environment jsdom
// Part 9g demo script against the REAL wb-server, no mocks: edit, refresh, press Diff, play the ripple, flip before / after, click a cell and read the
// chain, click a step, switch 2D / 3D mid-animation, lock cells and see they never change.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { WbApp, coreLinkLines, frameDelta, revealedCount } from 'wb-client';
import { startServer, type RunningServer } from '../../wb-client/test/harness';
import { DiffPanel } from '../src/panels/DiffPanel';
import { ExplainPanel } from '../src/panels/ExplainPanel';
import { MapView } from '../src/MapView';
import { MapController, viewLayers } from '../src/state/controller';
import { colormap } from '../src/viewport/colormaps';

const noop = () => undefined;
const ctx2d = { fillRect: noop, save: noop, restore: noop, beginPath: noop, rect: noop, clip: noop, moveTo: noop, lineTo: noop, stroke: noop, fill: noop, setTransform: noop, clearRect: noop, drawImage: noop, putImageData: noop, createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }), imageSmoothingEnabled: true, globalCompositeOperation: '' };
(HTMLCanvasElement.prototype as any).getContext = (kind: string) => (kind === '2d' ? ctx2d : null);
(globalThis as any).requestAnimationFrame = (f: () => void) => setTimeout(f, 1);
(globalThis as any).cancelAnimationFrame = (h: number) => clearTimeout(h);
(globalThis as any).ResizeObserver = undefined;

let srv: RunningServer, app: WbApp, ctl: MapController;
let edited = -1;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function landCell(): number {
  const z = app.store.getState().fields['elevation#0'].values;
  const mesh = app.store.getState().mesh!;
  let best = -1, bs = -1;
  for (let i = 0; i < z.length; i++) {
    if (z[i] <= 0) continue;
    let s = 0;
    for (let k = mesh.adjOffsets[i]; k < mesh.adjOffsets[i + 1]; k++) if (z[mesh.adj[k]] > 0) s++;
    if (s > bs) { bs = s; best = i; }
  }
  return best;
}
const ring = (c: number) => { const m = app.store.getState().mesh!; return [c, ...Array.from(m.adj.slice(m.adjOffsets[c], m.adjOffsets[c + 1]))]; };

beforeAll(async () => {
  srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=3', '--param', 'spinup_years=1']);
  app = await WbApp.connect(srv.wsUrl, { WebSocketImpl: (globalThis as any).WebSocket });
  ctl = new MapController(app);
  await ctl.params.solve();
  await app.waitJob(app.store.getState().jobs.current!.id);
  await app.idle();
  await ctl.selectLayer('elevation');
  await ctl.diff.data.refreshVersions();
  render(<><MapView app={app} ctl={ctl} /><DiffPanel app={app} ctl={ctl} /><ExplainPanel app={app} ctl={ctl} /></>);
}, 120000);
afterAll(async () => { cleanup(); ctl?.dispose(); app?.dispose(); await srv?.stop(); });

describe('Part 9g demo script (real server)', () => {
  it('with one stored version the panel says so', async () => {
    fireEvent.click(screen.getAllByRole('button', { name: 'Show diff' })[0]);
    await waitFor(() => expect(screen.getByText(/Only one version is stored/)).toBeTruthy());
    expect(ctl.diff.ui.getState().on).toBe(true);
  });

  it('edit + refine, then the diff loads: summary, table, diverging layer centred on zero', async () => {
    edited = landCell();
    const cells = ring(edited);
    await app.command({ type: 'paint_stroke', layer: 'elevation', op: 'add', cells, weights: cells.map((_, i) => (i === 0 ? 1 : 0.6)), value: 3500, strength: 1 });
    expect(await app.refreshAndWait('refined')).toBe('completed');
    await app.idle();
    await waitFor(() => expect(ctl.diff.data.state.versions!.versions.length).toBeGreaterThanOrEqual(2), { timeout: 5000 });
    await waitFor(() => expect(ctl.diff.data.state.summary?.no_change).toBe(false), { timeout: 15000 });
    await waitFor(() => expect(ctl.diff.data.state.diff?.noChange).toBe(false), { timeout: 15000 });
    await waitFor(() => expect(screen.getByTestId('diff-summary')).toBeTruthy());
    expect(screen.getByRole('table', { name: 'Changes per field' })).toBeTruthy();
    const d = ctl.diff.data.state.diff!;
    const v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: d })!;
    const decl = v.layers[0];
    expect(decl.colormap).toBe('diverging');
    expect(decl.range[0]).toBe(-decl.range[1]);
    expect(colormap('diverging', 0.5)).toEqual([247, 247, 247]);
    expect([...v.fields[decl.id].values]).toEqual([...frameDelta(d, 1)]);
  });

  it('the ripple: the playhead only moves forward, reveals cells in front order and ends with exactly the changed cells', async () => {
    const d = ctl.diff.data.state.diff!;
    ctl.diff.setT(0);
    const first = revealedCount(d, 0);
    ctl.diff.play();
    const seen: number[] = [];
    for (let i = 0; i < 6; i++) { await sleep(150); seen.push(ctl.diff.ui.getState().t); }
    expect(seen.every((t, i) => i === 0 || t >= seen[i - 1])).toBe(true);
    expect(seen[seen.length - 1]).toBeGreaterThan(0);
    // switch 2D <-> 3D mid-animation: the playhead keeps its value and keeps running
    const t0 = ctl.diff.ui.getState().t;
    ctl.setView('3d');
    await sleep(120);
    const t1 = ctl.diff.ui.getState().t;
    expect(t1).toBeGreaterThanOrEqual(t0);
    ctl.setView('2d');
    await sleep(120);
    expect(ctl.diff.ui.getState().t).toBeGreaterThanOrEqual(t1);
    ctl.diff.stop();
    ctl.diff.setT(1);
    expect(revealedCount(d, 1)).toBe(d.summary.changed_cells);
    expect(revealedCount(d, 0)).toBe(first);
    expect(first).toBeLessThan(d.summary.changed_cells);
  }, 20000);

  it('before / after flip and swipe show the stored arrays', () => {
    const d = ctl.diff.data.state.diff!;
    ctl.diff.setMode('before');
    let v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: d })!;
    expect([...Object.values(v.fields)[0].values]).toEqual([...d.before]);
    ctl.diff.flip();
    v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: d })!;
    expect(ctl.diff.ui.getState().mode).toBe('after');
    expect([...Object.values(v.fields)[0].values]).toEqual([...d.after]);
    ctl.diff.setMode('swipe');
    ctl.diff.setSwipe(0);
    v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: d })!;
    expect([...Object.values(v.fields)[0].values]).toEqual([...d.before]);
    ctl.diff.setSwipe(1.0);
    v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: d })!;
    expect(Object.values(v.fields)[0].values.length).toBe(d.nCells);
    ctl.diff.setMode('diff');
  });

  it('click a cell: the chain on screen equals the core chain, step by step; a step click highlights its cells', async () => {
    const f = ctl.diff.diffField()!;
    const d = await ctl.diff.data.loadDiff(f);
    const far = [...d!.delta.keys()].filter((i) => d!.delta[i] !== 0).sort((a, b) => d!.distKm[b] - d!.distKm[a])[0];
    await ctl.pick(far);
    await waitFor(() => expect(ctl.diff.data.state.explanation?.cell).toBe(far), { timeout: 5000 });
    const ex = ctl.diff.data.state.explanation!;
    const list = await screen.findByRole('list', { name: 'Causal chain, effect first' });
    const shown = within(list).getAllByRole('listitem').map((li) => li.querySelector('code')!.textContent);
    expect(shown).toEqual(coreLinkLines(ex.text).map((l) => l.split('  ')[0].split('; ')[0]));
    expect(shown.length).toBe(ex.steps.length);
    // clicking a step highlights its source cells; clicking it again clears
    fireEvent.click(within(list).getAllByRole('button')[0]);
    expect(ctl.diff.ui.getState().highlight[0]).toBe(ex.steps[0].cell);
    expect(ctl.diff.ui.getState().highlight.length).toBeGreaterThan(0);
    fireEvent.click(within(list).getAllByRole('button')[0]);
    expect(ctl.diff.ui.getState().highlight).toEqual([]);
  });

  it('a version compared with itself says "No change"', async () => {
    const v = ctl.diff.data.state.to!;
    await ctl.diff.selectPair(v, v);
    await waitFor(() => expect(screen.getByTestId('diff-no-change')).toBeTruthy());
    await ctl.diff.followLatest();
    await waitFor(() => expect(ctl.diff.data.state.summary?.no_change).toBe(false));
  });

  it('locked cells never appear as changed', async () => {
    const cells = ring(edited);
    await app.command({ type: 'set_lock', field: 'temperature', cells, locked: true });
    await app.command({ type: 'set_lock', field: 'elevation', cells, locked: true });
    const m = app.store.getState().mesh!;
    const other = [...Array(m.nCells).keys()].find((i) => !cells.includes(i) && i !== edited && app.store.getState().fields['elevation#0'].values[i] > 0)!;
    await app.command({ type: 'paint_stroke', layer: 'elevation', op: 'add', cells: [other], weights: [1], value: 2500, strength: 1 });
    expect(await app.refreshAndWait('refined')).toBe('completed');
    await app.idle();
    await ctl.diff.followLatest();
    for (const f of ['temperature', 'elevation']) {
      const d = (await ctl.diff.data.loadDiff(f))!;
      for (const c of cells) { expect(d.delta[c]).toBe(0); expect(d.front[c]).toBe(-1); }
    }
  }, 60000);

  it('turning the diff off restores the normal map layer', async () => {
    await ctl.diff.setOn(false);
    const v = viewLayers(app.store.getState(), ctl.ui.getState(), { ui: ctl.diff.ui.getState(), data: ctl.diff.data.state.diff })!;
    expect(v.layers[0].colormap).not.toBe('diverging');
  });
});
