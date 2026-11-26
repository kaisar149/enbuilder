import { layerChoices } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

/** The layer dropdown. Every option comes from `get_schema`; nothing here names a field. */
export function LayersPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const layer = useStore(ctl.ui, (s) => s.layer);
  const choices = layerChoices(schema, world);
  const featured = choices.filter((c) => c.group === 'featured');
  const other = choices.filter((c) => c.group === 'other');
  const current = choices.find((c) => c.field === layer);
  const opt = (c: (typeof choices)[number]) => (
    <option key={c.field} value={c.field} disabled={!c.available} title={c.note}>
      {c.label}{c.available ? '' : ' (solve first)'}
    </option>
  );
  return (
    <div className="stack">
      <label className="field">
        <span>Show on map</span>
        <select value={layer ?? ''} onChange={(e) => void ctl.selectLayer(e.target.value)} disabled={choices.length === 0}>
          <optgroup label="Main layers">{featured.map(opt)}</optgroup>
          {other.length > 0 && <optgroup label="More layers">{other.map(opt)}</optgroup>}
        </select>
      </label>
      {current && <p className="note">{current.note}</p>}
    </div>
  );
}
