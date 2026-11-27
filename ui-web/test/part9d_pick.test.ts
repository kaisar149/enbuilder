import { describe, expect, it } from 'vitest';
import { buildCellGeometry, cellRelief, cellTop, shapeKind, surfaceNormal } from '../src/viewport/geometry3d';
import { cameraFrame, clampCamera, focusOn, homeCamera, panBy, projectPoint, rayFromPixel, orbitBy, zoomBy, limitsFor } from '../src/viewport/orbit';
import { pickRay } from '../src/viewport/pick3d';
import { sunDirection } from '../src/viewport/sun';
import type { OrbitCamera } from '../src/viewport/types';
import { cubeMesh, fakeElevation, planeMesh, sphereMesh } from './synth';

const W = 800, H = 600;
const meshes = { sphere: sphereMesh(5), cube: cubeMesh(24), plane: planeMesh(48, 24) };

function visibleSamples(mesh: ReturnType<typeof sphereMesh>, cam: OrbitCamera, R: number, count: number) {
  const f = cameraFrame(cam, R);
  const out: { cell: number; x: number; y: number }[] = [];
  const kind = shapeKind(mesh);
  const step = Math.max(1, Math.floor(mesh.nCells / (count * 12)));
  for (let i = 0; i < mesh.nCells && out.length < count; i += step) {
    const p: [number, number, number] = [mesh.centroid[3 * i], mesh.centroid[3 * i + 1], mesh.centroid[3 * i + 2]];
    const to = [f.pos[0] - p[0], f.pos[1] - p[1], f.pos[2] - p[2]];
    const l = Math.hypot(to[0], to[1], to[2]);
    const up = surfaceNormal(kind, p);
    if ((to[0] * up[0] + to[1] * up[1] + to[2] * up[2]) / l < 0.45) continue; // facing the camera
    const s = projectPoint(f, p, W, H);
    if (s.depth > 0 && s.x > 5 && s.x < W - 5 && s.y > 5 && s.y < H - 5) out.push({ cell: i, x: s.x, y: s.y });
  }
  return out;
}

describe('pick by ray cast', () => {
  for (const kind of ['sphere', 'cube', 'plane'] as const) {
    it(`${kind}: the cell under a projected cell centre is that cell (no relief)`, () => {
      const mesh = meshes[kind];
      const g = buildCellGeometry(mesh);
      for (const cam of [homeCamera(kind), clampCamera({ ...homeCamera(kind), yaw: 2.2, pitch: 0.8, distance: limitsFor(kind).minDist * 1.8 }, kind)]) {
        const f = cameraFrame(cam, g.radius);
        const samples = visibleSamples(mesh, cam, g.radius, 120);
        expect(samples.length).toBeGreaterThan(20);
        let ok = 0;
        for (const s of samples) {
          const r = rayFromPixel(f, s.x, s.y, W, H);
          const hit = pickRay(mesh, g, null, 0, r.origin, r.dir);
          if (hit?.cell === s.cell) ok++;
        }
        expect(ok).toBe(samples.length);
      }
    });

    it(`${kind}: with relief the pick finds the raised cell, and misses return null`, () => {
      const mesh = meshes[kind];
      const g = buildCellGeometry(mesh);
      const relief = cellRelief(fakeElevation(mesh), 1, 1 / 10)!; // up to a few hundred units of relief on a radius of 1..? scaled below
      const scaled = new Float32Array(relief.length);
      let mx = 0;
      for (let i = 0; i < relief.length; i++) mx = Math.max(mx, relief[i]);
      for (let i = 0; i < relief.length; i++) scaled[i] = (relief[i] / (mx || 1)) * 0.06 * g.radius;
      const cam = homeCamera(kind);
      const f = cameraFrame(cam, g.radius);
      const samples = visibleSamples(mesh, cam, g.radius, 150);
      let ok = 0;
      for (const s of samples) {
        const top = cellTop(g, mesh, s.cell, scaled);
        const p = projectPoint(f, top, W, H);
        const r = rayFromPixel(f, p.x, p.y, W, H);
        const hit = pickRay(mesh, g, scaled, 0.06 * g.radius, r.origin, r.dir);
        if (hit?.cell === s.cell) ok++;
      }
      // a raised cell can be hidden behind a nearer ridge at a low viewing angle; then the pick correctly returns the occluder
      expect(ok / samples.length).toBeGreaterThan(kind === 'plane' ? 0.9 : 0.97);
      // pointing far away from the world
      const away = pickRay(mesh, g, scaled, 0.06 * g.radius, [0, 0, 50 * g.radius], [0, 0, 1]);
      expect(away).toBeNull();
    });
  }

  it('camera maths: projection inverts the ray, limits hold, focus centres the cell', () => {
    const g = buildCellGeometry(meshes.sphere);
    const cam = homeCamera('sphere');
    const f = cameraFrame(cam, g.radius);
    const r = rayFromPixel(f, 123, 456, W, H);
    const p: [number, number, number] = [r.origin[0] + r.dir[0] * 2, r.origin[1] + r.dir[1] * 2, r.origin[2] + r.dir[2] * 2];
    const s = projectPoint(f, p, W, H);
    expect(s.x).toBeCloseTo(123, 3); expect(s.y).toBeCloseTo(456, 3);
    let c = cam;
    for (let i = 0; i < 200; i++) c = zoomBy(c, 'sphere', -400);
    expect(c.distance).toBeCloseTo(limitsFor('sphere').minDist, 6);
    for (let i = 0; i < 200; i++) c = orbitBy(c, 'sphere', 0, 500);
    expect(c.pitch).toBeLessThanOrEqual(limitsFor('sphere').maxPitch);
    const pl = panBy(homeCamera('plane'), 'plane', 1, 100000, 0, H);
    expect(Math.abs(pl.target[0])).toBeLessThanOrEqual(1);
    const target: [number, number, number] = [0, 0.6 * g.radius, 0.8 * g.radius];
    const fc = focusOn(cam, 'sphere', target, g.radius);
    const ff = cameraFrame(fc, g.radius);
    const sc = projectPoint(ff, target, W, H);
    expect(sc.x).toBeCloseTo(W / 2 - 0.5, 1); expect(sc.y).toBeCloseTo(H / 2 - 0.5, 1);
  });

  it('sun direction follows the core geometry', () => {
    const e = { axial_tilt_deg: 23.4, year_length_days: 365.25, day_length_h: 24 };
    expect(sunDirection(e, false, 0, 0.5)).toEqual([1, 0, 0].map((v, i) => sunDirection(e, false, 0, 0.5)[i]));
    const june = sunDirection(e, false, 0.25, 0.5);
    expect(Math.asin(june[2]) * 180 / Math.PI).toBeCloseTo(23.4, 1); // declination at the solstice
    const locked = { axial_tilt_deg: 0, year_length_days: 0, day_length_h: 0 };
    expect(sunDirection(locked, false, 0.7, 0.2)).toEqual([1, 0, 0]);
    const noon = sunDirection(e, false, 0, 0.5), dusk = sunDirection(e, false, 0, 0.75);
    expect(noon[0] * dusk[0] + noon[1] * dusk[1]).toBeCloseTo(0, 5);
  });
});
