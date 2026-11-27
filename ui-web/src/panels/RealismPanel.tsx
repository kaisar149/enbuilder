// Part 9f: realism sliders (one per output field, from `schema.edit.realism`) and the view switch (blend, physics only, painted).
import { useState } from 'react';
import type { FieldViewName, OutputField } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');
const VIEWS: { id: FieldViewName; label: string; hint: string }[] = [
  { id: 'blended', label: 'Blend', hint: 'Painting and physics mixed by the realism sliders' },
  { id: 'computed', label: 'Physics only', hint: 'What the solver computed, ignoring paint' },
  { id: 'painted', label: 'Painted', hint: 'What you painted, ignoring the sliders' },
];

export function RealismPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const fieldView = useStore(ctl.ui, (s) => s.fieldView);
  const layer = useStore(ctl.ui, (s) => s.layer);
  const [draft, setDraft] = useState<Partial<Record<OutputField | 'all', number>>>({});
  const r = schema?.edit?.realism;
  const info = world?.edit;
  if (!r || !info) return <p className="note">Waiting for the world.</p>;
  const commit = (f: OutputField | 'all') => {
    const v = draft[f];
    if (v === undefined) return;
    setDraft((d) => { const { [f]: _gone, ...rest } = d; return rest; });
    void ctl.edit.setRealism(f, v).catch(() => undefined);
  };
  const slider = (f: OutputField | 'all', label: string, value: number, extra?: string) => (
    <label className="field" key={f}>
      <span>{label} <output>{Math.round(value)}</output>{extra && <span className="param-unit"> {extra}</span>}</span>
      <input
        type="range" aria-label={`${label} realism`} min={r.min} max={r.max} step={1} value={value}
        onChange={(e) => setDraft((d) => ({ ...d, [f]: Number(e.target.value) }))} onPointerUp={() => commit(f)} onKeyUp={() => commit(f)} onBlur={() => commit(f)}
      />
    </label>
  );
  const canView = !!layer && (r.fields as string[]).includes(layer);
  return (
    <div className="stack">
      <p className="note">0 shows what you painted. 100 shows only the physics. In between blends them.</p>
      {slider('all', 'All fields', draft.all ?? Math.round(r.fields.reduce((a, f) => a + info.realism[f], 0) / r.fields.length))}
      {r.fields.map((f) => {
        const layerId = schema?.edit?.layers.find((l) => l.output === f)?.id;
        const painted = layerId ? info.paint[layerId] ?? 0 : 0;
        const locked = info.locks[f] ?? 0;
        const extra = [painted ? `${painted} painted` : '', locked ? `${locked} locked` : ''].filter(Boolean).join(', ');
        return slider(f, cap(f), draft[f] ?? info.realism[f], extra);
      })}
      <div className="seg seg-wrap" role="group" aria-label="Which values the map shows">
        {VIEWS.map((v) => (
          <button key={v.id} type="button" title={v.hint} className={fieldView === v.id ? 'seg-on' : ''} aria-pressed={fieldView === v.id} disabled={v.id !== 'blended' && !canView} onClick={() => ctl.setFieldView(v.id)}>{v.label}</button>
        ))}
      </div>
      {!canView && <p className="note">Choose elevation, temperature, rainfall or biome as the map layer to compare views.</p>}
    </div>
  );
}
