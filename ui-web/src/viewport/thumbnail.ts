// Small map thumbnails (variations, previews). Pure: mesh + field + layer declaration -> RGBA. No DOM, so it runs in Node tests.
import type { FieldData, LayerDecl, MeshData } from 'wb-client';
import { buildPixelCells, makeProjection, type Projection } from './projections';
import { paintPixels, type PaintContext } from './raster';

const cache = new WeakMap<MeshData, Map<number, { proj: Projection; cells: Int32Array }>>();

/** Pixel to cell table for a mesh at a width; built once per mesh and width. */
export function thumbnailGrid(mesh: MeshData, width: number): { proj: Projection; cells: Int32Array } {
  let per = cache.get(mesh);
  if (!per) cache.set(mesh, (per = new Map()));
  let g = per.get(width);
  if (!g) {
    const proj = makeProjection(mesh, width);
    g = { proj, cells: buildPixelCells(mesh, proj) };
    per.set(width, g);
  }
  return g;
}

export function renderThumbnail(mesh: MeshData, field: FieldData, decl: LayerDecl, ctx: PaintContext, width = 120): { width: number; height: number; rgba: Uint8ClampedArray } {
  const { proj, cells } = thumbnailGrid(mesh, width);
  const rgba = new Uint8ClampedArray(cells.length * 4);
  paintPixels(cells, field, decl, ctx, rgba);
  return { width: proj.width, height: proj.height, rgba };
}
