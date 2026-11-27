import { useEffect, useRef, useState } from 'react';
import { displayValue, metaOf, selectField } from 'wb-client';
import { RefreshBadge } from './panels/RefreshBadge';
import { lockedForLayer, toolOverlayItems } from './tools/overlays';
import type { WbApp } from 'wb-client';
import type { MapController } from './state/controller';
import { viewLayers } from './state/controller';
import { buildOverlayItems } from './layers/items';
import { useStore } from './state/hooks';
import { MapViewport2D } from './viewport/MapViewport2D';
import { createViewport3D } from './viewport/factory';
import { runBenchmark } from './viewport/bench';
import { shapeKind } from './viewport/geometry3d';
import { RELIEF_FIELD, tooltipRows, view3DOptions } from './viewport/view3d';
import type { Viewport, Viewport3D } from './viewport/types';

const is3D = (v: Viewport | null): v is Viewport3D => !!v && typeof (v as Viewport3D).setView === 'function';

/**
 * Hosts a Viewport. Only talks to it through the Viewport / Viewport3D interfaces: the 2D map and the 3D renderer (made by viewport/factory.ts)
 * are interchangeable here, and both read the same layer, season and selection state, so a 2D/3D switch keeps all of them.
 */
export function MapView({ app, ctl }: { app: WbApp; ctl: MapController }) {
  const host = useRef<HTMLDivElement>(null);
  const vp = useRef<Viewport | null>(null);
  /** Bumped when a viewport instance is installed, so the data effects below run again for the new instance. */
  const [ready, setReady] = useState(0);
  const mesh = useStore(app.store, (s) => s.mesh);
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const appState = useStore(app.store, (s) => s);
  const ui = useStore(ctl.ui, (s) => s);
  const editState = useStore(ctl.edit.store, (s) => s);
  const locks = useStore(app.store, (s) => s.locks);

  // create / replace the viewport when the view kind changes
  useEffect(() => {
    let cancelled = false;
    let made: Viewport | null = null;
    let off: (() => void)[] = [];
    const install = (v: Viewport, name: string) => {
      if (cancelled) { v.dispose(); return; }
      made = v; vp.current = v;
      off = [v.onHover((e) => ctl.hover(e.cell, { x: e.x, y: e.y })), v.onPick((e) => void ctl.pick(e.cell)), v.onStroke((e) => ctl.handleStroke(e))];
      ctl.setRenderer(name);
      setReady((n) => n + 1);
    };
    if (ui.view === '2d') install(new MapViewport2D(host.current!), '2d');
    else void createViewport3D(host.current!).then((r) => install(r.viewport, r.renderer));
    return () => { cancelled = true; off.forEach((o) => o()); made?.dispose(); if (vp.current === made) vp.current = null; };
  }, [ctl, ui.view]);

  useEffect(() => { if (mesh) vp.current?.setMesh(mesh); }, [mesh, ready]);
  useEffect(() => { vp.current?.setToolMode(ui.editOn); }, [ui.editOn, ready]);
  // Escape abandons the stroke in progress
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.key === 'Escape') ctl.edit.cancelStroke(); };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [ctl]);

  useEffect(() => {
    const v = viewLayers(appState, ui);
    if (v && mesh) vp.current?.setLayers(v.layers, v.fields, { biomes: schema?.biomes ?? [] });
  }, [appState.fields, appState.fieldViews, appState.schema, ui.layer, ui.season, ui.fieldView, mesh, schema, ready]);

  // overlays (Part 9e): vectors, lines and masks above the scalar layers, in the stack's order. Colours come from the theme tokens --overlay-<token>.
  useEffect(() => {
    const v = vp.current;
    if (!v || !mesh || !host.current) return;
    const css = getComputedStyle(host.current);
    const resolve = (token: string) => css.getPropertyValue(`--overlay-${token}`).trim() || css.color || 'currentColor';
    const tools = toolOverlayItems({
      mesh,
      ring: ui.editOn && ui.hoverCell !== null && ctl.edit.tool ? { cell: ui.hoverCell, brush: editState.brush } : null,
      stroke: editState.preview,
      locked: lockedForLayer(locks, ui.layer),
      resolveColor: resolve,
    });
    v.setOverlays([...buildOverlayItems(appState, schema, ui.stack, resolve), ...tools], { animate: ui.animate });
  }, [appState.fields, appState.rivers, appState.outlines, schema, ui.stack, ui.animate, mesh, ready, ui.editOn, ui.hoverCell, ui.layer, editState.brush, editState.preview, editState.toolId, locks]);

  // 3D only: relief heights, lighting options, selection, camera reset, benchmark, frame statistics
  const heights = selectField(appState, RELIEF_FIELD)?.values ?? null;
  useEffect(() => { const v = vp.current; if (is3D(v)) v.setElevation(heights); }, [heights, mesh, ready]);
  useEffect(() => {
    const v = vp.current;
    if (is3D(v)) v.setView(view3DOptions({ ...ui.view3d, season: ui.season }, world, mesh, schema?.seasons?.count ?? 0));
  }, [ui.view3d, ui.season, world?.params, mesh, ready]);
  useEffect(() => { const v = vp.current; if (is3D(v)) v.setSelection(ui.pickedCell); }, [ui.pickedCell, mesh, ready]);
  useEffect(() => { const v = vp.current; if (is3D(v) && ui.resetNonce) v.resetView(); }, [ui.resetNonce]);
  useEffect(() => {
    const v = vp.current;
    if (!is3D(v) || !mesh || !ui.benchNonce) return;
    void runBenchmark(v, shapeKind(mesh)).then((r) => ctl.setBench(r));
  }, [ui.benchNonce]);
  useEffect(() => {
    if (ui.view !== '3d') return;
    let last = performance.now();
    const t = setInterval(() => {
      const v = vp.current;
      const s = is3D(v) ? v.frameStats() : null;
      const now = performance.now();
      if (s) ctl.setFrame({ fps: (1000 * s.frames) / (now - last), drawMs: s.meanMs });
      last = now;
    }, 1000);
    return () => clearInterval(t);
  }, [ctl, ui.view]);

  const field = ui.layer ? selectField(appState, ui.layer) : undefined;
  const meta = ui.layer ? metaOf(schema, ui.layer) : undefined;
  let readout = ui.view === '3d' ? 'Drag to turn the world, wheel to zoom, double-click a cell to centre it.' : 'Move over the map to read values.';
  if (ui.hoverCell !== null && field && meta) {
    const raw = field.values[ui.hoverCell];
    if (meta.kind === 'category') {
      const b = schema?.biomes.find((x) => x.id === Math.round(raw));
      readout = `Cell ${ui.hoverCell}: ${b?.name ?? `class ${Math.round(raw)}`}`;
    } else {
      const v = displayValue(meta, raw);
      readout = `Cell ${ui.hoverCell}: ${Number.isFinite(v) ? v.toFixed(Math.abs(v) >= 100 ? 0 : 2) : 'no value'} ${meta.display.unit}`.trim();
    }
  }
  const tipRows = ui.view === '3d' && ui.hoverInfo && ui.hoverInfo.cell === ui.hoverCell ? tooltipRows(schema, ui.hoverInfo, ui.layer) : [];
  return (
    <div className="map-wrap">
      <div className="map-toolbar" role="toolbar" aria-label="Map view">
        <div className="seg" role="group" aria-label="2D or 3D">
          <button type="button" className={ui.view === '2d' ? 'seg-on' : ''} aria-pressed={ui.view === '2d'} onClick={() => ctl.setView('2d')}>2D map</button>
          <button type="button" className={ui.view === '3d' ? 'seg-on' : ''} aria-pressed={ui.view === '3d'} onClick={() => ctl.setView('3d')}>3D world</button>
        </div>
        {ui.view === '3d' && <button type="button" className="btn" onClick={() => ctl.resetCamera()}>Reset view</button>}
        <RefreshBadge app={app} ctl={ctl} />
      </div>
      <div className="map-host" ref={host} />
      {ui.view === '3d' && ui.hoverPos && ui.hoverCell !== null && (
        <div className="map-tip" role="tooltip" style={{ left: ui.hoverPos.x + 14, top: ui.hoverPos.y + 14 }}>
          <strong>{ui.hoverInfo?.biome_name ?? `Cell ${ui.hoverCell}`}</strong>
          {tipRows.map((r) => <div key={r.label}>{r.label}: {r.text}</div>)}
          {tipRows.length === 0 && <div className="map-tip-wait">Reading cell {ui.hoverCell}</div>}
        </div>
      )}
      <div className="map-readout" aria-live="polite">{readout}</div>
    </div>
  );
}
