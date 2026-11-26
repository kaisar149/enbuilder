import { paramGroups } from 'wb-client';
import { useStore } from '../state/hooks';
import { ParamControl } from './ParamControl';
import type { PanelProps } from './components';

/** Renders every schema group. Adding a parameter in Rust adds a control here with no UI change. Advanced groups start collapsed. */
export function ParameterPanel({ app, ctl }: PanelProps) {
  const schema = useStore(app.store, (s) => s.schema);
  const world = useStore(app.store, (s) => s.world);
  const ui = useStore(ctl.params.ui, (s) => s);
  if (!schema || !world) return <p className="note">Waiting for the world.</p>;
  const groups = paramGroups(schema);
  return (
    <div className="stack">
      {groups.map((g) => {
        const nonDefault = g.params.some((p) => (ui.drafts[p.name] ?? world.params[p.name]) !== p.default);
        return (
          <details key={g.group} className="group" open={!g.advanced} data-group={g.group}>
            <summary>{g.title}{g.advanced ? ' (advanced)' : ''}</summary>
            <div className="stack">
              {g.params.map((p) => (
                <ParamControl
                  key={p.name} def={p} value={ctl.params.value(p.name) ?? p.default} error={ui.errors[p.name]} busy={!!ui.sending[p.name]}
                  onEdit={(v) => ctl.params.edit(p.name, v)} onCommit={() => void ctl.params.commit(p.name)} onReset={() => void ctl.params.reset(p.name)}
                />
              ))}
              <button type="button" className="btn" disabled={!nonDefault} onClick={() => void ctl.params.resetGroup(g.group)}>Reset {g.title.toLowerCase()} to defaults</button>
            </div>
          </details>
        );
      })}
    </div>
  );
}
