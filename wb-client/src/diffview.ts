// Part 9g: pure models for the diff view. No framework, no rendering types, no physics: they only reshape what `get_diff` and `explain` returned.
import type { DiffData, ExplainStep, Explanation, MeshData } from './types';

/** Number of changed cells revealed at playhead `t` in [0, 1] (cells with 0 <= front <= t). */
export function revealedCount(d: DiffData, t: number): number {
  let k = 0;
  for (let i = 0; i < d.nCells; i++) if (d.front[i] >= 0 && d.front[i] <= t) k++;
  return k;
}

/** The delta layer at playhead `t`: changed cells already reached by the front show their change, every other cell shows 0. t = 1 shows all changes. */
export function frameDelta(d: DiffData, t: number): Float32Array {
  const out = new Float32Array(d.nCells);
  for (let i = 0; i < d.nCells; i++) if (d.front[i] >= 0 && d.front[i] <= t) out[i] = d.delta[i];
  return out;
}

/** Symmetric colour range [-m, m] for a diverging colormap centred on zero. `frac` in (0, 1] is the range slider (1 = the largest change). */
export function symmetricRange(d: DiffData, frac = 1): [number, number] {
  const peak = d.summary.peak_abs > 0 ? d.summary.peak_abs : 1;
  const m = peak * Math.min(1, Math.max(1e-6, frac));
  return [-m, m];
}

/** Moves the playhead. `durationMs` = time for a full 0 -> 1 run. Returns the new t and whether the run ended. */
export function advancePlayhead(t: number, dtMs: number, durationMs: number): { t: number; done: boolean } {
  const n = Math.min(1, t + dtMs / Math.max(1, durationMs));
  return { t: n, done: n >= 1 };
}

/** A sweep coordinate in [0, 1] per cell for the before/after swipe: x on a plane, longitude (atan2(y, x)) on a sphere or cube. Mesh space, so any viewport can use it. */
export function sweepCoord(mesh: MeshData): Float32Array {
  const n = mesh.nCells;
  const raw = new Float32Array(n);
  const plane = mesh.kind === 'plane';
  for (let i = 0; i < n; i++) {
    const x = mesh.centroid[3 * i], y = mesh.centroid[3 * i + 1];
    raw[i] = plane ? x : Math.atan2(y, x);
  }
  let lo = Infinity, hi = -Infinity;
  for (const v of raw) { lo = Math.min(lo, v); hi = Math.max(hi, v); }
  const span = hi - lo || 1;
  for (let i = 0; i < n; i++) raw[i] = (raw[i] - lo) / span;
  return raw;
}

export type CompareMode = 'diff' | 'before' | 'after' | 'swipe';

/** Values of the underlying field for the before/after modes. Swipe: cells left of the handle `s` (sweep coordinate < s) show AFTER, the rest BEFORE. */
export function compareValues(d: DiffData, mode: Exclude<CompareMode, 'diff'>, sweep?: Float32Array, s = 0.5): Float32Array {
  if (mode === 'before') return d.before;
  if (mode === 'after') return d.after;
  const out = new Float32Array(d.nCells);
  for (let i = 0; i < d.nCells; i++) out[i] = (sweep ? sweep[i] < s : true) ? d.after[i] : d.before[i];
  return out;
}

// ---- explainer ---------------------------------------------------------------------------------------------------------------

const sign = (v: number) => (v < 0 || Object.is(v, -0) ? '-' : '+');

/** One link in the core's own line format (`Explanation::text`, without the module / origin suffix): `field @cell c: old -> new (+delta)`. */
export function stepLine(s: ExplainStep): string {
  return `${s.field} @cell ${s.cell}: ${s.old.toFixed(4)} -> ${s.new.toFixed(4)} (${sign(s.delta)}${Math.abs(s.delta).toFixed(4)})`;
}

/** The lines of the core text that describe links (one per step, in order). */
export function coreLinkLines(text: string): string[] { return text.split('\n').filter((l) => l.includes(' @cell ')); }

/** What the step says about its cause, from the structured fields only. */
export function causeText(s: ExplainStep): string {
  switch (s.hop) {
    case 'local': return `${s.module} responded to an input that changed ${s.dist_km.toFixed(0)} km away`;
    case 'global': return `${s.module}: global coupling, no nearby cause (best ranked cause ${s.dist_km.toFixed(0)} km away)`;
    case 'origin': return 'origin of the chain';
    default: return 'the chain stops here';
  }
}

export interface StepRow {
  index: number;
  field: string;
  cell: number;
  /** Raw core line (exactly the numbers the core measured). */
  line: string;
  cause: string;
  modules: string[];
  anomalies: string[];
  contributors: string[];
  also: string[];
  feedback: string[];
  /** Cells to highlight when the step is clicked. */
  highlight: number[];
}

/** The explanation as ordered rows. Pure reshaping: one row per step, nothing added or inferred. */
export function explainRows(ex: Explanation): StepRow[] {
  return ex.steps.map((s) => ({
    index: s.index, field: s.field, cell: s.cell, line: stepLine(s), cause: causeText(s), modules: s.module ? [s.module, ...s.modules.filter((m) => m !== s.module)] : s.modules,
    anomalies: s.anomalies, contributors: s.contributors, also: s.also.map((a) => `${a.field} @cell ${a.cell}`), feedback: s.via_feedback,
    highlight: [s.cell, ...s.source_cells],
  }));
}
