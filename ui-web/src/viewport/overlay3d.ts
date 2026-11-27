// Part 9e: overlay geometry for the 3D viewports (globe, cube, flat) as plain typed arrays. Pure TypeScript: no Three.js, no DOM. A renderer only
// uploads these arrays (MeshViewport3D) or ignores them (StubViewport logs them), so the geometry is testable under Node.
import type { MeshData } from 'wb-client';
import { arrowLength, cellSpacing, glyphsFor, maskTriangles, outlineSegments, ringsForCount, riverPolylines, thinCells, vectorStats, type Components, type V3 } from '../overlays/geometry';
import { liftDir, shapeKind, type CellGeometry } from './geometry3d';
import type { OverlayItem, OverlayStyle } from './types';

export type Overlay3D =
  | { id: string; style: Omit<OverlayStyle, 'id' | 'label'>; kind: 'segments'; /** 6 floats per segment. */ positions: Float32Array; /** 0..1 per segment endpoint pair (brightness / emphasis). */ weight: Float32Array }
  | { id: string; style: Omit<OverlayStyle, 'id' | 'label'>; kind: 'fill'; indices: Uint32Array };

/** How many arrows to aim for at a camera distance (in units of the world radius). Closer = more. */
export function arrowTarget(distance: number): number {
  return Math.round(Math.max(400, Math.min(9000, 2200 * (3 / Math.max(distance, 0.3)) ** 2)));
}

/** Lifts a mesh-space point by `d` mesh units along the surface up direction. */
function lift(kind: ReturnType<typeof shapeKind>, p: V3, d: number): V3 {
  if (d === 0) return p;
  const u = liftDir(kind, p);
  return [p[0] + u[0] * d, p[1] + u[1] * d, p[2] + u[2] * d];
}

export interface Build3D {
  mesh: MeshData;
  geom: CellGeometry;
  /** Relief height per cell in mesh units (null = flat). */
  relief: Float32Array | null;
  /** Arrow target count (from the camera distance). */
  arrows: number;
}

/** Floating height above the relief so lines are not buried in it. */
const clearance = (g: CellGeometry) => g.radius * 0.004;

/** Arrow glyphs: shaft + two barbs as 3 segments each, on the relief top of every kept cell. */
export function arrowSegments(b: Build3D, comps: Components): { positions: Float32Array; weight: Float32Array } {
  const { mesh, geom } = b;
  const kind = shapeKind(mesh);
  const rings = ringsForCount(mesh.nCells, b.arrows);
  const cells = thinCells(mesh, rings);
  const glyphs = glyphsFor(mesh, comps, cells);
  const ref = vectorStats(comps).p95;
  const sp = cellSpacing(mesh) * (rings + 1);
  const pos = new Float32Array(glyphs.length * 18);
  const w = new Float32Array(glyphs.length * 3);
  let o = 0, k = 0;
  for (const g of glyphs) {
    const c: V3 = [mesh.centroid[3 * g.cell], mesh.centroid[3 * g.cell + 1], mesh.centroid[3 * g.cell + 2]];
    const base = lift(kind, c, (b.relief ? b.relief[g.cell] : 0) + clearance(geom));
    // length in mesh units: between 35% and 90% of the spacing between kept cells (arrowLength works in "pixels", reused with 0..1)
    const L = sp * arrowLength(g.mag, ref, 0.35, 0.9);
    const n = liftDir(kind, c);
    const side: V3 = [n[1] * g.dir[2] - n[2] * g.dir[1], n[2] * g.dir[0] - n[0] * g.dir[2], n[0] * g.dir[1] - n[1] * g.dir[0]];
    const tail: V3 = [base[0] - g.dir[0] * L / 2, base[1] - g.dir[1] * L / 2, base[2] - g.dir[2] * L / 2];
    const tip: V3 = [base[0] + g.dir[0] * L / 2, base[1] + g.dir[1] * L / 2, base[2] + g.dir[2] * L / 2];
    const h = L * 0.3, cb = Math.cos(2.62), sb = Math.sin(2.62);
    const b1: V3 = [tip[0] + h * (g.dir[0] * cb + side[0] * sb), tip[1] + h * (g.dir[1] * cb + side[1] * sb), tip[2] + h * (g.dir[2] * cb + side[2] * sb)];
    const b2: V3 = [tip[0] + h * (g.dir[0] * cb - side[0] * sb), tip[1] + h * (g.dir[1] * cb - side[1] * sb), tip[2] + h * (g.dir[2] * cb - side[2] * sb)];
    pos.set([...tail, ...tip, ...tip, ...b1, ...tip, ...b2], o);
    o += 18;
    w.fill(Math.min(1, g.mag / (ref || 1)), k, k + 3);
    k += 3;
  }
  return { positions: pos.subarray(0, o), weight: w.subarray(0, k) };
}

function outlineLines(b: Build3D, item: Extract<OverlayItem, { kind: 'mask'; mode: 'outline' }>): { positions: Float32Array; weight: Float32Array } {
  const { mesh, geom } = b;
  const kind = shapeKind(mesh);
  const seg = outlineSegments(mesh, item.outline);
  const pos = new Float32Array(seg.length);
  const w = new Float32Array(item.outline.nEdges).fill(1);
  for (let e = 0; e < item.outline.nEdges; e++) {
    const a = item.outline.pairs[2 * e], c = item.outline.pairs[2 * e + 1];
    const d = (b.relief ? Math.max(b.relief[a], b.relief[c]) : 0) + clearance(geom);
    const p0 = lift(kind, [seg[6 * e], seg[6 * e + 1], seg[6 * e + 2]], d);
    const p1 = lift(kind, [seg[6 * e + 3], seg[6 * e + 4], seg[6 * e + 5]], d);
    pos.set([...p0, ...p1], 6 * e);
  }
  return { positions: pos, weight: w };
}

function riverLines(b: Build3D, item: Extract<OverlayItem, { kind: 'lines' }>): { positions: Float32Array; weight: Float32Array } {
  const { mesh, geom } = b;
  const kind = shapeKind(mesh);
  const poly = riverPolylines(mesh, item.rivers);
  const nSeg = Math.max(0, poly.points.length / 3 - (poly.offsets.length - 1));
  const pos = new Float32Array(nSeg * 6);
  const w = new Float32Array(nSeg * 2);
  let s = 0;
  for (let k = 0; k + 1 < poly.offsets.length; k++) {
    for (let i = poly.offsets[k]; i + 1 < poly.offsets[k + 1]; i++) {
      const c0 = item.rivers.cells[i], c1 = item.rivers.cells[i + 1];
      const p0 = lift(kind, [poly.points[3 * i], poly.points[3 * i + 1], poly.points[3 * i + 2]], (b.relief ? b.relief[c0] : 0) + clearance(geom));
      const p1 = lift(kind, [poly.points[3 * i + 3], poly.points[3 * i + 4], poly.points[3 * i + 5]], (b.relief ? b.relief[c1] : 0) + clearance(geom));
      pos.set([...p0, ...p1], 6 * s);
      w[2 * s] = poly.width[i]; w[2 * s + 1] = poly.width[i + 1];
      s++;
    }
  }
  return { positions: pos, weight: w };
}

/** Everything a 3D renderer needs for one overlay item. */
export function buildOverlay3D(item: OverlayItem, b: Build3D): Overlay3D {
  const style = { color: item.color, opacity: item.opacity, blend: item.blend };
  if (item.kind === 'vector') return { id: item.id, style, kind: 'segments', ...arrowSegments(b, item.components as Components) };
  if (item.kind === 'lines') return { id: item.id, style, kind: 'segments', ...riverLines(b, item) };
  if (item.mode === 'outline') return { id: item.id, style, kind: 'segments', ...outlineLines(b, item) };
  return { id: item.id, style, kind: 'fill', indices: maskTriangles(b.geom.triCell, b.geom.indices, item.values, item.threshold) };
}
