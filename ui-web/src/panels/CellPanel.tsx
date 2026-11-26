import { displayValue, metaOf, prettyName } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function CellPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const info = useStore(ctl.ui, (s) => s.cellInfo);
  const picked = useStore(ctl.ui, (s) => s.pickedCell);
  const error = useStore(ctl.ui, (s) => s.cellError);
  if (error) return <p className="note error" role="alert">{error}</p>;
  if (picked === null) return <p className="note">Click the map to inspect a cell.</p>;
  if (!info) return <p className="note">Reading cell {picked}.</p>;
  const rows = Object.entries(info.values).map(([name, v]) => {
    const meta = metaOf(schema, name);
    return { name, text: meta ? `${fmt(displayValue(meta, v))} ${meta.display.unit}`.trim() : fmt(v) };
  });
  return (
    <div className="stack">
      <p className="cell-title">Cell {info.cell}{info.lat_deg !== undefined && info.lon_deg !== undefined ? `, ${info.lat_deg.toFixed(1)}°, ${info.lon_deg.toFixed(1)}°` : ''}</p>
      {info.biome_name && <p className="note">{info.biome_name}</p>}
      {info.stale && <p className="note">Values are out of date.</p>}
      <dl className="facts">{rows.map((r) => <FactRow key={r.name} label={prettyName(r.name)} value={r.text} />)}</dl>
    </div>
  );
}

const fmt = (v: number) => (Number.isFinite(v) ? (Math.abs(v) >= 1000 || (v !== 0 && Math.abs(v) < 0.01) ? v.toExponential(2) : v.toFixed(2)) : 'none');

function FactRow({ label, value }: { label: string; value: string }) {
  return (<><dt>{label}</dt><dd>{value}</dd></>);
}
