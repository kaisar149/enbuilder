// Browser harness for the REAL renderer (MeshViewport3D under software WebGL). Bundled with esbuild and driven over CDP by run.mjs.
// Not part of `vitest run`: it needs a Chromium. See HANDOFF_PART9D.md.
import { MeshViewport3D } from '../../src/viewport/MeshViewport3D';
import { cellColors } from '../../src/viewport/raster';
import { buildCellGeometry, surfaceNormal } from '../../src/viewport/geometry3d';
import { cameraFrame, projectPoint } from '../../src/viewport/orbit';
import { runBenchmark } from '../../src/viewport/bench';
import { cubeMesh, fakeElevation, planeMesh, sphereMesh } from '../synth';
import type { LayerDecl } from 'wb-client';

const decl: LayerDecl = { id: 'f', field: 'f', kind: 'scalar', colormap: 'viridis', range: [0, 1], opacity: 1 } as LayerDecl;
let vp: MeshViewport3D | null = null;
let mesh: ReturnType<typeof sphereMesh>;
let values: Float32Array;

(window as any).setup = (kind: 'sphere' | 'cube' | 'plane', size: number, exaggeration: number, lighting: 'flat' | 'sun') => {
  vp?.dispose();
  document.body.innerHTML = '<div id="h" style="width:800px;height:600px;position:fixed;left:0;top:0"></div>';
  const host = document.getElementById('h')!;
  const t0 = performance.now();
  mesh = kind === 'sphere' ? sphereMesh(size) : kind === 'cube' ? cubeMesh(size) : planeMesh(size, Math.round(size / 2));
  const tMesh = performance.now() - t0;
  vp = new MeshViewport3D(host);
  const t1 = performance.now();
  vp.setMesh(mesh);
  const tBuild = performance.now() - t1;
  values = Float32Array.from({ length: mesh.nCells }, (_, i) => ((i * 2654435761) % 1000) / 1000);
  vp.setLayers([decl], { f: { name: 'f', values, min: 0, max: 1, version: 1 } as any });
  const elev = fakeElevation(mesh);
  vp.setElevation(elev);
  vp.setView({ exaggeration, lighting, dayNight: lighting === 'sun', sunDir: [0.8, 0.3, 0.5], unitsPerMetre: 1 / 6.371e6 });
  return { nCells: mesh.nCells, tMesh, tBuild, nCorners: buildCellGeometry(mesh).nCorners };
};

/** Cell colour at sample points: screen centre of visible cells, expected vs. read back from the framebuffer by the runner. */
(window as any).samples = (count: number) => {
  const g = buildCellGeometry(mesh);
  const f = cameraFrame(vp!.getCamera() as any, g.radius);
  const want = cellColors(decl, values, { biomes: [] });
  const out: any[] = [];
  const step = Math.max(1, Math.floor(mesh.nCells / (count * 8)));
  for (let i = 0; i < mesh.nCells && out.length < count; i += step) {
    // centre of the displaced polygon (what is actually drawn), not the lifted centroid
    const d = (vp as any).displaced as Float32Array;
    const a = g.cornerStart[i], b = g.cornerStart[i + 1];
    const p: [number, number, number] = [0, 0, 0];
    for (let j = a; j < b; j++) { p[0] += d[3 * j] / (b - a); p[1] += d[3 * j + 1] / (b - a); p[2] += d[3 * j + 2] / (b - a); }
    const nrm = surfaceNormal(g.kind, p), to = [f.pos[0] - p[0], f.pos[1] - p[1], f.pos[2] - p[2]];
    if ((to[0] * nrm[0] + to[1] * nrm[1] + to[2] * nrm[2]) / Math.hypot(to[0], to[1], to[2]) < 0.35) continue; // edge-on cells are sub-pixel slivers
    const s = projectPoint(f, p, 800, 600);
    if (s.depth <= 0 || s.x < 6 || s.x > 794 || s.y < 6 || s.y > 594) continue;
    const hit = vp!.pick(s.x, s.y);
    if (hit !== i) continue; // occluded or on the far side: not visible
    out.push({ cell: i, x: Math.round(s.x), y: Math.round(s.y), rgb: [want[3 * i], want[3 * i + 1], want[3 * i + 2]] });
  }
  return out;
};

(window as any).bench = (ms: number) => runBenchmark(vp!, (vp as any).kind, ms);
(window as any).nextFrame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(true))));
(window as any).glInfo = () => {
  const c = document.createElement('canvas');
  const gl = c.getContext('webgl2') as WebGL2RenderingContext | null;
  const ext = gl?.getExtension('WEBGL_debug_renderer_info');
  return gl ? { version: gl.getParameter(gl.VERSION), renderer: ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : 'unknown' } : null;
};
(window as any).ready = true;
