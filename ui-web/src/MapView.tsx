import { useEffect, useRef } from 'react';
import { displayValue, metaOf, selectField } from 'wb-client';
import type { WbApp } from 'wb-client';
import type { MapController } from './state/controller';
import { viewLayers } from './state/controller';
import { useStore } from './state/hooks';
import { MapViewport2D } from './viewport/MapViewport2D';
import type { Viewport } from './viewport/types';

/** Hosts a Viewport. Only talks to it through the Viewport interface, so swapping in a globe means changing the constructor line below. */
export function MapView({ app, ctl }: { app: WbApp; ctl: MapController }) {
  const host = useRef<HTMLDivElement>(null);
  const vp = useRef<Viewport | null>(null);
  const mesh = useStore(app.store, (s) => s.mesh);
  const schema = useStore(app.store, (s) => s.schema);
  const appState = useStore(app.store, (s) => s);
  const ui = useStore(ctl.ui, (s) => s);

  useEffect(() => {
    const v = new MapViewport2D(host.current!);
    vp.current = v;
    const off = [v.onHover((e) => ctl.hover(e.cell)), v.onPick((e) => void ctl.pick(e.cell))];
    return () => { off.forEach((o) => o()); v.dispose(); vp.current = null; };
  }, [ctl]);

  useEffect(() => { if (mesh) vp.current?.setMesh(mesh); }, [mesh]);

  useEffect(() => {
    const v = viewLayers(appState, ui);
    if (v && mesh) vp.current?.setLayers(v.layers, v.fields, { biomes: schema?.biomes ?? [] });
  }, [appState.fields, appState.schema, ui.layer, mesh, schema]);

  const field = ui.layer ? selectField(appState, ui.layer) : undefined;
  const meta = ui.layer ? metaOf(schema, ui.layer) : undefined;
  let readout = 'Move over the map to read values.';
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
  return (
    <div className="map-wrap">
      <div className="map-host" ref={host} />
      <div className="map-readout" aria-live="polite">{readout}</div>
    </div>
  );
}
