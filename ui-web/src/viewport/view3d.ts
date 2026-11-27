// Pure helpers that turn world info + UI state into what the 3D viewport needs. No DOM, no Three.js: testable under Node.
import type { CellInfo, MeshData, Schema, WorldInfo } from 'wb-client';
import { displayValue, metaOf, prettyName } from 'wb-client';
import cfg from './view3d.config.json';
import { sunDirection } from './sun';
import { DEFAULT_VIEW3D, type View3DOptions } from './types';

export const RELIEF_FIELD: string = cfg.reliefField;

export interface View3DState {
  exaggeration: number;
  lighting: 'flat' | 'sun';
  dayNight: boolean;
  /** 0..1 of the day, 0.5 = sun over the +X meridian. */
  timeOfDay: number;
  /** 0-based stored season layer, or null for the yearly mean (sun at the equinox). */
  season: number | null;
}

export const initialView3D = (): View3DState => ({ exaggeration: DEFAULT_VIEW3D.exaggeration, lighting: DEFAULT_VIEW3D.lighting, dayNight: false, timeOfDay: 0.5, season: null });

const radiusCache = new Map<string, number>();
/** Bounding radius of a mesh in mesh units (same meaning as CellGeometry.radius, cheap: one pass over the centroids). */
export function meshRadius(mesh: MeshData): number {
  const hit = radiusCache.get(mesh.meshId);
  if (hit !== undefined) return hit;
  const c = mesh.centroid, n = mesh.nCells;
  let r = 0;
  if (mesh.kind === 'cube' || mesh.kind === 'plane') {
    for (let i = 0; i < n; i++) r = Math.max(r, Math.abs(c[3 * i]), Math.abs(c[3 * i + 1]), mesh.kind === 'cube' ? Math.abs(c[3 * i + 2]) : 0);
  } else {
    for (let i = 0; i < n; i++) r += Math.hypot(c[3 * i], c[3 * i + 1], c[3 * i + 2]);
    r /= Math.max(n, 1);
  }
  if (radiusCache.size > 8) radiusCache.clear();
  radiusCache.set(mesh.meshId, r);
  return r;
}

/** Everything `Viewport3D.setView` takes, derived from the world's parameters (nothing is sent to the core). */
export function view3DOptions(v: View3DState, world: WorldInfo | null, mesh: MeshData | null, seasonCount: number): View3DOptions {
  const p = world?.params ?? {};
  const plane = (world?.domain.kind ?? mesh?.kind) === 'plane';
  const season = v.season !== null && seasonCount > 0 ? v.season / seasonCount : 0;
  const sunDir = sunDirection(
    { axial_tilt_deg: p.axial_tilt_deg ?? 0, year_length_days: p.year_length_days ?? 0, day_length_h: p.day_length_h ?? 0 }, plane, season, v.timeOfDay,
  );
  const km = p.radius_km ?? 0;
  const unitsPerMetre = mesh && km > 0 ? meshRadius(mesh) / (km * 1000) : DEFAULT_VIEW3D.unitsPerMetre;
  return { exaggeration: v.exaggeration, lighting: v.lighting, dayNight: v.dayNight, sunDir, unitsPerMetre };
}

/** Rows of the hover tooltip: the shown layer first, then the schema's featured scalar fields (temperature, rainfall...), at most `max`. */
export function tooltipRows(schema: Schema | null, info: CellInfo, shown: string | null, max = 4): { label: string; text: string }[] {
  if (!schema) return [];
  const names: string[] = [];
  if (shown) names.push(shown);
  // headline fields first: the schema gives those a curated colour range (elevation, temperature, rainfall...); the rest follow in schema order
  const rest = schema.fields.filter((f) => f.featured && f.kind !== 'category' && !names.includes(f.name));
  for (const f of [...rest.filter((f) => f.range), ...rest.filter((f) => !f.range)]) names.push(f.name);
  const rows: { label: string; text: string }[] = [];
  for (const n of names) {
    const raw = info.values[n];
    const meta = metaOf(schema, n);
    // a vector or line field has no single number; masks are not readable values
    if (raw === undefined || !meta || meta.kind === 'category' || meta.kind === 'vector' || meta.kind === 'lines' || meta.kind === 'mask') continue;
    const v = displayValue(meta, raw);
    rows.push({ label: prettyName(n), text: Number.isFinite(v) ? `${v.toFixed(Math.abs(v) >= 100 ? 0 : 1)} ${meta.display.unit}`.trim() : 'no value' });
    if (rows.length >= max) break;
  }
  return rows;
}
