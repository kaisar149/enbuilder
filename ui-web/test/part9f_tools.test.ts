// @vitest-environment jsdom
// Part 9f, no server: tool overlays are plain overlay items (pure), and viewports emit stroke gestures only in tool mode.
import { beforeEach, describe, expect, it } from 'vitest';
import type { MeshData } from 'wb-client';
import { defaultBrush } from 'wb-client';
import { MapViewport2D } from '../src/viewport/MapViewport2D';
import { cellValues, lockedForLayer, outlineOfCells, toolOverlayItems } from '../src/tools/overlays';
import { refreshBadge } from '../src/panels/RefreshBadge';
import { groupTools } from '../src/panels/ToolsPanel';
import { historyDetail } from '../src/panels/HistoryPanel';

function gridMesh(nx: number, ny: number): MeshData {
  const n = nx * ny, centroid = new Float32Array(3 * n), area = new Float32Array(n).fill(1), offs = new Uint32Array(n + 1), adj: number[] = [];
  for (let j = 0; j < ny; j++) for (let i = 0; i < nx; i++) {
    const c = j * nx + i;
    centroid[3 * c] = i * 1000; centroid[3 * c + 1] = j * 1000;
    for (const [di, dj] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) { const a = i + di, b = j + dj; if (a >= 0 && a < nx && b >= 0 && b < ny) adj.push(b * nx + a); }
    offs[c + 1] = adj.length;
  }
  return { meshId: 'grid', kind: 'plane', nCells: n, centroid, area, adjOffsets: offs, adj: Uint32Array.from(adj) };
}
const color = (t: string) => `var(--overlay-${t})`;

describe('tool overlays (pure)', () => {
  const mesh = gridMesh(8, 6);
  it('outline of a cell set joins inside cells to outside neighbours only', () => {
    const o = outlineOfCells(mesh, [9, 10, 17, 18]); // a 2x2 block
    expect(o.nEdges).toBe(8);
    const inside = new Set([9, 10, 17, 18]);
    for (let k = 0; k < o.nEdges; k++) { expect(inside.has(o.pairs[2 * k])).toBe(true); expect(inside.has(o.pairs[2 * k + 1])).toBe(false); }
  });
  it('items come bottom to top: lock hatch, stroke, ring; each a mask item with a theme colour', () => {
    const brush = { ...defaultBrush(), radiusKm: 1.5 }; // 1.5 km = 1.5 grid steps
    const items = toolOverlayItems({ mesh, ring: { cell: 20, brush }, stroke: { cells: Uint32Array.of(20, 21), weights: Float32Array.of(1, 0.5) }, locked: Uint32Array.of(3), resolveColor: color });
    expect(items.map((i) => i.id)).toEqual(['tool-lock', 'tool-stroke', 'tool-ring']);
    expect(items.every((i) => i.kind === 'mask')).toBe(true);
    expect(items[0].pattern).toBe('hatch');
    expect(items[2]).toMatchObject({ kind: 'mask', mode: 'outline' });
    expect(items[1]).toMatchObject({ kind: 'mask', mode: 'fill' });
    expect(items[0].color).toBe('var(--overlay-lock)');
  });
  it('nothing to draw gives no items', () => {
    expect(toolOverlayItems({ mesh, ring: null, stroke: null, locked: null, resolveColor: color })).toEqual([]);
    expect(toolOverlayItems({ mesh, ring: null, stroke: { cells: new Uint32Array(0), weights: new Float32Array(0) }, locked: new Uint32Array(0), resolveColor: color })).toEqual([]);
  });
  it('cellValues puts the weights on the cells', () => {
    const v = cellValues(mesh, [2, 5], [0.25, 0.75]);
    expect(v[2]).toBeCloseTo(0.25); expect(v[5]).toBeCloseTo(0.75); expect(v[0]).toBe(0);
  });
  it('lockedForLayer: the shown output field, else every locked cell', () => {
    const locks = { meshId: 'g', generation: 1, cells: { elevation: Uint32Array.of(1, 2), temperature: Uint32Array.of(2, 3), precipitation: new Uint32Array(0), biome: new Uint32Array(0) } };
    expect(Array.from(lockedForLayer(locks, 'temperature')!)).toEqual([2, 3]);
    expect(Array.from(lockedForLayer(locks, 'wind_speed')!).sort()).toEqual([1, 2, 3]);
    expect(lockedForLayer(null, 'elevation')).toBeNull();
  });
  it('refresh badge always says which result is shown', () => {
    expect(refreshBadge({ kind: 'refresh', mode: 'preview', status: 'running' }, 'refined')!.text).toMatch(/preview/i);
    expect(refreshBadge({ kind: 'refresh', mode: 'refined', status: 'running' }, 'preview')!.running).toBe(true);
    expect(refreshBadge(null, 'preview')!.text).toBe('Preview quality');
    expect(refreshBadge(null, 'refined')!.text).toBe('Refined');
    expect(refreshBadge(null, 'full')).toBeNull();
    expect(refreshBadge({ kind: 'solve', status: 'running' }, null)!.text).toBe('Solving');
  });
  it('history detail: strokes and locks say how many cells', () => {
    expect(historyDetail({ label: 'x', preset: 'p', changes: [], kind: 'stroke', cells: 12 })).toBe('12 cells');
    expect(historyDetail({ label: 'x', preset: 'p', changes: [{ name: 'a', old: 1, new: 2 }], kind: 'realism' })).toBe('a: 1 to 2');
  });
  it('tool groups come from the schema layers', () => {
    const g = groupTools({
      tools: [{ id: 'raise', label: 'Raise', hint: '', kind: 'paint', layer: 'elevation', op: 'add' }, { id: 'eraser', label: 'Eraser', hint: '', kind: 'erase', layer: 'any', op: 'erase' }, { id: 'lock', label: 'Lock', hint: '', kind: 'lock', locked: true }],
      layers: [{ id: 'elevation', label: 'Elevation', output: 'elevation', unit: 'm', min: -1, max: 1 }],
    } as any);
    expect(g.map((x) => x.title)).toEqual(['Elevation', 'Eraser', 'Locks']);
  });
});

describe('viewport tool mode', () => {
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
  const setup = () => {
    const vp = new MapViewport2D(host, { rasterWidth: 80 });
    vp.setMesh(gridMesh(8, 4));
    const cv = host.querySelector('canvas')!;
    const ev = (t: string, x: number, y: number, init: MouseEventInit = {}) => cv.dispatchEvent(new MouseEvent(t, { clientX: x, clientY: y, bubbles: true, ...init }));
    const got: string[] = [];
    vp.onStroke((e) => got.push(`${e.phase}:${e.cell}`));
    return { vp, ev, got };
  };
  it('off by default: a drag pans and no stroke is reported', () => {
    const { vp, ev, got } = setup();
    vp.setCamera({ x: 0.5, y: 0.5, zoom: 4 });
    ev('pointerdown', 200, 100); ev('pointermove', 260, 100); ev('pointerup', 260, 100);
    expect(got).toEqual([]);
    expect(vp.getCamera()).toMatchObject({ x: expect.any(Number) });
    vp.dispose();
  });
  it('on: left drag reports down, one move per new cell, then up; the camera stays put', () => {
    const { vp, ev, got } = setup();
    vp.setToolMode(true);
    const cam = vp.getCamera();
    ev('pointerdown', 50, 100); ev('pointermove', 52, 100); ev('pointermove', 150, 100); ev('pointermove', 152, 100); ev('pointerup', 250, 100);
    expect(got[0]).toMatch(/^down:\d+$/);
    expect(got.filter((g) => g.startsWith('move')).length).toBeGreaterThanOrEqual(1);
    expect(new Set(got.filter((g) => g.startsWith('move'))).size).toBe(got.filter((g) => g.startsWith('move')).length); // never twice for one cell in a row
    expect(got.at(-1)).toMatch(/^up:/);
    expect(vp.getCamera()).toEqual(cam);
    vp.dispose();
  });
  it('shift-drag still pans in tool mode', () => {
    const { vp, ev, got } = setup();
    vp.setToolMode(true);
    vp.setCamera({ x: 0.5, y: 0.5, zoom: 4 });
    ev('pointerdown', 200, 100, { shiftKey: true }); ev('pointermove', 260, 100); ev('pointerup', 260, 100);
    expect(got).toEqual([]);
    expect(vp.getCamera()).toMatchObject({ x: expect.any(Number) });
    expect((vp.getCamera() as any).x).toBeLessThan(0.5);
    vp.dispose();
  });
});
