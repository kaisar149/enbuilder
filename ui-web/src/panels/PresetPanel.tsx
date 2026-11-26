import { fmtParam, presetStatus, prettyName } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

export function PresetPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const base = useStore(ctl.params.ui, (s) => s.basePreset);
  useStore(ctl.params.ui, (s) => s.drafts); // re-render while the user edits
  if (!schema || !world) return <p className="note">Waiting for the world.</p>;
  const st = presetStatus(schema, world.params, world.preset, base);
  const shown = st.base ?? '';
  const defs = new Map(schema.params.map((p) => [p.name, p]));
  return (
    <div className="stack">
      <label className="field">
        <span>Preset</span>
        <select value={shown} onChange={(e) => void ctl.params.applyPreset(e.target.value).catch(() => undefined)}>
          {schema.presets.map((p) => <option key={p.name} value={p.name} title={p.description}>{prettyName(p.name)}</option>)}
        </select>
      </label>
      <p className="note">{schema.presets.find((p) => p.name === shown)?.description}</p>
      <p className={`dirty ${st.dirty ? 'is-dirty' : 'is-clean'}`} role="status" data-dirty={st.dirty}>
        {st.dirty ? `Changed from ${prettyName(shown)}: ${st.diffs.length} setting${st.diffs.length === 1 ? '' : 's'}` : `Matches ${prettyName(shown)}`}
      </p>
      {st.dirty && (
        <ul className="diffs" aria-label="Changes from the preset">
          {st.diffs.map((d) => {
            const def = defs.get(d.name)!;
            return <li key={d.name}>{d.name.replace(/_/g, ' ')}: {fmtParam(def, d.expected)} to {fmtParam(def, d.current)} {def.unit}</li>;
          })}
        </ul>
      )}
      {st.dirty && <button type="button" className="btn" onClick={() => void ctl.params.applyPreset(shown).catch(() => undefined)}>Back to {prettyName(shown)}</button>}
    </div>
  );
}
