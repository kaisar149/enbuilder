import { useEffect, useMemo } from 'react';
import type { WbApp } from 'wb-client';
import { MapView } from './MapView';
import { PANEL_COMPONENTS } from './panels/components';
import { PANEL_REGISTRY, resolveLayout, type PanelDef } from './panels/registry';
import { MapController } from './state/controller';
import { useStore } from './state/hooks';

function Dock({ name, panels, app, ctl }: { name: string; panels: PanelDef[]; app: WbApp; ctl: MapController }) {
  if (panels.length === 0) return null;
  return (
    <aside className={`dock dock-${name}`} aria-label={`${name} panels`}>
      {panels.map((p) => {
        const C = PANEL_COMPONENTS[p.component];
        return (
          <section key={p.id} className="panel" aria-labelledby={`panel-${p.id}`}>
            <h2 id={`panel-${p.id}`}>{p.title}</h2>
            {C ? <C app={app} ctl={ctl} /> : <p className="note error">Unknown panel component {p.component}.</p>}
          </section>
        );
      })}
    </aside>
  );
}

export function App({ app }: { app: WbApp }) {
  const ctl = useMemo(() => new MapController(app), [app]);
  useEffect(() => () => ctl.dispose(), [ctl]);
  const connection = useStore(app.store, (s) => s.connection);
  const layout = useMemo(() => resolveLayout(PANEL_REGISTRY), []);
  return (
    <div className="shell">
      <header className="bar">
        <h1>World builder</h1>
        <span className={`conn conn-${connection}`} role="status">{connection === 'open' ? 'Connected' : connection === 'closed' ? 'Disconnected. Restart wb-server and reload.' : 'Connecting'}</span>
      </header>
      <Dock name="left" panels={layout.left} app={app} ctl={ctl} />
      <main className="stage">
        <MapView app={app} ctl={ctl} />
        <Dock name="bottom" panels={layout.bottom} app={app} ctl={ctl} />
      </main>
      <Dock name="right" panels={layout.right} app={app} ctl={ctl} />
    </div>
  );
}
