import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

/** Options of the 3D view. UI-only state: nothing here is sent to the core. Hidden controls appear only in 3D. */
export function View3DPanel({ ctl }: PanelProps) {
  const ui = useStore(ctl.ui, (s) => s);
  const v = ui.view3d;
  if (ui.view === '2d') {
    return (
      <div className="stack">
        <button type="button" className="btn" onClick={() => ctl.setView('3d')}>Show 3D world</button>
        <p className="note">The same layer, season and selected cell are used in 3D.</p>
      </div>
    );
  }
  const b = ui.bench;
  return (
    <div className="stack">
      <label className="field">
        <span>Relief exaggeration: {v.exaggeration === 0 ? 'flat' : `x${v.exaggeration}`}</span>
        <input type="range" aria-label="Relief exaggeration" min={0} max={100} step={1} value={v.exaggeration} onChange={(e) => ctl.setView3d({ exaggeration: Number(e.target.value) })} />
      </label>
      <label className="field">
        <span>Lighting</span>
        <select aria-label="Lighting" value={v.lighting} onChange={(e) => ctl.setView3d({ lighting: e.target.value as 'flat' | 'sun' })}>
          <option value="flat">Flat (map colours)</option>
          <option value="sun">Sun from the star</option>
        </select>
      </label>
      {v.lighting === 'sun' && (
        <>
          <label className="check">
            <input type="checkbox" checked={v.dayNight} onChange={(e) => ctl.setView3d({ dayNight: e.target.checked })} /> Day and night (dark night side)
          </label>
          <label className="field">
            <span>Time of day: {Math.round(v.timeOfDay * 24)} h</span>
            <input type="range" aria-label="Time of day" min={0} max={1} step={0.01} value={v.timeOfDay} onChange={(e) => ctl.setView3d({ timeOfDay: Number(e.target.value) })} />
          </label>
        </>
      )}
      <div className="row">
        <button type="button" className="btn" onClick={() => ctl.resetCamera()}>Reset view</button>
        <button type="button" className="btn" disabled={b === 'running'} onClick={() => ctl.runBenchmark()}>{b === 'running' ? 'Measuring' : 'Measure frame rate'}</button>
      </div>
      <p className="note" aria-live="polite">
        Renderer: {ui.renderer ?? 'starting'}{ui.renderer === 'stub' ? ' (no WebGL: nothing is drawn)' : ''}.
        {ui.frame ? ` Last second: ${ui.frame.fps.toFixed(0)} draws/s, ${ui.frame.drawMs.toFixed(1)} ms each (CPU time).` : ''}
        {b && b !== 'running' ? ` Benchmark: ${b.meanFps.toFixed(0)} fps mean, 95% of frames under ${b.p95Ms.toFixed(0)} ms, worst ${b.worstMs.toFixed(0)} ms.` : ''}
      </p>
    </div>
  );
}
