import { historyRedo, historyUndo, type HistoryEntry } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

/** One line of detail per step kind (Part 9f): strokes and locks say how many cells, realism and parameter steps list their changes. */
export function historyDetail(h: HistoryEntry): string {
  if (h.kind === 'stroke' || h.kind === 'lock') return `${h.cells ?? 0} cells`;
  return h.changes.map((c) => `${c.name}: ${c.old} to ${c.new}`).join('\n');
}

export function HistoryPanel({ app, ctl }: PanelProps) {
  const world = useStore(app.store, (s) => s.world);
  if (!world) return <p className="note">Waiting for the world.</p>;
  const undo = historyUndo(world);
  const redo = historyRedo(world);
  return (
    <div className="stack">
      <div className="row">
        <button type="button" className="btn" disabled={!world.can_undo} onClick={() => void ctl.params.undo().catch(() => undefined)}>Undo</button>
        <button type="button" className="btn" disabled={!world.can_redo} onClick={() => void ctl.params.redo().catch(() => undefined)}>Redo</button>
      </div>
      {undo.length + redo.length === 0 ? <p className="note">No changes yet.</p> : (
        <ol className="history" aria-label="Edit history">
          {undo.map((h, i) => <li key={`u${i}`} data-kind={h.kind ?? 'params'} className={`kind-${h.kind ?? 'params'}${i === undo.length - 1 ? ' is-latest' : ''}`} title={historyDetail(h)}>{h.label}</li>)}
          {[...redo].reverse().map((h, i) => <li key={`r${i}`} data-kind={h.kind ?? 'params'} className={`undone kind-${h.kind ?? 'params'}`} title="Undone; Redo brings it back">{h.label}</li>)}
        </ol>
      )}
    </div>
  );
}
