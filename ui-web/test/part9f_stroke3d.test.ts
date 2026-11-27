// @vitest-environment jsdom
// Part 9f: the 3D viewports report strokes too (stub renderer, sphere): left drag in tool mode paints, shift-drag still orbits.
import { beforeEach, describe, expect, it } from 'vitest';
import { StubViewport } from '../src/viewport/StubViewport';
import { buildCellGeometry } from '../src/viewport/geometry3d';
import { cameraFrame, projectPoint } from '../src/viewport/orbit';
import { sphereMesh } from './synth';

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

describe('3D stroke gestures', () => {
  const setup = () => {
    const mesh = sphereMesh(4);
    const v = new StubViewport(host);
    v.setMesh(mesh);
    v.resize();
    const g = buildCellGeometry(mesh);
    const f = cameraFrame(v.getCamera() as any, g.radius);
    // two cells that face the camera, far enough apart to be different picks
    const screenOf = (i: number) => projectPoint(f, [mesh.centroid[3 * i], mesh.centroid[3 * i + 1], mesh.centroid[3 * i + 2]], 800, 600);
    const visible: { i: number; x: number; y: number }[] = [];
    for (let i = 0; i < mesh.nCells && visible.length < 400; i++) {
      const s = screenOf(i);
      if (s.depth > 0 && s.x > 300 && s.x < 500 && s.y > 200 && s.y < 400 && v.pick(s.x, s.y) === i) visible.push({ i, x: s.x, y: s.y });
    }
    const cv = host.querySelector('canvas')!;
    const ev = (t: string, x: number, y: number, init: MouseEventInit = {}) => cv.dispatchEvent(new MouseEvent(t, { clientX: x, clientY: y, bubbles: true, ...init }));
    const got: string[] = [];
    v.onStroke((e) => got.push(`${e.phase}:${e.cell}`));
    return { v, visible, ev, got };
  };

  it('off by default: no stroke events', () => {
    const { v, visible, ev, got } = setup();
    expect(visible.length).toBeGreaterThan(3);
    ev('pointerdown', visible[0].x, visible[0].y); ev('pointermove', visible[1].x, visible[1].y); ev('pointerup', visible[1].x, visible[1].y);
    expect(got).toEqual([]);
    v.dispose();
  });

  it('tool mode: down on a cell, a move for each new cell, up; the camera does not move', () => {
    const { v, visible, ev, got } = setup();
    v.setToolMode(true);
    const cam = JSON.stringify(v.getCamera());
    const a = visible[0], b = visible.find((p) => p.i !== a.i && Math.hypot(p.x - a.x, p.y - a.y) > 15)!;
    ev('pointerdown', a.x, a.y); ev('pointermove', b.x, b.y); ev('pointermove', b.x + 0.2, b.y); ev('pointerup', b.x, b.y);
    expect(got[0]).toBe(`down:${a.i}`);
    expect(got).toContain(`move:${b.i}`);
    expect(got.filter((g) => g === `move:${b.i}`).length).toBe(1);
    expect(got.at(-1)).toBe(`up:${b.i}`);
    expect(JSON.stringify(v.getCamera())).toBe(cam);
    v.dispose();
  });

  it('a press beside the globe starts no stroke', () => {
    const { v, ev, got } = setup();
    v.setToolMode(true);
    ev('pointerdown', 2, 2); ev('pointermove', 300, 300); ev('pointerup', 300, 300);
    expect(got).toEqual([]);
    v.dispose();
  });

  it('shift-drag still orbits in tool mode', () => {
    const { v, visible, ev, got } = setup();
    v.setToolMode(true);
    const cam = JSON.stringify(v.getCamera());
    ev('pointerdown', visible[0].x, visible[0].y, { shiftKey: true }); ev('pointermove', visible[0].x + 60, visible[0].y + 20); ev('pointerup', visible[0].x + 60, visible[0].y + 20);
    expect(got).toEqual([]);
    expect(JSON.stringify(v.getCamera())).not.toBe(cam);
    v.dispose();
  });
});
