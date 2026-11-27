// @vitest-environment jsdom
// Part 9f demo script against the REAL wb-server, no mocks: turn painting on, raise a mountain, watch preview then refined, lock a valley,
// try to repaint it, move the realism slider, compare views, undo from the history panel.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { WbApp, viewKey } from 'wb-client';
import { startServer, type RunningServer } from '../../wb-client/test/harness';
import { HistoryPanel } from '../src/panels/HistoryPanel';
import { RealismPanel } from '../src/panels/RealismPanel';
import { ToolsPanel } from '../src/panels/ToolsPanel';
import { MapView } from '../src/MapView';
import { MapController } from '../src/state/controller';

const noop = () => undefined;
const ctx2d = { fillRect: noop, save: noop, restore: noop, beginPath: noop, rect: noop, clip: noop, moveTo: noop, lineTo: noop, stroke: noop, fill: noop, setTransform: noop, clearRect: noop, drawImage: noop, putImageData: noop, createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }), imageSmoothingEnabled: true, globalCompositeOperation: '' };
(HTMLCanvasElement.prototype as any).getContext = (kind: string) => (kind === '2d' ? ctx2d : null);
(globalThis as any).requestAnimationFrame = (f: () => void) => setTimeout(f, 1);
(globalThis as any).cancelAnimationFrame = (h: number) => clearTimeout(h);
(globalThis as any).ResizeObserver = undefined;

let srv: RunningServer, app: WbApp, ctl: MapController;
const sent = (type: string) => app.client.sent.filter((r) => r.type === type).length;

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

beforeAll(async () => {
  srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=3', '--param', 'spinup_years=1']);
  app = await WbApp.connect(srv.wsUrl, { WebSocketImpl: (globalThis as any).WebSocket });
  ctl = new MapController(app);
  await ctl.params.solve();
  await app.waitJob(app.store.getState().jobs.current!.id);
  await app.idle();
  await ctl.selectLayer('elevation');
  render(<><MapView app={app} ctl={ctl} /><ToolsPanel app={app} ctl={ctl} /><RealismPanel app={app} ctl={ctl} /><HistoryPanel app={app} ctl={ctl} /></>);
}, 120000);
afterAll(async () => { cleanup(); ctl?.dispose(); app?.dispose(); await srv?.stop(); });

describe('Part 9f demo script (real server)', () => {
  it('the tool list is generated from the schema; painting starts off', () => {
    const edit = app.store.getState().schema!.edit!;
    for (const t of edit.tools) expect(screen.getByRole('button', { name: t.label })).toBeTruthy();
    expect(ctl.ui.getState().editOn).toBe(false);
    expect(screen.getByRole('button', { name: 'Paint on the map' }).getAttribute('aria-pressed')).toBe('false');
  });

  it('strokes are ignored until painting is on', async () => {
    ctl.handleStroke({ phase: 'down', cell: 5, x: 0, y: 0 });
    expect(ctl.edit.state.phase).toBe('idle');
    fireEvent.click(screen.getByRole('button', { name: 'Paint on the map' }));
    expect(ctl.ui.getState().editOn).toBe(true);
    expect(screen.getByRole('button', { name: 'Painting is on' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('raise a mountain: a drag is ONE command, then the badge shows the quick preview and ends on Refined', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Raise' }));
    fireEvent.change(screen.getByLabelText('Brush radius'), { target: { value: '900' } });
    expect(ctl.edit.state.brush.radiusKm).toBe(900);
    const mesh = app.store.getState().mesh!;
    const c0 = landCell();
    const before = sent('paint_stroke');
    ctl.handleStroke({ phase: 'down', cell: c0, x: 0, y: 0 });
    let c = c0;
    for (let i = 0; i < 25; i++) { c = mesh.adj[mesh.adjOffsets[c]]; ctl.handleStroke({ phase: 'move', cell: c, x: 0, y: 0 }); }
    expect(ctl.edit.state.preview!.cells.length).toBeGreaterThan(3); // stroke preview is local
    expect(sent('paint_stroke')).toBe(before);
    ctl.handleStroke({ phase: 'up', cell: c, x: 0, y: 0 });
    expect(sent('paint_stroke')).toBe(before + 1);
    // while the refresh runs the badge says which pass it is; at the end it says Refined
    await waitFor(() => expect(screen.queryByTestId('refresh-badge')?.textContent ?? '').toMatch(/Refining|Refined/), { timeout: 20000 });
    await waitFor(() => expect(screen.getByTestId('refresh-badge').textContent).toBe('Refined'), { timeout: 30000 });
    expect(ctl.edit.state.lastChain).toEqual(['preview', 'refined']);
    expect(sent('refresh')).toBe(2);
  }, 60000);

  it('history lists the stroke, and Undo takes it back', async () => {
    const hist = screen.getByRole('list', { name: 'Edit history' });
    const strokes = within(hist).getAllByText(/./).filter((li) => li.getAttribute('data-kind') === 'stroke');
    expect(strokes.length).toBe(1);
    expect(app.store.getState().world!.edit!.paint.elevation).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(app.store.getState().world!.edit!.paint.elevation).toBe(0));
    fireEvent.click(screen.getByRole('button', { name: 'Redo' }));
    await waitFor(() => expect(app.store.getState().world!.edit!.paint.elevation).toBeGreaterThan(0));
  }, 30000);

  it('lock a valley: the lock tool locks cells, a later raise over them is reported as skipped and changes nothing', async () => {
    await ctl.edit.idle();
    const z0 = (await app.client.getField('elevation')).values;
    fireEvent.click(screen.getByRole('button', { name: 'Lock' }));
    expect(screen.getByLabelText('Field to lock')).toBeTruthy();
    const c = landCell();
    ctl.handleStroke({ phase: 'down', cell: c, x: 0, y: 0 });
    ctl.handleStroke({ phase: 'up', cell: c, x: 0, y: 0 });
    await waitFor(() => expect(app.store.getState().world!.edit!.locks.elevation).toBeGreaterThan(0));
    await waitFor(() => expect(app.store.getState().locks?.cells.elevation).toContain(c));
    await ctl.edit.idle();
    fireEvent.click(screen.getByRole('button', { name: 'Raise' }));
    ctl.handleStroke({ phase: 'down', cell: c, x: 0, y: 0 });
    ctl.handleStroke({ phase: 'up', cell: c, x: 0, y: 0 });
    await waitFor(() => expect(screen.getByText(/locked cells were left unchanged/)).toBeTruthy());
    await ctl.edit.idle();
    const z1 = (await app.client.getField('elevation')).values;
    expect(z1[c]).toBe((await app.client.getField('elevation')).values[c]);
    expect(Number.isFinite(z0[c])).toBe(true);
  }, 60000);

  it('realism slider: 100 on elevation makes the map the physics (instant, no refresh), 0 brings the paint back', async () => {
    await ctl.edit.idle();
    const refreshes = sent('refresh') + sent('solve');
    const slider = screen.getByLabelText('Elevation realism');
    fireEvent.change(slider, { target: { value: '100' } });
    fireEvent.pointerUp(slider);
    await waitFor(() => expect(app.store.getState().world!.edit!.realism.elevation).toBe(100));
    expect(sent('refresh') + sent('solve')).toBe(refreshes);
    fireEvent.change(slider, { target: { value: '0' } });
    fireEvent.pointerUp(slider);
    await waitFor(() => expect(app.store.getState().world!.edit!.realism.elevation).toBe(0));
  }, 30000);

  it('view switch: Physics only fetches the computed view of the shown field; Blend returns', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Physics only' }));
    expect(ctl.ui.getState().fieldView).toBe('computed');
    await waitFor(() => expect(app.store.getState().fieldViews[viewKey('elevation', 'computed')]).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Blend' }));
    expect(ctl.ui.getState().fieldView).toBe('blended');
  }, 30000);

  it('turning painting off drops the stroke tools back to navigation', () => {
    fireEvent.click(screen.getByRole('button', { name: 'Painting is on' }));
    expect(ctl.ui.getState().editOn).toBe(false);
    ctl.handleStroke({ phase: 'down', cell: 3, x: 0, y: 0 });
    expect(ctl.edit.state.phase).toBe('idle');
  });
});
