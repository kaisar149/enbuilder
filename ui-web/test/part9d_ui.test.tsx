// @vitest-environment jsdom
// Part 9d demo script against the REAL wb-server, no mocks: solve, show a layer, switch 2D <-> 3D, hover for the tooltip, change relief.
// jsdom has no WebGL, so the factory hands out the stub renderer; everything above the renderer is the production code path.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { WbApp } from 'wb-client';
import { startServer, type RunningServer } from '../../wb-client/test/harness';
import { CellPanel } from '../src/panels/CellPanel';
import { LayersPanel } from '../src/panels/LayersPanel';
import { LegendPanel } from '../src/panels/LegendPanel';
import { View3DPanel } from '../src/panels/View3DPanel';
import { MapView } from '../src/MapView';
import { MapController } from '../src/state/controller';
import { RELIEF_FIELD } from '../src/viewport/view3d';

const noop = () => undefined;
const ctx2d = { setTransform: noop, clearRect: noop, drawImage: noop, putImageData: noop, createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }), imageSmoothingEnabled: true, globalCompositeOperation: '' };
// 2D canvas works (stub context); WebGL does not exist in jsdom, so the 3D factory falls back to the stub renderer
(HTMLCanvasElement.prototype as any).getContext = (kind: string) => (kind === '2d' ? ctx2d : null);
(globalThis as any).requestAnimationFrame = (f: () => void) => setTimeout(f, 1);
(globalThis as any).cancelAnimationFrame = (h: number) => clearTimeout(h);
(globalThis as any).ResizeObserver = undefined;

let srv: RunningServer, app: WbApp, ctl: MapController;
const commands = () => app.client.sent.filter((r) => !['get_field', 'get_cell', 'get_mesh', 'get_schema', 'get_world_info'].includes(r.type)).map((r) => r.type);

beforeAll(async () => {
  srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2', '--param', 'spinup_years=1']);
  app = await WbApp.connect(srv.wsUrl, { WebSocketImpl: (globalThis as any).WebSocket });
  ctl = new MapController(app);
  await ctl.params.solve();
  await app.waitJob(app.store.getState().jobs.current!.id);
  await app.idle();
}, 120000);
afterAll(async () => { cleanup(); ctl?.dispose(); app?.dispose(); await srv?.stop(); });

describe('Part 9d demo script (real server)', () => {
  it('2D/3D toggle keeps layer, selection and legend; the toggle sends no command', async () => {
    await ctl.selectLayer('temperature');
    render(<><MapView app={app} ctl={ctl} /><View3DPanel app={app} ctl={ctl} /><LayersPanel app={app} ctl={ctl} /><div data-testid="legend"><LegendPanel app={app} ctl={ctl} /></div><CellPanel app={app} ctl={ctl} /></>);
    await ctl.pick(7);
    await waitFor(() => expect(ctl.ui.getState().cellInfo?.cell).toBe(7));
    const legendBefore = screen.getByTestId('legend').textContent;
    expect(legendBefore).toMatch(/\S/);
    const before = commands().length;

    fireEvent.click(screen.getByRole('button', { name: '3D world' }));
    await waitFor(() => expect(ctl.ui.getState().renderer).toBe('stub'));
    expect(ctl.ui.getState().view).toBe('3d');
    expect(ctl.ui.getState().layer).toBe('temperature');
    expect(ctl.ui.getState().pickedCell).toBe(7);
    // 3D asks for the relief field with a query, never a command
    await waitFor(() => expect(app.store.getState().fields[`${RELIEF_FIELD}#0`]).toBeTruthy());
    expect(screen.getByLabelText('Relief exaggeration')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '2D map' }));
    await waitFor(() => expect(ctl.ui.getState().renderer).toBe('2d'));
    expect(ctl.ui.getState().layer).toBe('temperature');
    expect(ctl.ui.getState().pickedCell).toBe(7);
    expect(ctl.ui.getState().cellInfo?.cell).toBe(7);
    expect(commands().length).toBe(before);
    const legendAfter = screen.getByTestId('legend').textContent;
    expect(legendAfter).toBe(legendBefore);
    cleanup();
  });

  it('hover in 3D reads the cell with get_cell and shows temperature and rainfall rows', async () => {
    render(<MapView app={app} ctl={ctl} />);
    ctl.setView('3d');
    await waitFor(() => expect(ctl.ui.getState().renderer).toBe('stub'));
    const q0 = app.client.sent.filter((r) => r.type === 'get_cell').length;
    act(() => ctl.hover(12, { x: 40, y: 50 }));
    await waitFor(() => expect(ctl.ui.getState().hoverInfo?.cell).toBe(12), { timeout: 3000 });
    expect(app.client.sent.filter((r) => r.type === 'get_cell').length).toBe(q0 + 1);
    const tip = await screen.findByRole('tooltip');
    expect(tip.textContent).toMatch(/Temperature/);
    expect(tip.textContent).toMatch(/Precipitation|Rain/i);
    // moving away and back uses the cache: no second query for the same cell
    act(() => ctl.hover(null)); act(() => ctl.hover(12, { x: 41, y: 50 }));
    expect(app.client.sent.filter((r) => r.type === 'get_cell').length).toBe(q0 + 1);
    ctl.setView('2d');
    cleanup();
  });

  it('relief exaggeration and lighting are UI-only: sliders send nothing to the core', async () => {
    render(<View3DPanel app={app} ctl={ctl} />);
    ctl.setView('3d');
    const before = app.client.sent.length;
    fireEvent.change(await screen.findByLabelText('Relief exaggeration'), { target: { value: '60' } });
    fireEvent.change(screen.getByLabelText('Lighting'), { target: { value: 'sun' } });
    fireEvent.change(await screen.findByLabelText('Time of day'), { target: { value: '0.25' } });
    expect(ctl.ui.getState().view3d).toMatchObject({ exaggeration: 60, lighting: 'sun', timeOfDay: 0.25 });
    expect(app.client.sent.length).toBe(before);
    cleanup();
  });
});
