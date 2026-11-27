// @vitest-environment jsdom
// 3D viewport behaviour without a GPU: the stub renderer (swap proof), picking through the viewport, selection, camera, and 2D/3D colour parity.
import { beforeEach, describe, expect, it } from 'vitest';
import type { FieldData, LayerDecl } from 'wb-client';
import { StubViewport } from '../src/viewport/StubViewport';
import { createViewport3D } from '../src/viewport/factory';
import { cellColors, paintPixels } from '../src/viewport/raster';
import { buildPixelCells, makeProjection } from '../src/viewport/projections';
import { cameraFrame, projectPoint } from '../src/viewport/orbit';
import { buildCellGeometry, cornerColors, surfaceNormal } from '../src/viewport/geometry3d';
import { isOrbit } from '../src/viewport/types';
import { cubeMesh, fakeElevation, planeMesh, sphereMesh } from './synth';

const decl = (id = 'temp'): LayerDecl => ({ id, field: id, kind: 'scalar', colormap: 'viridis', range: [0, 1], opacity: 1 } as LayerDecl);
const field = (n: number): FieldData => ({ name: 'temp', values: Float32Array.from({ length: n }, (_, i) => (i % 97) / 97), min: 0, max: 1, version: 1 } as unknown as FieldData);

let host: HTMLDivElement;
beforeEach(() => {
  (HTMLCanvasElement.prototype as any).getContext = () => null;
  (globalThis as any).requestAnimationFrame = (f: () => void) => setTimeout(f, 1) as unknown as number;
  (globalThis as any).cancelAnimationFrame = (h: number) => clearTimeout(h);
  host = document.createElement('div');
  Object.defineProperty(host, 'clientWidth', { value: 800, configurable: true });
  Object.defineProperty(host, 'clientHeight', { value: 600, configurable: true });
  document.body.appendChild(host);
});

describe('stub viewport (renderer swap)', () => {
  it('factory falls back to the stub without WebGL and the stub keeps every behaviour', async () => {
    const { viewport, renderer } = await createViewport3D(host);
    expect(renderer).toBe('stub'); // jsdom has no WebGL
    expect(viewport).toBeInstanceOf(StubViewport);
    viewport.dispose();
  });

  for (const [name, mesh] of [['sphere', sphereMesh(4)], ['cube', cubeMesh(16)], ['plane', planeMesh(32, 16)]] as const) {
    it(`${name}: pick through the viewport returns the cell under the pixel, draws nothing`, () => {
      const v = new StubViewport(host);
      v.setMesh(mesh);
      v.setLayers([decl()], { temp: field(mesh.nCells) });
      v.resize();
      const g = buildCellGeometry(mesh);
      const f = cameraFrame(v.getCamera() as any, g.radius);
      let ok = 0, tried = 0;
      for (let i = 0; i < mesh.nCells && tried < 40; i += Math.max(1, Math.floor(mesh.nCells / 400))) {
        const p: [number, number, number] = [mesh.centroid[3 * i], mesh.centroid[3 * i + 1], mesh.centroid[3 * i + 2]];
        const s = projectPoint(f, p, 800, 600);
        if (s.depth <= 0 || s.x < 5 || s.x > 795 || s.y < 5 || s.y > 595) continue;
        const up = surfaceNormal(g.kind, p), to = [f.pos[0] - p[0], f.pos[1] - p[1], f.pos[2] - p[2]];
        if ((to[0] * up[0] + to[1] * up[1] + to[2] * up[2]) / Math.hypot(to[0], to[1], to[2]) < 0.45) continue; // facing the camera
        const hit = v.pick(s.x, s.y);
        if (hit === null) continue;
        tried++;
        // either this cell, or (back side / grazing) a visible cell; the visible ones must round-trip
        if (hit === i) ok++;
      }
      expect(tried).toBeGreaterThan(5);
      expect(ok / tried).toBeGreaterThan(0.9);
      expect(v.pick(-5, 10)).toBeNull();
      expect(v.log.geometry).toBe(1);
      v.dispose();
    });
  }

  it('camera: orbit-only, reset, focus, selection and listeners', async () => {
    const v = new StubViewport(host);
    const mesh = sphereMesh(3);
    v.setMesh(mesh);
    const seen: unknown[] = [];
    v.onCameraChange((c) => seen.push(c));
    v.setCamera({ x: 0.2, y: 0.2, zoom: 3 }); // a map camera is ignored
    expect(isOrbit(v.getCamera())).toBe(true);
    expect(seen.length).toBe(0);
    v.focusCell(5);
    await new Promise((r) => setTimeout(r, 600));
    expect(seen.length).toBeGreaterThan(0);
    v.setSelection(3); v.setSelection(null);
    v.resetView();
    v.setElevation(fakeElevation(mesh));
    v.setView({ exaggeration: 30, lighting: 'sun', dayNight: true, sunDir: [1, 0, 0], unitsPerMetre: 1e-4 });
    expect(v.log.positions).toBeGreaterThanOrEqual(3);
    await new Promise((r) => setTimeout(r, 30));
    expect(v.frameStats()?.frames).toBeGreaterThan(0);
    v.dispose();
    v.dispose(); // idempotent
  });
});

describe('2D / 3D colour parity', () => {
  it('every cell has the same RGB in the 3D corners as the cell colour the 2D painter uses', () => {
    const mesh = sphereMesh(4);
    const d = decl(), f = field(mesh.nCells);
    const v = new StubViewport(host);
    v.setMesh(mesh); v.setLayers([d], { temp: f });
    const want = cellColors(d, f.values, { biomes: [] });
    const got = v.cellColours()!;
    expect(Array.from(got)).toEqual(Array.from(want));
    const g = buildCellGeometry(mesh);
    const corners = cornerColors(g, got);
    for (let i = 0; i < mesh.nCells; i += 7) for (let j = g.cornerStart[i]; j < g.cornerStart[i + 1]; j++) {
      expect([corners[3 * j], corners[3 * j + 1], corners[3 * j + 2]]).toEqual([want[3 * i], want[3 * i + 1], want[3 * i + 2]]);
    }
  });

  it('plane: a 2D raster pixel and the 3D cell colour agree for the cell under it', () => {
    const mesh = planeMesh(24, 12);
    const d = decl(), f = field(mesh.nCells);
    const proj = makeProjection(mesh, 240);
    const cells = buildPixelCells(mesh, proj);
    const rgba = new Uint8ClampedArray(cells.length * 4);
    paintPixels(cells, f, d, { biomes: [] }, rgba);
    const v = new StubViewport(host); v.setMesh(mesh); v.setLayers([d], { temp: f });
    const rgb = v.cellColours()!;
    let checked = 0;
    for (let p = 0; p < cells.length; p += 17) {
      const c = cells[p]; if (c < 0) continue;
      expect([rgba[4 * p], rgba[4 * p + 1], rgba[4 * p + 2]]).toEqual([rgb[3 * c], rgb[3 * c + 1], rgb[3 * c + 2]]);
      checked++;
    }
    expect(checked).toBeGreaterThan(50);
  });
});
