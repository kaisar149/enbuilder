import { useState } from 'react';
import { inspectorModel } from '../layers/inspector';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

/**
 * The cell inspector. It lists exactly the values `get_cell` returned, grouped by the module that produces each field, with the twelve season
 * values where the core sent them. "Pin" keeps one cell so a second click can be compared with it. No value is computed here.
 */
export function InspectorPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const info = useStore(ctl.ui, (s) => s.cellInfo);
  const picked = useStore(ctl.ui, (s) => s.pickedCell);
  const pinned = useStore(ctl.ui, (s) => s.pinnedInfo);
  const pinnedCell = useStore(ctl.ui, (s) => s.pinnedCell);
  const error = useStore(ctl.ui, (s) => s.cellError);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  if (error) return <p className="note error" role="alert">{error}</p>;
  if (picked === null) return <p className="note">Click the map to inspect a cell. Pin a cell, then click another to compare.</p>;
  if (!info) return <p className="note">Reading cell {picked}.</p>;
  const model = inspectorModel(schema, info, pinned && pinned.cell !== info.cell ? pinned : null);
  const isPinned = pinnedCell === info.cell;
  return (
    <div className="stack inspector">
      <div className="inspector-head">
        <p className="cell-title">{model.title}</p>
        <button type="button" className="btn" aria-pressed={isPinned} onClick={() => void ctl.pinCell(isPinned ? null : info.cell)}>{isPinned ? 'Unpin' : 'Pin this cell'}</button>
      </div>
      {model.subtitle && <p className="note">{model.subtitle}</p>}
      {model.stale && <p className="note">Values are out of date. Solve again to refresh.</p>}
      {pinned && pinnedCell !== info.cell && (
        <p className="note">Comparing with pinned {model.pinnedTitle}. <button type="button" className="link-btn" onClick={() => void ctl.pinCell(null)}>Clear pin</button></p>
      )}
      {model.groups.map((g) => (
        <section key={g.id} className="inspector-group" aria-label={g.title}>
          <h3>{g.title}</h3>
          <table className="facts-table">
            {model.pinnedTitle && <thead><tr><th scope="col">Field</th><th scope="col">This cell</th><th scope="col">Pinned</th><th scope="col">Change</th></tr></thead>}
            <tbody>
              {g.rows.map((r) => (
                <FactRows key={r.name} r={r} compare={!!model.pinnedTitle} open={!!open[r.name]} toggle={() => setOpen((o) => ({ ...o, [r.name]: !o[r.name] }))} />
              ))}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}

function FactRows({ r, compare, open, toggle }: { r: import('../layers/inspector').InspectorRow; compare: boolean; open: boolean; toggle: () => void }) {
  return (
    <>
      <tr data-field={r.name}>
        <th scope="row">
          {r.seasons ? <button type="button" className="link-btn" aria-expanded={open} onClick={toggle}>{r.label}</button> : r.label}
        </th>
        <td>{r.text}</td>
        {compare && (<><td>{r.other?.text ?? ''}</td><td>{r.other?.delta ?? ''}</td></>)}
      </tr>
      {r.seasons && open && (
        <tr className="seasons-row" data-field-seasons={r.name}>
          <td colSpan={compare ? 4 : 2}>
            <ol className="seasons" aria-label={`${r.label} by season`}>{r.seasons.text.map((t, i) => <li key={i}><span>S{i + 1}</span>{t}</li>)}</ol>
            <p className="note">Lowest {r.seasons.min}, highest {r.seasons.max} {r.unit}</p>
          </td>
        </tr>
      )}
    </>
  );
}
