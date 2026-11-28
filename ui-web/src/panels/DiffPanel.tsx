// Part 9g: the Diff panel. Version pickers, the field to compare, play / time slider, before / after, colour range and the summary. Every number
// comes from `get_diff` / `get_diff_summary`; the panel only lays them out.
import { revealedCount, type CompareMode, type DiffSummaryRow } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');
const MODES: { id: CompareMode; label: string; hint: string }[] = [
  { id: 'diff', label: 'Change', hint: 'Difference after minus before, centred on zero' },
  { id: 'before', label: 'Before', hint: 'The earlier version' },
  { id: 'after', label: 'After', hint: 'The later version' },
  { id: 'swipe', label: 'Swipe', hint: 'After on one side of the handle, before on the other' },
];

export function fmtChange(row: Pick<DiffSummaryRow, 'display'>, raw: number): string {
  const v = raw * row.display.scale;
  const a = Math.abs(v);
  const t = a === 0 ? '0' : a >= 1e5 || a < 1e-2 ? v.toExponential(1) : a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
  return `${t} ${row.display.unit}`.trim();
}

export function DiffPanel({ ctl }: PanelProps) {
  const dc = ctl.diff;
  const u = useStore(dc.ui, (s) => s);
  const d = useStore(dc.data.store, (s) => s);
  const field = dc.diffField();
  const rows = d.summary?.fields ?? [];
  const row = rows.find((r) => r.field === field);
  const versions = d.versions?.versions ?? [];
  const shown = d.diff && d.diff.field === field ? d.diff : null;
  const label = (v: { version: number; label: string }) => `#${v.version}  ${v.label}`;
  return (
    <div className="stack">
      <button type="button" className={`btn${u.on ? ' btn-on' : ''}`} aria-pressed={u.on} onClick={() => void dc.setOn(!u.on)}>{u.on ? 'Hide diff' : 'Show diff'}</button>
      {!u.on && <p className="note">After an edit and a refresh, press Diff to see what changed between two stored versions.</p>}
      {u.on && versions.length < 2 && <p className="note">Only one version is stored. Paint something and refresh, then compare.</p>}
      {u.on && versions.length > 0 && (
        <>
          <label className="field"><span>Before</span>
            <select aria-label="Before version" value={d.from ?? ''} onChange={(e) => void dc.selectPair(Number(e.target.value), d.to as number)}>
              {versions.map((v) => <option key={v.version} value={v.version}>{label(v)}</option>)}
            </select>
          </label>
          <label className="field"><span>After</span>
            <select aria-label="After version" value={d.to ?? ''} onChange={(e) => void dc.selectPair(d.from as number, Number(e.target.value))}>
              {versions.map((v) => <option key={v.version} value={v.version}>{label(v)}</option>)}
            </select>
          </label>
          <button type="button" className="btn" onClick={() => void dc.followLatest()}>Latest two versions</button>
          {d.error && <p className="note" role="alert">{d.error}</p>}
          {d.summary?.no_change && <p className="note" role="status" data-testid="diff-no-change">No change between these versions.</p>}
          {d.summary && !d.summary.no_change && (
            <>
              <label className="field"><span>Field</span>
                <select aria-label="Field to compare" value={field ?? ''} onChange={(e) => void dc.setField(e.target.value)}>
                  {rows.map((r) => <option key={r.field} value={r.field}>{cap(r.field)} ({r.changed_cells})</option>)}
                </select>
              </label>
              <div className="seg seg-wrap" role="group" aria-label="What the map shows">
                {MODES.map((m) => <button key={m.id} type="button" title={m.hint} className={u.mode === m.id ? 'seg-on' : ''} aria-pressed={u.mode === m.id} onClick={() => dc.setMode(m.id)}>{m.label}</button>)}
              </div>
              {(u.mode === 'before' || u.mode === 'after') && <button type="button" className="btn" onClick={() => dc.flip()}>Flip to {u.mode === 'after' ? 'before' : 'after'}</button>}
              {u.mode === 'swipe' && (
                <label className="field"><span>Swipe <output>{Math.round(u.swipe * 100)}%</output></span>
                  <input type="range" aria-label="Swipe position" min={0} max={1000} value={Math.round(u.swipe * 1000)} onChange={(e) => dc.setSwipe(Number(e.target.value) / 1000)} />
                </label>
              )}
              {u.mode === 'diff' && (
                <>
                  <div className="row">
                    <button type="button" className="btn" onClick={() => (u.playing ? dc.stop() : dc.play())}>{u.playing ? 'Pause' : 'Play ripple'}</button>
                    <output aria-label="Cells reached" data-testid="diff-reached">{shown ? `${revealedCount(shown, u.t)} of ${shown.summary.changed_cells} cells` : ''}</output>
                  </div>
                  <label className="field"><span>Time</span>
                    <input type="range" aria-label="Propagation time" min={0} max={1000} value={Math.round(u.t * 1000)} onChange={(e) => dc.setT(Number(e.target.value) / 1000)} />
                  </label>
                  <label className="field"><span>Colour range <output>{Math.round(u.rangeFrac * 100)}% of largest change</output></span>
                    <input type="range" aria-label="Colour range" min={1} max={100} value={Math.round(u.rangeFrac * 100)} onChange={(e) => dc.setRange(Number(e.target.value) / 100)} />
                  </label>
                </>
              )}
              <label className="check"><input type="checkbox" checked={u.hideSmall} onChange={(e) => void dc.setHideSmall(e.target.checked)} /> Hide changes under 1% of the largest</label>
              {shown && row && (
                <dl className="kv" data-testid="diff-summary">
                  <dt>Cells changed</dt><dd>{shown.summary.changed_cells} of {shown.nCells}{shown.hiddenCells > 0 ? ` (${shown.hiddenCells} tiny ones hidden)` : ''}</dd>
                  <dt>Largest change</dt><dd>{fmtChange(row, shown.summary.peak_abs)}</dd>
                  <dt>Reached</dt><dd>{Math.round(shown.summary.reach_km).toLocaleString()} km from {shown.frontKind === 'edit' ? 'the edit' : 'the biggest change'}</dd>
                  <dt>Edited cells</dt><dd>{d.summary.edit_cells}</dd>
                </dl>
              )}
              <table className="table" aria-label="Changes per field">
                <thead><tr><th>Field</th><th>Cells</th><th>Largest</th><th>Reach km</th></tr></thead>
                <tbody>
                  {rows.slice(0, 14).map((r) => (
                    <tr key={r.field} className={r.field === field ? 'row-on' : ''}>
                      <td><button type="button" className="link-btn" onClick={() => void dc.setField(r.field)}>{cap(r.field)}</button></td>
                      <td>{r.changed_cells}</td><td>{fmtChange(r, r.peak_abs)}</td><td>{Math.round(r.reach_km).toLocaleString()}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </>
      )}
    </div>
  );
}
