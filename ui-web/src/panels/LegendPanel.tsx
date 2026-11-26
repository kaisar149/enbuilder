import { legendFor, selectField, layerDecl } from 'wb-client';
import { cssGradient } from '../viewport/colormaps';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function LegendPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const layer = useStore(ctl.ui, (s) => s.layer);
  const field = useStore(app.store, (s) => (layer ? selectField(s, layer) : undefined));
  if (!schema || !field) return <p className="note">No layer to describe yet.</p>;
  const decl = layerDecl(schema, field);
  const legend = legendFor(schema, decl, field);
  if (legend.type === 'categories') {
    return (
      <ul className="swatches" aria-label="Legend">
        {legend.items.map((it) => (
          <li key={it.id}>
            <span className="swatch" style={{ background: it.color ? `rgb(${it.color.join(' ')})` : 'var(--swatch-unknown)' }} />
            {it.name}
          </li>
        ))}
      </ul>
    );
  }
  return (
    <div className="legend" aria-label="Legend">
      <div className="legend-bar" style={{ background: cssGradient(legend.colormap) }} />
      <div className="legend-labels"><span>{legend.loLabel}</span><span className="legend-unit">{legend.unit}</span><span>{legend.hiLabel}</span></div>
    </div>
  );
}
