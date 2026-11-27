import type { WbApp } from 'wb-client';
import type { MapController } from '../state/controller';
import { useStore } from '../state/hooks';

/** What the map's result currently is: a refresh running (preview or refined), or the quality of the solve it shows. Always says which one (Part 9f). */
export function refreshBadge(job: { kind?: string; mode?: string; status: string } | null, quality: string | null | undefined): { text: string; running: boolean } | null {
  if (job && job.status === 'running') {
    if (job.kind === 'refresh') return { text: job.mode === 'preview' ? 'Refining: quick preview' : job.mode === 'refined' ? 'Refining: refined pass' : 'Refining', running: true };
    if (job.kind === 'solve') return { text: 'Solving', running: true };
    return null;
  }
  if (quality === 'preview') return { text: 'Preview quality', running: false };
  if (quality === 'refined') return { text: 'Refined', running: false };
  return null;
}

export function RefreshBadge({ app, ctl }: { app: WbApp; ctl: MapController }) {
  const job = useStore(app.store, (s) => s.jobs.current);
  const quality = useStore(app.store, (s) => s.world?.edit?.quality);
  const err = useStore(ctl.edit.store, (s) => (s.phase === 'error' ? s.error : null));
  const b = refreshBadge(job, quality);
  if (err) return <span className="badge badge-error" role="alert">{err}</span>;
  if (!b) return null;
  return <span className={`badge${b.running ? ' badge-run' : ''}`} role="status" data-testid="refresh-badge">{b.text}</span>;
}
