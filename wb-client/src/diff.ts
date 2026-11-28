// Part 9g: `DiffController` keeps the diff session state (stored versions, the chosen pair, the diffed field, summary, explanation) in sync with the
// server. It holds NO view state (playhead, swipe position, colour range are UI-only); it only fetches and caches.
import { Store, type WbApp } from './store';
import type { DiffData, DiffSummary, Explanation, VersionsData } from './types';

export interface DiffState {
  versions: VersionsData | null;
  from: number | null;
  to: number | null;
  field: string | null;
  minRel: number;
  summary: DiffSummary | null;
  diff: DiffData | null;
  explanation: Explanation | null;
  busy: boolean;
  error: string | null;
}

export class DiffController {
  readonly store = new Store<DiffState>({ versions: null, from: null, to: null, field: null, minRel: 0.01, summary: null, diff: null, explanation: null, busy: false, error: null });
  private cache = new Map<string, DiffData>();
  private unsub: () => void;
  private lastSig = '';
  private seq = 0;
  /** True until the user picks a pair: then the pair follows the newest versions (previous, latest). */
  private auto = true;

  constructor(readonly app: WbApp) {
    // new stored versions appear when a solve or refresh installs fields: refetch the list when any field version moves
    this.unsub = app.store.subscribe(() => {
      const w = app.store.getState().world;
      const sig = w ? `${w.domain.mesh_id}|${w.solved}|${w.fields.map((f) => f.version).join(',')}` : '';
      if (sig !== this.lastSig) {
        this.lastSig = sig;
        void this.refreshVersions().catch(() => undefined);
      }
    });
  }

  dispose(): void { this.unsub(); }

  get state(): DiffState { return this.store.getState(); }

  /** Re-reads the list. Keeps the chosen pair when both versions still exist, otherwise defaults to (previous, latest). */
  async refreshVersions(): Promise<VersionsData> {
    const v = await this.app.client.getVersions();
    const ids = v.versions.map((x) => x.version);
    const s = this.state;
    let { from, to } = s;
    if (this.auto || from === null || to === null || !ids.includes(from) || !ids.includes(to)) {
      to = ids.length ? ids[ids.length - 1] : null;
      from = ids.length > 1 ? ids[ids.length - 2] : to;
    }
    const pairChanged = from !== s.from || to !== s.to;
    this.store.setState({ versions: v, from, to, ...(pairChanged ? { summary: null, diff: null, explanation: null } : {}) });
    return v;
  }

  /** Picks a pair by hand; from now on the pair stays put while newer versions arrive. */
  select(from: number, to: number): void {
    this.auto = false;
    this.store.setState({ from, to, summary: null, diff: null, explanation: null, error: null });
  }

  /** Goes back to following the newest versions. */
  followLatest(): void { this.auto = true; void this.refreshVersions().catch(() => undefined); }

  setMinRel(minRel: number): void {
    this.cache.clear();
    this.store.setState({ minRel, summary: null, diff: null, explanation: null });
  }

  async loadSummary(): Promise<DiffSummary | null> {
    const { from, to, minRel } = this.state;
    if (from === null || to === null) return null;
    const my = ++this.seq;
    this.store.setState({ busy: true, error: null });
    try {
      const summary = await this.app.client.getDiffSummary(from, to, minRel);
      if (my === this.seq) this.store.setState({ summary, busy: false });
      return summary;
    } catch (e) {
      this.store.setState({ busy: false, error: (e as Error).message });
      return null;
    }
  }

  /** The diff of `field` for the chosen pair (cached per pair, field and threshold). */
  async loadDiff(field: string): Promise<DiffData | null> {
    const { from, to, minRel } = this.state;
    if (from === null || to === null) return null;
    const key = `${from}>${to}|${field}|${minRel}`;
    let d = this.cache.get(key);
    try {
      if (!d) {
        this.store.setState({ busy: true, error: null });
        d = await this.app.client.getDiff(field, from, to, minRel);
        this.cache.set(key, d);
        if (this.cache.size > 24) this.cache.delete(this.cache.keys().next().value as string);
      }
      this.store.setState({ field, diff: d, busy: false });
      return d;
    } catch (e) {
      this.store.setState({ busy: false, error: (e as Error).message });
      return null;
    }
  }

  async explain(cell: number, field: string): Promise<Explanation | null> {
    const { from, to, minRel } = this.state;
    if (from === null || to === null) return null;
    try {
      const ex = await this.app.client.explain(cell, field, from, to, minRel);
      this.store.setState({ explanation: ex, error: null });
      return ex;
    } catch (e) {
      this.store.setState({ error: (e as Error).message });
      return null;
    }
  }

  clearExplanation(): void { this.store.setState({ explanation: null }); }
}
