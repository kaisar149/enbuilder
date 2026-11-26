import { isJobRunning } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function SolvePanel({ app, ctl }: PanelProps) {
  const running = useStore(app.store, isJobRunning);
  const job = useStore(app.store, (s) => s.jobs.current);
  const world = useStore(app.store, (s) => s.world);
  const err = useStore(app.store, (s) => s.lastError);
  const pct = Math.round((job?.status === 'completed' ? 1 : job?.fraction ?? 0) * 100);
  const showBar = running || job?.status === 'completed';
  const status = running
    ? `${job?.stage || 'starting'} (${pct}%)`
    : job?.status === 'failed' ? 'The solve failed.'
    : job?.status === 'cancelled' ? 'Solve cancelled.'
    : world?.solved ? (world.stale ? 'Results are out of date. Solve again.' : 'Solved.') : 'Only elevation is available until you solve.';
  return (
    <div className="stack">
      <div className="row">
        <button className="btn primary" disabled={running} onClick={() => void ctl.params.solve().catch(() => undefined)}>Solve</button>
        <button className="btn" disabled={!running} onClick={() => void app.command({ type: 'cancel' }).catch(() => undefined)}>Cancel</button>
      </div>
      {showBar && (
        <div className="progress" role="progressbar" aria-label="Solve progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
          <div className="progress-fill" style={{ width: `${pct}%` }} />
        </div>
      )}
      <p className="note" role="status">{status}</p>
      {err && <p className="note error" role="alert">{err}</p>}
    </div>
  );
}
