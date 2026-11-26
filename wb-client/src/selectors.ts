// Pure functions of the store state and the schema. Nothing here knows a field name: lists, ranges, labels and palettes all come from the schema.
import { fieldKey, type WbState } from './store';
import type { FieldData, FieldMeta, HistoryEntry, LayerDecl, ParamDef, Schema, WorldInfo } from './types';

const LAYER_KINDS = ['scalar', 'mask', 'category'];

export interface LayerChoice {
  field: string;
  label: string;
  group: 'featured' | 'other';
  kind: FieldMeta['kind'];
  /** The server has this field now (elevation always; the rest after a solve). */
  available: boolean;
  /** Why a field cannot be shown yet, for a tooltip. */
  note: string;
}

export const prettyName = (name: string): string => {
  const s = name.replace(/_/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
};

/** Layer list generated from the schema: curated fields first (those produced by the world itself before the rest), then the others. */
export function layerChoices(schema: Schema | null, world: WorldInfo | null): LayerChoice[] {
  if (!schema) return [];
  const have = new Set(world?.fields.map((f) => f.name) ?? []);
  const items = schema.fields
    .filter((f) => LAYER_KINDS.includes(f.kind))
    .map((f): LayerChoice & { base: boolean } => ({
      field: f.name,
      label: prettyName(f.name),
      group: f.featured ? 'featured' : 'other',
      kind: f.kind,
      available: have.has(f.name),
      note: have.has(f.name) ? f.description : 'press Solve first',
      base: (f.produced_by ?? []).includes('world'),
    }));
  const rank = (c: LayerChoice & { base: boolean }) => (c.group === 'featured' ? 0 : 1) * 2 + (c.base ? 0 : 1);
  items.sort((a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label));
  return items.map(({ base: _b, ...c }) => c);
}

/** First layer that can be shown right now (so the map is never empty on connect). */
export function defaultLayer(choices: LayerChoice[]): string | null { return choices.find((c) => c.available)?.field ?? null; }

export function metaOf(schema: Schema | null, name: string): FieldMeta | undefined { return schema?.fields.find((f) => f.name === name); }

/** Colour range rule shared with the Rust renderer (`raster::value_range`): the schema's fixed range, else the finite min/max (hi <= lo widens by 1). */
export function resolveRange(meta: FieldMeta, min: number, max: number): [number, number] {
  if (meta.range) return [meta.range[0], meta.range[1]];
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0, 1];
  if (max <= min) return [min, min + 1];
  return [min, max];
}

/** `{ id, field, kind, colormap, range, opacity }` (UI_ARCHITECTURE section 3). */
export function layerDecl(schema: Schema, field: FieldData, opacity = 1): LayerDecl {
  const meta = metaOf(schema, field.name) ?? field.meta;
  return {
    id: `${field.name}#${field.layer}`,
    field: field.name,
    kind: meta.kind,
    colormap: meta.colormap,
    range: resolveRange(meta, field.min, field.max),
    opacity,
    palette: meta.palette,
  };
}

/** Same short number format as the Rust legend (`raster::fmt_num`). */
export function fmtNum(v: number): string {
  const a = Math.abs(v);
  if (v === 0) return '0';
  if (a >= 1e5 || a < 1e-2) return v.toExponential(1).replace('e+', 'e').replace(/e(-?)0*(\d)/, 'e$1$2');
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  return v.toFixed(2);
}

export type Legend =
  | { type: 'gradient'; colormap: string; lo: number; hi: number; loLabel: string; hiLabel: string; unit: string }
  | { type: 'categories'; items: { id: number; name: string; color: [number, number, number] | null }[] };

/** Legend data for a layer; labels use the schema's display scale/offset/unit (temperature in degC, rain in mm/day, ...). */
export function legendFor(schema: Schema, decl: LayerDecl, field: FieldData): Legend {
  const meta = metaOf(schema, decl.field) ?? field.meta;
  if (decl.kind === 'category') {
    const present = new Set<number>();
    for (const v of field.values) if (Number.isFinite(v)) present.add(Math.round(v));
    const useBiomes = decl.palette === 'biomes';
    const items = [...present].sort((a, b) => a - b).map((id) => {
      const b = useBiomes ? schema.biomes.find((x) => x.id === id) : undefined;
      return { id, name: b?.name ?? `class ${id}`, color: b ? b.color : null };
    });
    return { type: 'categories', items };
  }
  const { scale, offset, unit } = meta.display;
  const [lo, hi] = decl.range;
  return { type: 'gradient', colormap: decl.colormap, lo: lo * scale + offset, hi: hi * scale + offset, loLabel: fmtNum(lo * scale + offset), hiLabel: fmtNum(hi * scale + offset), unit };
}

/** Value of a cell in display units, for hover read-outs. */
export function displayValue(meta: FieldMeta, v: number): number { return v * meta.display.scale + meta.display.offset; }

export interface ParamGroupView {
  /** Group key as used by `params[].group`. */
  group: string;
  title: string;
  advanced: boolean;
  params: ParamDef[];
}

/**
 * Parameter groups for the generated panel. Order and titles come from `schema.groups`; a parameter whose group is not listed there still
 * appears (in a group named after it, not advanced), so adding a parameter in Rust never needs a UI change.
 */
export function paramGroups(schema: Schema | null): ParamGroupView[] {
  const out: ParamGroupView[] = [];
  const byName = new Map<string, ParamGroupView>();
  for (const g of schema?.groups ?? []) {
    const v: ParamGroupView = { group: g.name, title: g.title, advanced: g.advanced, params: [] };
    out.push(v);
    byName.set(g.name, v);
  }
  for (const p of schema?.params ?? []) {
    let v = byName.get(p.group);
    if (!v) {
      v = { group: p.group, title: prettyName(p.group), advanced: false, params: [] };
      out.push(v);
      byName.set(p.group, v);
    }
    v.params.push(p);
  }
  return out.filter((g) => g.params.length > 0);
}

export const isDefault = (def: ParamDef, value: number | undefined): boolean => value === undefined || value === def.default;

/** Number of decimals worth showing for a parameter (from its range and widget). */
export function paramDecimals(def: ParamDef): number {
  if (def.widget === 'integer' || def.widget === 'select') return 0;
  const span = def.max - def.min;
  return span <= 1 ? 3 : span <= 10 ? 2 : span <= 100 ? 1 : 0;
}

export function fmtParam(def: ParamDef, value: number): string {
  if (def.widget === 'select' && def.options) return def.options[value] ?? String(value);
  return value.toFixed(paramDecimals(def)).replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, '');
}

export interface PresetStatus {
  /** The preset this world is measured against (null when the schema has no presets). */
  base: string | null;
  /** True when some non-identity parameter differs from the base preset. */
  dirty: boolean;
  /** Non-identity parameters that differ from the base preset. */
  diffs: { name: string; current: number; expected: number }[];
}

/** What `load_preset(name)` sets: every parameter's default, then the preset's overrides (same rule as the core). */
export function presetTarget(schema: Schema, name: string): Record<string, number> | null {
  const p = schema.presets.find((x) => x.name === name);
  if (!p) return null;
  const t: Record<string, number> = {};
  for (const d of schema.params) t[d.name] = d.default;
  return { ...t, ...(p.params ?? {}) };
}

function diffAgainst(schema: Schema, params: Record<string, number>, target: Record<string, number>): PresetStatus['diffs'] {
  const out: PresetStatus['diffs'] = [];
  for (const d of schema.params) {
    if (d.identity) continue;
    const cur = params[d.name];
    if (cur !== undefined && cur !== target[d.name]) out.push({ name: d.name, current: cur, expected: target[d.name] });
  }
  return out;
}

/**
 * Dirty-vs-preset indicator. `base` is the preset the world reports (`world.preset`) if it is a real one, else `hint` (the preset the user last
 * picked, UI state), else the closest preset (fewest differing parameters). Identity parameters (shape, resolution, radius, seed) never count.
 */
export function presetStatus(schema: Schema | null, params: Record<string, number> | undefined, worldPreset?: string, hint?: string | null): PresetStatus {
  if (!schema || !params || schema.presets.length === 0) return { base: null, dirty: false, diffs: [] };
  const known = (n?: string | null) => (n && schema.presets.some((p) => p.name === n) ? n : null);
  let base = known(worldPreset) ?? known(hint);
  if (!base) {
    let best = Infinity;
    for (const p of schema.presets) {
      const n = diffAgainst(schema, params, presetTarget(schema, p.name)!).length;
      if (n < best) { best = n; base = p.name; }
    }
  }
  const diffs = diffAgainst(schema, params, presetTarget(schema, base!)!);
  return { base, dirty: diffs.length > 0, diffs };
}

/** Random seed for the "random seed" button, inside the schema's range for `seed` (`rng` is injectable for tests). */
export function randomSeed(schema: Schema | null, rng: () => number = Math.random): number {
  const d = schema?.params.find((p) => p.name === 'seed');
  const lo = d?.min ?? 0, hi = d?.max ?? 1e9;
  return lo + Math.floor(rng() * (hi - lo + 1));
}

// ---- seasons ------------------------------------------------------------------------------------------------------------------

/** The multi-layer season field that stands in for `field` (temperature -> season_temperature), or null if the schema has none. */
export function seasonVariant(schema: Schema | null, field: string): string | null { return schema?.seasons?.variants[field] ?? null; }

/** Seasons can be shown when the schema declares them, the world has orbital seasons (year_length_days > 0) and the variant field was solved. */
export function seasonsAvailable(schema: Schema | null, world: WorldInfo | null, field: string): boolean {
  const v = seasonVariant(schema, field);
  if (!v || !schema?.seasons || !world) return false;
  if ((world.params.year_length_days ?? 0) <= 0) return false;
  return world.fields.some((f) => f.name === v && f.layers === schema.seasons!.count);
}

/** Label for season layer k of n: the day of the year it is centred on (layer 0 starts at day 0). */
export function seasonLabel(layer: number, count: number, yearDays: number): string {
  const day = Math.round(((layer + 0.5) / count) * yearDays);
  return `Season ${layer + 1} of ${count} (day ${day})`;
}

/** Field to draw in a variation thumbnail: the first kept field the schema calls a category (biomes), else the first kept field. */
export function variationThumbField(schema: Schema | null, info: { fields: string[] } | null): string | null {
  if (!info || info.fields.length === 0) return null;
  return info.fields.find((f) => metaOf(schema, f)?.kind === 'category') ?? info.fields[0];
}

// ---- history / variations -----------------------------------------------------------------------------------------------------

export const historyUndo = (w: WorldInfo | null): HistoryEntry[] => w?.history?.undo ?? [];
export const historyRedo = (w: WorldInfo | null): HistoryEntry[] => w?.history?.redo ?? [];

export const selectField = (s: WbState, name: string, layer = 0): FieldData | undefined => s.fields[fieldKey(name, layer)];
export const isJobRunning = (s: WbState): boolean => s.jobs.current?.status === 'running' || !!s.world?.job;
