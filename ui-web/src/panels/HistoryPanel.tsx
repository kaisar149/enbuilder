import { historyRedo, historyUndo } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

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
        <ol className="history" aria-label="Parameter history">
          {undo.map((h, i) => <li key={`u${i}`} className={i === undo.length - 1 ? 'is-latest' : ''} title={h.changes.map((c) => `${c.name}: ${c.old} to ${c.new}`).join('\n')}>{h.label}</li>)}
          {[...redo].reverse().map((h, i) => <li key={`r${i}`} className="undone" title="Undone; Redo brings it back">{h.label}</li>)}
        </ol>
      )}
    </div>
  );
}
