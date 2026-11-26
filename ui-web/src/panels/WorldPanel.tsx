import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function WorldPanel({ app }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  if (!world || !schema) return <p className="note">Waiting for the world.</p>;
  return (
    <div className="stack">
      <dl className="facts">
        <dt>Shape</dt><dd>{world.domain.kind}</dd>
        <dt>Cells</dt><dd>{world.domain.n_cells.toLocaleString()}</dd>
      </dl>
    </div>
  );
}
