// @vitest-environment jsdom
// Part 9c demo script, automated against the REAL wb-server: set tilt, Solve, compare seasons, preset, variations, undo. No mocks.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { WbApp } from 'wb-client';
import { startServer, type RunningServer } from '../../wb-client/test/harness';
import { HistoryPanel } from '../src/panels/HistoryPanel';
import { ParameterPanel } from '../src/panels/ParameterPanel';
import { PresetPanel } from '../src/panels/PresetPanel';
import { SeasonPanel } from '../src/panels/SeasonPanel';
import { VariationsPanel } from '../src/panels/VariationsPanel';
import { MapController, viewLayers } from '../src/state/controller';
import { renderThumbnail } from '../src/viewport/thumbnail';
import { layerDecl } from 'wb-client';

// jsdom has no canvas; the panel skips drawing when there is no 2d context, and the pixels are checked through renderThumbnail below
HTMLCanvasElement.prototype.getContext = (() => null) as any;
let srv: RunningServer;
let app: WbApp;
let ctl: MapController;
const sent = (t: string) => app.client.sent.filter((r) => r.type === t);
const world = () => app.store.getState().world!;
const tiltInput = () => document.querySelector('[data-param="axial_tilt_deg"] input[type="number"]') as HTMLInputElement;

beforeAll(async () => {
  srv = await startServer(['--preset', 'earthlike', '--param', 'resolution=2', '--param', 'spinup_years=1']);
  app = await WbApp.connect(srv.wsUrl, { WebSocketImpl: (globalThis as any).WebSocket });
  ctl = new MapController(app);
});
afterAll(async () => {
  cleanup();
  ctl?.dispose();
  app?.dispose();
  await srv?.stop();
});

describe('Part 9c demo script', () => {
  it('Earth-like, tilt 23 -> 60 with ONE set_param, Solve, then compare seasons', async () => {
    render(<><PresetPanel app={app} ctl={ctl} /><ParameterPanel app={app} ctl={ctl} /></>);
    fireEvent.change(tiltInput(), { target: { value: '60' } });
    fireEvent.keyDown(tiltInput(), { key: 'Enter' });
    await waitFor(() => expect(world().params.axial_tilt_deg).toBe(60));
    expect(sent('set_param').length).toBe(1);
    expect(sent('solve').length).toBe(0);
    expect(document.querySelector('[data-dirty="true"]')).toBeTruthy(); // the preset indicator noticed the change
    await ctl.params.solve();
    expect(await app.waitJob(app.store.getState().jobs.current!.id)).toBe('completed');
    await app.idle();
    await ctl.selectLayer('temperature');
    const annual = viewLayers(app.store.getState(), ctl.ui.getState())!;
    const yearly = Array.from(Object.values(annual.fields)[0].values);
    ctl.setSeason(0);
    await waitFor(() => expect(app.store.getState().fields['season_temperature#0']).toBeTruthy());
    ctl.setSeason(6);
    await waitFor(() => expect(app.store.getState().fields['season_temperature#6']).toBeTruthy());
    const winter = viewLayers(app.store.getState(), { ...ctl.ui.getState(), season: 0 })!;
    const summer = viewLayers(app.store.getState(), ctl.ui.getState())!;
    const a = Array.from(Object.values(winter.fields)[0].values), b = Array.from(Object.values(summer.fields)[0].values);
    expect(a).not.toEqual(b);
    expect(a).not.toEqual(yearly);
    expect(Object.values(summer.layers)[0].colormap).toBe(Object.values(annual.layers)[0].colormap);
    expect(sent('solve').length).toBe(1); // switching seasons asked for data only
    ctl.setSeason(null);
    cleanup();
  });

  it('the season panel offers the slider once solved', async () => {
    render(<SeasonPanel app={app} ctl={ctl} />);
    expect(screen.getByLabelText('Season')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Season'), { target: { value: '3' } });
    expect(ctl.ui.getState().season).toBe(3);
    ctl.setSeason(null);
    cleanup();
  });

  it('an out-of-range value is rejected by the real core and shown next to the control', async () => {
    render(<ParameterPanel app={app} ctl={ctl} />);
    fireEvent.change(tiltInput(), { target: { value: '120' } });
    fireEvent.keyDown(tiltInput(), { key: 'Enter' });
    const box = document.querySelector('[data-param="axial_tilt_deg"]') as HTMLElement;
    await waitFor(() => expect(within(box).getByRole('alert').textContent).toContain('axial_tilt_deg'));
    expect(world().params.axial_tilt_deg).toBe(60);
    expect(tiltInput().value).toBe('120');
    ctl.params.discard();
    cleanup();
  });

  it('pick Waterworld, then undo it from the history panel', async () => {
    render(<><PresetPanel app={app} ctl={ctl} /><HistoryPanel app={app} ctl={ctl} /></>);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'waterworld' } });
    await waitFor(() => expect(world().params.ocean_fraction).toBe(0.97));
    await waitFor(() => expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toContain('preset waterworld'));
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(world().params.ocean_fraction).toBe(0.7));
    expect(world().params.axial_tilt_deg).toBe(60);
    cleanup();
  });

  it('generate 6 variations, see 6 thumbnails, open one and get its world', async () => {
    render(<VariationsPanel app={app} ctl={ctl} />);
    fireEvent.click(screen.getByRole('button', { name: /generate 6 variations/i }));
    await waitFor(() => expect(screen.getAllByRole('button', { name: /^Seed \d+$/ }).length).toBe(6), { timeout: 60000 });
    await waitFor(() => expect(Object.keys(app.store.getState().variationFields).length).toBe(6), { timeout: 20000 });
    const canvases = document.querySelectorAll('.thumb canvas');
    expect(canvases.length).toBe(6);
    // the thumbnail pixels: painted, and the six seeds do not all give the same picture
    const st = app.store.getState();
    const imgs = Object.values(st.variationFields).map((f) => renderThumbnail(st.mesh!, f, layerDecl(st.schema!, f), { biomes: st.schema!.biomes }, 120));
    for (const im of imgs) {
      let painted = 0;
      for (let i = 3; i < im.rgba.length; i += 4) if (im.rgba[i] === 255) painted++;
      expect(painted).toBeGreaterThan(im.width * im.height * 0.5);
    }
    expect(new Set(imgs.map((im) => Buffer.from(im.rgba.buffer).toString('base64'))).size).toBeGreaterThan(1);
    const first = screen.getAllByRole('button', { name: /^Seed \d+$/ })[1];
    const seed = Number(first.getAttribute('aria-label')!.replace('Seed ', ''));
    fireEvent.click(first);
    await waitFor(() => expect(world().params.seed).toBe(seed));
    await waitFor(() => expect(app.store.getState().jobs.current?.kind).toBe('solve'));
    expect(await app.waitJob(app.store.getState().jobs.current!.id)).toBe('completed');
    cleanup();
  });
});
