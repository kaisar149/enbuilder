// Drives test/gl/harness.ts in headless Chromium (software WebGL) over the DevTools protocol. Usage: node test/gl/run.mjs [outDir]
// Needs: a Chromium (CHROME env or the Playwright one), esbuild (ships with vite). Writes screenshots and gl_results.json to outDir.
import { spawn, execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(process.argv[2] ?? '/tmp/gl_out');
mkdirSync(out, { recursive: true });
const chrome = process.env.CHROME ?? '/opt/pw-browsers/chromium-1243/chrome-linux64/chrome';
execFileSync('npx', ['esbuild', join(here, 'harness.ts'), '--bundle', '--format=iife', `--outfile=${out}/harness.js`, '--log-level=warning'], { cwd: resolve(here, '../..'), stdio: 'inherit' });
writeFileSync(join(out, 'index.html'), '<!doctype html><body style="margin:0"><script src="harness.js"></script></body>');

const port = 9333;
const proc = spawn(chrome, ['--headless=new', '--no-sandbox', '--disable-gpu-sandbox', '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist',
  `--remote-debugging-port=${port}`, '--window-size=800,600', '--hide-scrollbars', `--user-data-dir=${out}/profile`, 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let targets;
for (let i = 0; i < 50; i++) { try { targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); if (targets.find((t) => t.type === 'page')) break; } catch {} await sleep(200); }
const ws = new WebSocket(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evalJs = async (expr) => { const r = await cdp('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }); if (r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 600)); return r.result.result.value; };
const shot = async (name) => { const r = await cdp('Page.captureScreenshot', { format: 'png' }); const buf = Buffer.from(r.result.data, 'base64'); writeFileSync(join(out, name), buf); return buf; };

function decodePng(buf) { // 8-bit RGB/RGBA non-interlaced
  let p = 8, w = 0, h = 0, ct = 0; const idat = [];
  while (p < buf.length) { const len = buf.readUInt32BE(p), type = buf.toString('ascii', p + 4, p + 8); const data = buf.subarray(p + 8, p + 8 + len);
    if (type === 'IHDR') { w = data.readUInt32BE(0); h = data.readUInt32BE(4); ct = data[9]; } else if (type === 'IDAT') idat.push(data); p += 12 + len; }
  const ch = ct === 6 ? 4 : 3; const raw = zlib.inflateSync(Buffer.concat(idat)); const stride = w * ch; const px = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) { const f = raw[y * (stride + 1)]; for (let x = 0; x < stride; x++) { const v = raw[y * (stride + 1) + 1 + x]; const a = x >= ch ? px[y * stride + x - ch] : 0, b = y ? px[(y - 1) * stride + x] : 0, c = x >= ch && y ? px[(y - 1) * stride + x - ch] : 0;
    let r = v; if (f === 1) r = v + a; else if (f === 2) r = v + b; else if (f === 3) r = v + ((a + b) >> 1); else if (f === 4) { const pp = a + b - c, pa = Math.abs(pp - a), pb = Math.abs(pp - b), pc = Math.abs(pp - c); r = v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c); } px[y * stride + x] = r & 255; } }
  return { w, h, ch, px };
}

const results = { gl: null, parity: {}, bench: {}, build: {} };
try {
  await cdp('Page.enable');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 800, height: 600, deviceScaleFactor: 1, mobile: false });
  await cdp('Page.navigate', { url: `file://${out}/index.html` });
  for (let i = 0; i < 50 && !(await evalJs('window.ready === true').catch(() => false)); i++) await sleep(200);
  results.gl = await evalJs('window.glInfo()');
  // 1. colour parity: flat lighting, relief on and off, every shape
  for (const [kind, size] of [['sphere', 4], ['cube', 40], ['plane', 80]]) {
    for (const ex of [0, 40]) {
      const info = await evalJs(`window.setup('${kind}', ${size}, ${ex}, 'flat')`);
      await evalJs('window.nextFrame()');
      const samples = await evalJs('window.samples(60)');
      const png = decodePng(await shot(`flat_${kind}_ex${ex}.png`));
      let ok = 0, worst = 0; const bad = [];
      for (const s of samples) { const o = (s.y * png.w + s.x) * png.ch; const d = Math.max(...[0, 1, 2].map((k) => Math.abs(png.px[o + k] - s.rgb[k]))); worst = Math.max(worst, d); if (d <= 2) ok++; else bad.push({ x: s.x, y: s.y, r: Math.round(Math.hypot(s.x - 400, s.y - 300)), d }); }
      results.parity[`${kind}_ex${ex}`] = { cells: info.nCells, samples: samples.length, matching: ok, worstChannelDiff: worst, mismatches: bad.slice(0, 12) };
    }
  }
  // 2. sun lighting screenshot (night side must be darker than the day side)
  await evalJs(`window.setup('sphere', 5, 60, 'sun')`); await evalJs('window.nextFrame()'); await shot('sun_sphere.png');
  // 3. frame rate + build time on larger meshes (software GL: a LOWER bound, not a GPU number)
  for (const [kind, size] of [['sphere', 7], ['sphere', 8], ['cube', 290], ['plane', 1000]]) {
    const info = await evalJs(`window.setup('${kind}', ${size}, 20, 'flat')`);
    await evalJs('window.nextFrame()');
    const b = await evalJs('window.bench(2500)');
    results.bench[`${kind}_${info.nCells}`] = b;
    results.build[`${kind}_${info.nCells}`] = { meshMs: Math.round(info.tMesh), buildMs: Math.round(info.tBuild), corners: info.nCorners };
    if (info.nCells > 400000) await shot(`big_${kind}.png`);
  }
} finally { writeFileSync(join(out, 'gl_results.json'), JSON.stringify(results, null, 1)); ws.close(); proc.kill('SIGKILL'); }
console.log(JSON.stringify(results, null, 1));
