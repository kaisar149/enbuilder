import { drawnOverlays, BLENDS, isDrawn, matchesPreset, moveBy, setBlend, setOpacity, setVisible, toggleSolo, type Blend } from '../layers/stack';
import { overlayAvailable } from '../layers/items';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

/**
 * The layer stack: presets, the base layer, and the overlays top to bottom. Every overlay comes from `get_schema` (overlays[]), every preset from
 * layer_presets[]; nothing here names a field. Reordering and the other controls change UI state only (no command, no refetch of data).
 */
export function LayerStackPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const stack = useStore(ctl.ui, (s) => s.stack);
  const animate = useStore(ctl.ui, (s) => s.animate);
  if (!schema?.overlays?.length) return <p className="note">The core declares no overlays.</p>;
  const defs = new Map(schema.overlays.map((o) => [o.id, o]));
  const available = (f: string) => !!world?.fields.some((x) => x.name === f);
  const top = [...stack.entries].reverse(); // the list reads top to bottom, like a layer panel
  const drawn = new Set(drawnOverlays(stack, schema).map((d) => d.def.id));
  const anyAnimated = schema.overlays.some((o) => o.animated);
  return (
    <div className="stack">
      <div className="seg seg-wrap" role="group" aria-label="Layer presets">
        {(schema.layer_presets ?? []).map((p) => (
          <button key={p.id} type="button" className={matchesPreset(stack, schema, p.id) ? 'seg-on' : ''} aria-pressed={matchesPreset(stack, schema, p.id)} title={p.description} onClick={() => ctl.applyPreset(p.id)}>
            {p.name}
          </button>
        ))}
      </div>
      <ul className="layer-stack" aria-label="Layer stack, top layer first">
        {top.map((e, i) => {
          const def = defs.get(e.id);
          if (!def) return null;
          const ok = overlayAvailable(def, available);
          return (
            <li key={e.id} className={`layer-row${e.visible ? '' : ' layer-off'}${stack.solo === e.id ? ' layer-solo' : ''}`} data-overlay={e.id}>
              <div className="layer-head">
                <label className="layer-name">
                  <input type="checkbox" checked={e.visible} onChange={(ev) => ctl.updateStack((s) => setVisible(s, e.id, ev.target.checked))} aria-label={`Show ${def.label}`} />
                  <span className="layer-swatch" style={{ background: `var(--overlay-${def.token})` }} aria-hidden="true" />
                  {def.label}
                </label>
                <span className="layer-btns">
                  <button type="button" className="icon-btn" aria-label={`Move ${def.label} up`} disabled={i === 0} onClick={() => ctl.updateStack((s) => moveBy(s, e.id, 1))}>↑</button>
                  <button type="button" className="icon-btn" aria-label={`Move ${def.label} down`} disabled={i === top.length - 1} onClick={() => ctl.updateStack((s) => moveBy(s, e.id, -1))}>↓</button>
                  <button type="button" className={`icon-btn${stack.solo === e.id ? ' on' : ''}`} aria-pressed={stack.solo === e.id} aria-label={`Solo ${def.label}`} onClick={() => ctl.updateStack((s) => toggleSolo(s, e.id))}>S</button>
                </span>
              </div>
              {e.visible && !ok && <p className="note">Solve to see {def.label.toLowerCase()}.</p>}
              {e.visible && (
                <div className="layer-ctl">
                  <label>Opacity <input type="range" min={0} max={1} step={0.05} value={e.opacity} onChange={(ev) => ctl.updateStack((s) => setOpacity(s, e.id, Number(ev.target.value)))} aria-label={`${def.label} opacity`} /></label>
                  <label>Blend{' '}
                    <select value={e.blend} onChange={(ev) => ctl.updateStack((s) => setBlend(s, e.id, ev.target.value as Blend))} aria-label={`${def.label} blend mode`}>
                      {BLENDS.map((b) => <option key={b} value={b}>{b}</option>)}
                    </select>
                  </label>
                </div>
              )}
              {drawn.has(e.id) && ok && <OverlayLegend def={def} />}
            </li>
          );
        })}
        <li className={`layer-row${stack.base.visible ? '' : ' layer-off'}${stack.solo === 'base' ? ' layer-solo' : ''}`} data-overlay="base">
          <div className="layer-head">
            <label className="layer-name">
              <input type="checkbox" checked={stack.base.visible} onChange={(ev) => ctl.updateStack((s) => setVisible(s, 'base', ev.target.checked))} aria-label="Show base layer" />
              Base layer (chosen above)
            </label>
            <span className="layer-btns">
              <button type="button" className={`icon-btn${stack.solo === 'base' ? ' on' : ''}`} aria-pressed={stack.solo === 'base'} aria-label="Solo base layer" onClick={() => ctl.updateStack((s) => toggleSolo(s, 'base'))}>S</button>
            </span>
          </div>
          {stack.base.visible && (
            <div className="layer-ctl">
              <label>Opacity <input type="range" min={0} max={1} step={0.05} value={stack.base.opacity} onChange={(ev) => ctl.updateStack((s) => setOpacity(s, 'base', Number(ev.target.value)))} aria-label="Base layer opacity" /></label>
            </div>
          )}
        </li>
      </ul>
      {anyAnimated && (
        <label className="check"><input type="checkbox" checked={animate} onChange={(e) => ctl.setAnimate(e.target.checked)} /> Animated streaks (2D map)</label>
      )}
      {isDrawn(stack, 'base') || stack.solo ? null : <p className="note">The base layer is hidden.</p>}
    </div>
  );
}

/** Per-layer legend: the colour from the theme token and what the layer shows. Text comes from the schema declaration. */
function OverlayLegend({ def }: { def: import('wb-client').OverlayDef }) {
  const what =
    def.kind === 'vector' ? `Arrow length shows speed${def.unit_hint ? ` (${def.unit_hint})` : ''}; the arrow points where it blows or flows.`
    : def.kind === 'lines' ? 'Line width shows the water carried (discharge).'
    : def.mode === 'fill' ? 'Filled where the layer is present.'
    : 'Drawn along the edge of the area.';
  return <p className="layer-legend"><span className="layer-swatch" style={{ background: `var(--overlay-${def.token})` }} aria-hidden="true" /> {what}</p>;
}
