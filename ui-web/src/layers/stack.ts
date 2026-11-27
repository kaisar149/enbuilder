// Part 9e: the layer stack model. Pure data and pure functions (no React, no DOM), so it is testable under Node and any UI can reuse it.
// The stack lists the OVERLAYS bottom to top. The scalar base layer (the "Show on map" choice) is always drawn first, under every overlay;
// it can be hidden or faded but not reordered. Reordering changes draw order only: nothing here touches data or fetches anything.
import type { OverlayDef, Schema } from 'wb-client';

export type Blend = 'normal' | 'multiply' | 'screen';
export const BLENDS: Blend[] = ['normal', 'multiply', 'screen'];

export interface StackEntry {
  /** Overlay id from the schema. */
  id: string;
  visible: boolean;
  opacity: number;
  blend: Blend;
}

export interface LayerStack {
  /** Overlays, bottom to top. Every overlay of the schema is listed; hidden ones are just not drawn. */
  entries: StackEntry[];
  base: { visible: boolean; opacity: number };
  /** Draw only this layer ('base' or an overlay id). null = no solo. */
  solo: string | null;
  /** Preset this stack was built from, until the user changes anything. */
  preset: string | null;
}

const entry = (id: string, visible: boolean): StackEntry => ({ id, visible, opacity: 1, blend: 'normal' });
const clamp01 = (v: number) => (Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : 1);

/** All overlays hidden, in schema order. */
export function emptyStack(schema: Schema | null): LayerStack {
  return { entries: (schema?.overlays ?? []).map((o) => entry(o.id, false)), base: { visible: true, opacity: 1 }, solo: null, preset: null };
}

/** The stack a layer preset describes: its overlays visible in the preset's bottom-to-top order, the others hidden below them. */
export function stackFromPreset(schema: Schema | null, presetId: string): LayerStack {
  const p = schema?.layer_presets?.find((x) => x.id === presetId);
  if (!schema || !p) return emptyStack(schema);
  const known = new Set((schema.overlays ?? []).map((o) => o.id));
  const shown = p.overlays.filter((id) => known.has(id));
  const hidden = (schema.overlays ?? []).filter((o) => !shown.includes(o.id)).map((o) => entry(o.id, false));
  return { entries: [...hidden, ...shown.map((id) => entry(id, true))], base: { visible: true, opacity: 1 }, solo: null, preset: presetId };
}

const edit = (s: LayerStack, f: (e: StackEntry[]) => StackEntry[], keepPreset = false): LayerStack => ({ ...s, entries: f(s.entries.map((e) => ({ ...e }))), preset: keepPreset ? s.preset : null });

export function setVisible(s: LayerStack, id: string, visible: boolean): LayerStack {
  if (id === 'base') return { ...s, base: { ...s.base, visible }, preset: null };
  return edit(s, (es) => es.map((e) => (e.id === id ? { ...e, visible } : e)));
}

export function setOpacity(s: LayerStack, id: string, opacity: number): LayerStack {
  const o = clamp01(opacity);
  if (id === 'base') return { ...s, base: { ...s.base, opacity: o }, preset: null };
  return edit(s, (es) => es.map((e) => (e.id === id ? { ...e, opacity: o } : e)));
}

export function setBlend(s: LayerStack, id: string, blend: Blend): LayerStack {
  return edit(s, (es) => es.map((e) => (e.id === id ? { ...e, blend } : e)));
}

/** Moves an overlay to position `to` (0 = bottom). Only the order changes. */
export function moveTo(s: LayerStack, id: string, to: number): LayerStack {
  const from = s.entries.findIndex((e) => e.id === id);
  if (from < 0) return s;
  const t = Math.max(0, Math.min(s.entries.length - 1, Math.round(to)));
  if (t === from) return s;
  return edit(s, (es) => { const [e] = es.splice(from, 1); es.splice(t, 0, e); return es; });
}

/** One step up (towards the top, drawn later) or down. */
export const moveBy = (s: LayerStack, id: string, delta: number): LayerStack => {
  const i = s.entries.findIndex((e) => e.id === id);
  return i < 0 ? s : moveTo(s, id, i + delta);
};

/** Solo a layer; soloing the soloed layer again clears it. */
export function toggleSolo(s: LayerStack, id: string): LayerStack { return { ...s, solo: s.solo === id ? null : id }; }

/** Is this layer drawn right now (visibility and solo together)? */
export function isDrawn(s: LayerStack, id: string): boolean {
  if (s.solo !== null) return s.solo === id;
  if (id === 'base') return s.base.visible;
  return !!s.entries.find((e) => e.id === id)?.visible;
}

export interface DrawnOverlay { entry: StackEntry; def: OverlayDef }

/** The overlays to draw, bottom to top (drawn ones only). */
export function drawnOverlays(s: LayerStack, schema: Schema | null): DrawnOverlay[] {
  const defs = new Map((schema?.overlays ?? []).map((o) => [o.id, o]));
  const out: DrawnOverlay[] = [];
  for (const e of s.entries) {
    const def = defs.get(e.id);
    if (def && isDrawn(s, e.id)) out.push({ entry: e, def });
  }
  return out;
}

/** Overlay ids bottom to top, whatever their visibility (what a test compares to prove reordering changes order only). */
export const order = (s: LayerStack): string[] => s.entries.map((e) => e.id);

/** True when the stack still equals the preset it came from (so the panel can say "Hydrology" and not "Custom"). */
export function matchesPreset(s: LayerStack, schema: Schema | null, presetId: string): boolean {
  const ref = stackFromPreset(schema, presetId);
  return JSON.stringify({ e: s.entries, b: s.base, s: s.solo }) === JSON.stringify({ e: ref.entries, b: ref.base, s: ref.solo });
}
