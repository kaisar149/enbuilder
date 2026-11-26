import { createRoot } from 'react-dom/client';
import { WbApp } from 'wb-client';
import { App } from './App';
import './theme.css';
import './app.css';

/** `?server=127.0.0.1:8787` overrides the address; otherwise the page talks to the server it was served from (or the Vite proxy in dev). */
function wsUrl(): string {
  const q = new URLSearchParams(location.search).get('server');
  if (q) return `ws://${q}/ws`;
  return `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;
}

const root = createRoot(document.getElementById('root')!);
root.render(<p className="boot">Connecting to the world server.</p>);
WbApp.connect(wsUrl())
  .then((app) => root.render(<App app={app} />))
  .catch((e: Error) => root.render(<p className="boot error" role="alert">Cannot reach wb-server at {wsUrl()}. Start it with <code>wb-server --preset earthlike</code> and reload. ({e.message})</p>));
