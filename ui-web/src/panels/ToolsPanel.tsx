// Part 9f: the Tools panel. Everything (tool list, groups, amounts, brush limits, falloffs, refresh modes) comes from `schema.edit`;
// no tool is named here. The panel only edits brush settings in the EditController and starts refreshes.
import { useMemo } from 'react';
import type { EditSchema, OutputField, PaintLayerId, ToolDef } from 'wb-client';
import { useStore } from '../state/hooks';
import type { PanelProps } from './components';

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

/** Tools grouped for display: by the paint layer they edit (labels from the schema), then erase, then locks. */
export function groupTools(edit: EditSchema): { title: string; tools: ToolDef[] }[] {
  const groups: { title: string; tools: ToolDef[] }[] = [];
  const add = (title: string, t: ToolDef) => {
    let g = groups.find((x) => x.title === title);
    if (!g) groups.push((g = { title, tools: [] }));
    g.tools.push(t);
  };
  for (const t of edit.tools) {
    if (t.kind === 'lock') add('Locks', t);
    else if (t.kind === 'erase') add('Eraser', t);
    else add(edit.layers.find((l) => l.id === t.layer)?.label ?? cap(String(t.layer)), t);
  }
  return groups;
}

export function ToolsPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const job = useStore(app.store, (s) => s.jobs.current);
  const editOn = useStore(ctl.ui, (s) => s.editOn);
  const st = useStore(ctl.edit.store, (s) => s);
  const edit = schema?.edit;
  const groups = useMemo(() => (edit ? groupTools(edit) : []), [edit]);
  if (!edit) return <p className="note">This server has no editing tools.</p>;
  const tool = ctl.edit.tools.find((t) => t.id === st.toolId);
  const def = tool?.def ?? null;
  const b = st.brush;
  const running = !!job && job.status === 'running';
  const solved = !!world?.solved;
  const amount = def?.amount ?? null;
  const amountValue = b.amount ?? amount?.default ?? 0;
  return (
    <div className="stack">
      <button type="button" className={`btn${editOn ? ' primary' : ''}`} aria-pressed={editOn} onClick={() => ctl.setEditMode(!editOn)}>
        {editOn ? 'Painting is on' : 'Paint on the map'}
      </button>
      <p className="note">{editOn ? 'Drag on the map. Shift-drag or right-drag moves the view. Escape cancels a stroke.' : 'Turn painting on, then drag on the map.'}</p>

      {groups.map((g) => (
        <div key={g.title} className="tool-group" role="group" aria-label={g.title}>
          <span className="tool-group-title">{g.title}</span>
          <div className="layer-btns">
            {g.tools.map((t) => (
              <button key={t.id} type="button" className={`btn tiny${st.toolId === t.id ? ' primary' : ''}`} aria-pressed={st.toolId === t.id} title={t.hint} onClick={() => ctl.edit.setTool(t.id)}>{t.label}</button>
            ))}
          </div>
        </div>
      ))}

      <div className="stack tool-brush">
        <label className="field">
          <span>Radius <output>{Math.round(b.radiusKm)} km</output></span>
          <input type="range" aria-label="Brush radius" min={edit.brush.radius_min_km} max={edit.brush.radius_max_km} step={50} value={b.radiusKm} onChange={(e) => ctl.edit.setBrush({ radiusKm: Number(e.target.value) })} />
        </label>
        <label className="field">
          <span>Strength <output>{Math.round(b.strength * 100)}%</output></span>
          <input type="range" aria-label="Brush strength" min={0.05} max={1} step={0.05} value={b.strength} onChange={(e) => ctl.edit.setBrush({ strength: Number(e.target.value) })} />
        </label>
        <label className="field">
          <span>Falloff</span>
          <select aria-label="Brush falloff" value={b.falloff} onChange={(e) => ctl.edit.setBrush({ falloff: e.target.value })}>
            {edit.brush.falloffs.map((f) => <option key={f} value={f}>{cap(f)}</option>)}
          </select>
        </label>
        {amount && (
          <label className="field">
            <span>Amount <output>{amountValue} {amount.unit}</output></span>
            <input type="range" aria-label="Brush amount" min={amount.min} max={amount.max} step={(amount.max - amount.min) / 100} value={amountValue} onChange={(e) => ctl.edit.setBrush({ amount: Number(e.target.value) })} />
          </label>
        )}
        {def?.needs_value && (
          <label className="field">
            <span>Biome</span>
            <select aria-label="Biome to paint" value={b.biome ?? ''} onChange={(e) => ctl.edit.setBrush({ biome: e.target.value === '' ? null : Number(e.target.value) })}>
              <option value="">Choose a biome</option>
              {(schema?.biomes ?? []).map((bm) => <option key={bm.id} value={bm.id}>{bm.name}</option>)}
            </select>
          </label>
        )}
        {def?.kind === 'lock' && (
          <label className="field">
            <span>Field</span>
            <select aria-label="Field to lock" value={b.lockField} onChange={(e) => ctl.edit.setBrush({ lockField: e.target.value as OutputField })}>
              {edit.lockable.map((f) => <option key={f} value={f}>{cap(f)}</option>)}
            </select>
          </label>
        )}
        {def?.kind === 'erase' && (
          <label className="field">
            <span>Erase paint of</span>
            <select aria-label="Layer to erase" value={b.eraseLayer} onChange={(e) => ctl.edit.setBrush({ eraseLayer: e.target.value as PaintLayerId })}>
              {edit.layers.map((l) => <option key={l.id} value={l.id}>{l.label}</option>)}
            </select>
          </label>
        )}
      </div>

      <div className="tool-group" role="group" aria-label="Recalculate">
        <span className="tool-group-title">Recalculate</span>
        <div className="layer-btns">
          {edit.refresh_modes.map((m) => (
            <button key={m.id} type="button" className="btn tiny" title={m.description} disabled={running || (m.id !== 'full' && !solved)} onClick={() => void app.refreshAndWait(m.id).catch(() => undefined)}>{m.label}</button>
          ))}
        </div>
        {st.lastDetail && typeof st.lastDetail.skipped_locked === 'number' && st.lastDetail.skipped_locked > 0 && (
          <p className="note" role="status">{st.lastDetail.skipped_locked} locked cells were left unchanged.</p>
        )}
      </div>
    </div>
  );
}
