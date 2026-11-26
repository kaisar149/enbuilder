// Pure pixel painter: cell values + a layer declaration -> RGBA. No DOM, so it runs in Node tests and can be compared with the headless PNG.
// It knows nothing about field names: everything it needs is in the LayerDecl (colormap, range, kind, palette) and the biome table.
import type { BiomeDef, FieldData, LayerDecl } from 'wb-client';
import { categoryColor, colormap, type RGB } from './colormaps';

/** Colour of a non-finite value inside the map; same as the Rust renderer's background. */
export const MISSING: RGB = [24, 24, 28];

export interface PaintContext {
  biomes: BiomeDef[];
}

export function cellColor(decl: LayerDecl, v: number, ctx: PaintContext): RGB {
  if (decl.kind === 'category') {
    const id = Math.round(v);
    if (decl.palette === 'biomes') {
      const b = ctx.biomes.find((x) => x.id === id);
      if (b) return b.color;
    }
    return categoryColor(id);
  }
  if (!Number.isFinite(v)) return MISSING;
  const [lo, hi] = decl.range;
  return colormap(decl.colormap, (v - lo) / (hi - lo));
}

/** One colour per cell (fast path for repainting: palette lookup happens once per cell, not once per pixel). */
export function cellColors(decl: LayerDecl, values: Float32Array, ctx: PaintContext): Uint8ClampedArray {
  const out = new Uint8ClampedArray(values.length * 3);
  const cache = decl.kind === 'category' ? new Map<number, RGB>() : null;
  for (let i = 0; i < values.length; i++) {
    let c: RGB;
    if (cache) {
      const id = Math.round(values[i]);
      c = cache.get(id) ?? (cache.set(id, cellColor(decl, values[i], ctx)), cache.get(id)!);
    } else {
      c = cellColor(decl, values[i], ctx);
    }
    out[3 * i] = c[0]; out[3 * i + 1] = c[1]; out[3 * i + 2] = c[2];
  }
  return out;
}

/** Paints `pixelCells` into `rgba` (length 4 * pixels). Pixels outside the map are left fully transparent so the themed background shows through. */
export function paintPixels(pixelCells: Int32Array, field: FieldData, decl: LayerDecl, ctx: PaintContext, rgba: Uint8ClampedArray, opacity = decl.opacity): void {
  const colors = cellColors(decl, field.values, ctx);
  const alpha = Math.round(255 * opacity);
  for (let i = 0; i < pixelCells.length; i++) {
    const c = pixelCells[i];
    const o = 4 * i;
    if (c < 0) { rgba[o + 3] = 0; continue; }
    rgba[o] = colors[3 * c]; rgba[o + 1] = colors[3 * c + 1]; rgba[o + 2] = colors[3 * c + 2]; rgba[o + 3] = alpha;
  }
}
