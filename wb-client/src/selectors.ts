// Pure functions of the store state and the schema. Nothing here knows a field name: lists, ranges, labels and palettes all come from the schema.
import { fieldKey, type WbState } from './store';
import type { FieldData, FieldMeta, LayerDecl, ParamDef, Schema, WorldInfo } from './types';

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

export function paramGroups(schema: Schema | null): { group: string; params: ParamDef[] }[] {
  const out: { group: string; params: ParamDef[] }[] = [];
  for (const p of schema?.params ?? []) {
    let g = out.find((x) => x.group === p.group);
    if (!g) out.push((g = { group: p.group, params: [] }));
    g.params.push(p);
  }
  return out;
}

export const selectField = (s: WbState, name: string, layer = 0): FieldData | undefined => s.fields[fieldKey(name, layer)];
export const isJobRunning = (s: WbState): boolean => s.jobs.current?.status === 'running' || !!s.world?.job;
