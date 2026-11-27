// Part 9e: the cell inspector's model. Pure: it formats what `get_cell` returned (one cell, optionally a pinned second one) and groups it by the
// module that produces each field. It never computes a value: `raw` is exactly the number the core sent; only the unit conversion of the
// schema (`display.scale/offset`) is applied for the text.
import { displayValue, metaOf, prettyName, type CellInfo, type Schema } from 'wb-client';

export interface SeasonRow {
  /** 12 values exactly as sent (SI units). */
  raw: number[];
  text: string[];
  min: string;
  max: string;
}

export interface InspectorRow {
  name: string;
  label: string;
  unit: string;
  /** Value as sent by the core (SI). */
  raw: number;
  text: string;
  /** The same field at the pinned cell, when comparing. */
  other?: { raw: number; text: string; delta: string };
  seasons?: SeasonRow;
}

export interface InspectorGroup {
  id: string;
  title: string;
  rows: InspectorRow[];
}

export interface InspectorModel {
  title: string;
  subtitle: string | null;
  stale: boolean;
  groups: InspectorGroup[];
  /** Title of the pinned cell column, when comparing. */
  pinnedTitle: string | null;
}

export function fmtValue(v: number): string {
  if (!Number.isFinite(v)) return 'none';
  return Math.abs(v) >= 1000 || (v !== 0 && Math.abs(v) < 0.01) ? v.toExponential(2) : v.toFixed(2);
}

export function cellTitle(info: CellInfo): string {
  const pos = info.lat_deg !== undefined && info.lon_deg !== undefined ? `, ${info.lat_deg.toFixed(1)}°, ${info.lon_deg.toFixed(1)}°` : '';
  return `Cell ${info.cell}${pos}`;
}

const textOf = (schema: Schema | null, name: string, v: number): string => {
  const meta = metaOf(schema, name);
  return meta ? `${fmtValue(displayValue(meta, v))} ${meta.display.unit}`.trim() : fmtValue(v);
};

/** Group id of a field: the first module that produces it (schema order = pipeline order), else 'other'. */
export function groupOf(schema: Schema | null, name: string): string {
  const by = metaOf(schema, name)?.produced_by;
  return by && by.length ? by[0] : 'other';
}

export function inspectorModel(schema: Schema | null, info: CellInfo, pinned?: CellInfo | null): InspectorModel {
  const order = new Map<string, number>((schema?.modules ?? []).map((m, i) => [m.name, i]));
  const groups = new Map<string, InspectorGroup>();
  for (const [name, raw] of Object.entries(info.values)) {
    const meta = metaOf(schema, name);
    const id = groupOf(schema, name);
    let g = groups.get(id);
    if (!g) { g = { id, title: id === 'other' ? 'Other' : prettyName(id), rows: [] }; groups.set(id, g); }
    const row: InspectorRow = { name, label: prettyName(name), unit: meta?.display.unit ?? '', raw, text: textOf(schema, name, raw) };
    if (pinned && name in pinned.values) {
      const o = pinned.values[name];
      const d = meta ? displayValue(meta, raw) - displayValue(meta, o) : raw - o;
      row.other = { raw: o, text: textOf(schema, name, o), delta: Number.isFinite(d) ? `${d > 0 ? '+' : ''}${fmtValue(d)}` : 'none' };
    }
    const s = info.seasonal?.[name];
    if (s && s.length) {
      const shown = s.map((v) => (meta ? displayValue(meta, v) : v));
      row.seasons = { raw: [...s], text: shown.map(fmtValue), min: fmtValue(Math.min(...shown)), max: fmtValue(Math.max(...shown)) };
    }
    g.rows.push(row);
  }
  const list = [...groups.values()].sort((a, b) => (order.get(a.id) ?? 1e9) - (order.get(b.id) ?? 1e9) || a.title.localeCompare(b.title));
  for (const g of list) g.rows.sort((a, b) => a.label.localeCompare(b.label));
  return { title: cellTitle(info), subtitle: info.biome_name, stale: info.stale, groups: list, pinnedTitle: pinned ? cellTitle(pinned) : null };
}
