// Starts the REAL wb-server binary on a free port (no mocks): tests talk to it over a real WebSocket.
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
export const repoRoot = resolve(here, '..', '..');

export function serverBinary(): string {
  const exe = process.platform === 'win32' ? 'wb-server.exe' : 'wb-server';
  const candidates = [process.env.WB_SERVER_BIN, resolve(repoRoot, 'target/release', exe), resolve(repoRoot, 'target/debug', exe)].filter(Boolean) as string[];
  const found = candidates.find((c) => existsSync(c));
  if (!found) throw new Error(`wb-server binary not found. Build it first: cargo build --release -p wb-server (looked in ${candidates.join(', ')})`);
  return found;
}

export interface RunningServer {
  url: string;
  wsUrl: string;
  port: number;
  stop(): Promise<void>;
}

/** `args` are extra wb-server flags (defaults: a small, fast world on a free port). */
export function startServer(args: string[] = ['--preset', 'earthlike', '--param', 'resolution=2']): Promise<RunningServer> {
  return new Promise((ok, fail) => {
    const p: ChildProcess = spawn(serverBinary(), ['--port', '0', ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    const timer = setTimeout(() => { p.kill(); fail(new Error('wb-server did not start in 20 s: ' + out)); }, 20000);
    p.stdout!.on('data', (d: Buffer) => {
      out += d.toString();
      const m = out.match(/listening on (http:\/\/127\.0\.0\.1:(\d+))/);
      if (m) {
        clearTimeout(timer);
        const port = Number(m[2]);
        ok({
          url: m[1], port, wsUrl: `ws://127.0.0.1:${port}/ws`,
          stop: () => new Promise<void>((r) => { p.once('exit', () => r()); p.kill(); }),
        });
      }
    });
    p.stderr!.on('data', (d: Buffer) => { out += d.toString(); });
    p.on('exit', (code) => { clearTimeout(timer); fail(new Error(`wb-server exited with ${code}: ${out}`)); });
  });
}
