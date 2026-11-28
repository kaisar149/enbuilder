// Part 9g: "Why did this change?". Renders the core's causal chain exactly as `explain` returned it: one row per link, in order. Nothing is inferred here.
import { explainRows } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function ExplainPanel({ ctl }: PanelProps) {
  const dc = ctl.diff;
  const on = useStore(dc.ui, (s) => s.on);
  const hl = useStore(dc.ui, (s) => s.highlight);
  const ex = useStore(dc.data.store, (s) => s.explanation);
  if (!on) return <p className="note">Turn on Diff, then click a cell to see why it changed.</p>;
  if (!ex) return <p className="note">Click a cell on the map. The chain of causes for the diffed field appears here.</p>;
  if (!ex.changed) return <p className="note" role="status">{ex.field.replace(/_/g, ' ')} did not change at cell {ex.cell} (or changed less than the threshold).</p>;
  const rows = explainRows(ex);
  return (
    <div className="stack">
      <ol className="chain" aria-label="Causal chain, effect first">
        {rows.map((r) => {
          const active = hl.length > 0 && hl[0] === r.cell && hl.length === r.highlight.length;
          return (
            <li key={r.index}>
              <button type="button" className={`chain-step${active ? ' chain-on' : ''}`} aria-pressed={active} title="Highlight the cells behind this step" onClick={() => ctl.diff.highlightStep(active ? null : r.index)}>
                <code>{r.line}</code>
                <span className="chain-cause">{r.cause}</span>
              </button>
              {r.modules.length > 0 && <div className="note">Modules: {r.modules.join(', ')}</div>}
              {r.anomalies.length > 0 && <div className="note">Anomalies: {r.anomalies.join(', ')}</div>}
              {r.also.length > 0 && <div className="note">Also changed: {r.also.join(', ')}</div>}
              {r.feedback.length > 0 && <div className="note">Feedback loop through {r.feedback.join(', ')}</div>}
            </li>
          );
        })}
      </ol>
      {ex.origin.length > 0 && <p className="note">Origin: {ex.origin.join(', ')}</p>}
      {ex.stopped && <p className="note">Chain stops: {ex.stopped}</p>}
      <p className="note">{ex.note}</p>
    </div>
  );
}
