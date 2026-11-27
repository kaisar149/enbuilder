// Part 9e: turns the stack + the fetched data into the list of overlay items a viewport draws. Pure function of the two stores' values.
// The items carry data (arrays) and a style (colour, opacity, blend); a viewport decides how a kind looks in its own space.
import { fieldKey, outlineKey, type FieldData, type OverlayDef, type OutlineData, type RiversData, type Schema, type WbState } from 'wb-client';
import type { OverlayItem } from '../viewport/types';
import { drawnOverlays, type LayerStack } from './stack';

/** Fields a vector overlay needs: layer 0, 1, 2 of the vector field (x, y, z components in mesh space). */
export const VECTOR_LAYERS = [0, 1, 2] as const;

/**
 * What the core must have solved for an overlay to be drawable (used to grey out entries before a solve). Everything comes from the declaration:
 * a vector or mask reads `field`; a lines overlay is available once its `weight_field` is (the geometry query itself answers `not_solved` otherwise).
 */
export function overlayNeeds(def: OverlayDef): string[] {
  if (def.kind === 'lines') return def.weight_field ? [def.weight_field] : [];
  return def.field ? [def.field] : [];
}

export const overlayAvailable = (def: OverlayDef, available: (field: string) => boolean): boolean => overlayNeeds(def).every(available);

export interface DataNeeds {
  fields: { name: string; layer: number }[];
  rivers: boolean;
  outlines: { field: string; threshold: number }[];
}

/** The data the drawn overlays need (what the controller must have fetched). */
export function dataNeeds(stack: LayerStack, schema: Schema | null, available: (field: string) => boolean): DataNeeds {
  const needs: DataNeeds = { fields: [], rivers: false, outlines: [] };
  for (const { def } of drawnOverlays(stack, schema)) {
    if (!overlayAvailable(def, available)) continue;
    if (def.kind === 'vector') for (const l of VECTOR_LAYERS) needs.fields.push({ name: def.field!, layer: l });
    else if (def.kind === 'lines') needs.rivers = true;
    else if (def.mode === 'fill') needs.fields.push({ name: def.field!, layer: 0 });
    else needs.outlines.push({ field: def.field!, threshold: def.threshold ?? 0.5 });
  }
  return needs;
}

/** `resolveColor(token)` reads the theme token `--overlay-<token>` (the caller owns the DOM; this stays pure). */
export function buildOverlayItems(app: Pick<WbState, 'fields' | 'rivers' | 'outlines'>, schema: Schema | null, stack: LayerStack, resolveColor: (token: string) => string): OverlayItem[] {
  const items: OverlayItem[] = [];
  for (const { entry, def } of drawnOverlays(stack, schema)) {
    const style = { id: def.id, label: def.label, color: resolveColor(def.token), opacity: entry.opacity, blend: entry.blend };
    if (def.kind === 'vector') {
      const comps = VECTOR_LAYERS.map((l) => app.fields[fieldKey(def.field!, l)]) as (FieldData | undefined)[];
      if (comps.every(Boolean)) items.push({ ...style, kind: 'vector', components: [comps[0]!.values, comps[1]!.values, comps[2]!.values], animated: !!def.animated });
    } else if (def.kind === 'lines') {
      const rivers: RiversData | null = app.rivers;
      if (rivers) items.push({ ...style, kind: 'lines', rivers });
    } else if (def.mode === 'fill') {
      const f = app.fields[fieldKey(def.field!, 0)];
      if (f) items.push({ ...style, kind: 'mask', mode: 'fill', values: f.values, threshold: def.threshold ?? 0 });
    } else {
      const o: OutlineData | undefined = app.outlines[outlineKey(def.field!, def.threshold ?? 0.5)];
      if (o) items.push({ ...style, kind: 'mask', mode: 'outline', outline: o });
    }
  }
  return items;
}
