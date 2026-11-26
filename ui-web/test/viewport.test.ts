// @vitest-environment jsdom
// MapViewport2D behaviour (picking, pan, zoom) in jsdom. jsdom has no canvas, so getContext is stubbed; drawing itself is covered by the pixel-parity tests.
import { beforeEach, describe, expect, it } from 'vitest';
import type { MeshData } from 'wb-client';
import { MapViewport2D } from '../src/viewport/MapViewport2D';

function gridMesh(nx: number, ny: number): MeshData {
  const n = nx * ny, centroid = new Float32Array(3 * n), area = new Float32Array(n).fill(1), offs = new Uint32Array(n + 1), adj: number[] = [];
  for (let j = 0; j < ny; j++) for (let i = 0; i < nx; i++) {
    const c = j * nx + i;
    centroid[3 * c] = i; centroid[3 * c + 1] = j;
    for (const [di, dj] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) { const a = i + di, b = j + dj; if (a >= 0 && a < nx && b >= 0 && b < ny) adj.push(b * nx + a); }
    offs[c + 1] = adj.length;
  }
  return { meshId: 'grid', kind: 'plane', nCells: n, centroid, area, adjOffsets: offs, adj: Uint32Array.from(adj) };
}

let host: HTMLDivElement;
beforeEach(() => {
  const noop = () => undefined;
  const ctx = { setTransform: noop, clearRect: noop, drawImage: noop, putImageData: noop, createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }), imageSmoothingEnabled: true, globalCompositeOperation: '' };
  (HTMLCanvasElement.prototype as any).getContext = () => ctx;
  (globalThis as any).requestAnimationFrame = (f: () => void) => { f(); return 1; };
  (globalThis as any).cancelAnimationFrame = noop;
  host = document.createElement('div');
  host.getBoundingClientRect = () => ({ width: 400, height: 200, left: 0, top: 0, right: 400, bottom: 200, x: 0, y: 0, toJSON: noop } as DOMRect);
  document.body.appendChild(host);
});

describe('MapViewport2D', () => {
  it('pick maps screen positions to cells and returns null off the map', () => {
    const vp = new MapViewport2D(host, { rasterWidth: 80 });
    vp.setMesh(gridMesh(8, 4));
    expect(vp.pick(200, 100)).toBe(1 * 8 + 4); // centre of the map -> cell (4, 1)
    expect(vp.pick(1, 100)).toBe(1 * 8 + 0);
    expect(vp.pick(200, 2)).toBeNull(); // letterbox above the map
    expect(vp.pick(-10, 100)).toBeNull();
    vp.dispose();
  });

  it('zoom keeps the map point under the cursor and the camera stays clamped to the map', () => {
    const vp = new MapViewport2D(host, { rasterWidth: 80 });
    vp.setMesh(gridMesh(8, 4));
    const before = vp.pick(100, 100);
    host.querySelector('canvas')!.dispatchEvent(new WheelEvent('wheel', { clientX: 100, clientY: 100, deltaY: -400, cancelable: true }));
    expect(vp.getCamera().zoom).toBeGreaterThan(1.5);
    expect(vp.pick(100, 100)).toBe(before);
    vp.setCamera({ x: -5, y: 9, zoom: 4 });
    const c = vp.getCamera();
    expect(c.x).toBeGreaterThanOrEqual(0); expect(c.x).toBeLessThanOrEqual(1);
    expect(c.y).toBeGreaterThanOrEqual(0); expect(c.y).toBeLessThanOrEqual(1);
    vp.setCamera({ x: 0.5, y: 0.5, zoom: 1000 });
    expect(vp.getCamera().zoom).toBeLessThanOrEqual(24);
    vp.dispose();
  });

  it('emits hover and pick events and stops after dispose', () => {
    const vp = new MapViewport2D(host, { rasterWidth: 80 });
    vp.setMesh(gridMesh(8, 4));
    const hovers: (number | null)[] = [], picks: (number | null)[] = [];
    vp.onHover((e) => hovers.push(e.cell));
    vp.onPick((e) => picks.push(e.cell));
    const cv = host.querySelector('canvas')!;
    const ev = (t: string, x: number, y: number) => cv.dispatchEvent(new MouseEvent(t, { clientX: x, clientY: y, bubbles: true }));
    ev('pointermove', 200, 100);
    ev('pointerdown', 200, 100);
    ev('pointerup', 200, 100);
    expect(hovers).toContain(12);
    expect(picks).toEqual([12]);
    vp.dispose();
    expect(host.querySelector('canvas')).toBeNull();
  });

  it('a drag pans the camera and does not count as a pick', () => {
    const vp = new MapViewport2D(host, { rasterWidth: 80 });
    vp.setMesh(gridMesh(8, 4));
    vp.setCamera({ x: 0.5, y: 0.5, zoom: 4 });
    const picks: unknown[] = [];
    vp.onPick((e) => picks.push(e));
    const cv = host.querySelector('canvas')!;
    const ev = (t: string, x: number, y: number) => cv.dispatchEvent(new MouseEvent(t, { clientX: x, clientY: y, bubbles: true }));
    ev('pointerdown', 200, 100); ev('pointermove', 260, 100); ev('pointerup', 260, 100);
    expect(vp.getCamera().x).toBeLessThan(0.5);
    expect(picks).toEqual([]);
    vp.dispose();
  });
});
