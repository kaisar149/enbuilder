import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function WorldPanel({ app }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  if (!world || !schema) return <p className="note">Waiting for the world.</p>;
  const known = schema.presets.some((p) => p.name === world.preset);
  return (
    <div className="stack">
      <label className="field">
        <span>Preset</span>
        <select value={world.preset} onChange={(e) => void app.command({ type: 'load_preset', name: e.target.value }).catch(() => undefined)}>
          {!known && <option value={world.preset}>{world.preset}</option>}
          {schema.presets.map((p) => <option key={p.name} value={p.name} title={p.description}>{p.name.replace(/_/g, ' ')}</option>)}
        </select>
      </label>
      <dl className="facts">
        <dt>Shape</dt><dd>{world.domain.kind}</dd>
        <dt>Cells</dt><dd>{world.domain.n_cells.toLocaleString()}</dd>
      </dl>
    </div>
  );
}
