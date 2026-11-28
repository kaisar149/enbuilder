// Part 9g: UI-side controller of the diff view. Owns the view state (playhead, compare mode, range, highlight) in a store of its own, and drives the
// wb-client `DiffController` for data. The playhead timer lives here, not in a viewport, so switching 2D / 3D does not disturb it.
import { DiffController, Store, advancePlayhead, explainRows, type CompareMode, type WbApp } from 'wb-client';
import { initialDiffUi, type DiffUiState } from './diffLayer';

export const PLAY_MS = 4000;
const TICK_MS = 40;

export class DiffUiController {
  readonly ui = new Store<DiffUiState>(initialDiffUi());
  readonly data: DiffController;
  private timer: ReturnType<typeof setInterval> | undefined;
  private loadedKey = '';

  /** `mapLayer()` = the field the map currently shows (a hint for which field to diff first). */
  constructor(readonly app: WbApp, private mapLayer: () => string | null) {
    this.data = new DiffController(app);
    // a newer stored version arrives while the diff is on and the pair follows the latest: reload
    this.data.store.subscribe(() => {
      const s = this.data.state;
      const key = `${s.from}>${s.to}|${s.minRel}`;
      if (this.ui.getState().on && s.from !== null && s.to !== null && key !== this.loadedKey && !s.busy) void this.reload();
    });
  }

  dispose(): void { this.stop(); this.data.dispose(); }

  /** The field being diffed: the user's choice, else the map layer when it changed, else the first changed field. */
  diffField(): string | null {
    const u = this.ui.getState();
    const rows = this.data.state.summary?.fields ?? [];
    if (u.field && (rows.length === 0 || rows.some((r) => r.field === u.field))) return u.field;
    const layer = this.mapLayer();
    if (layer && rows.some((r) => r.field === layer)) return layer;
    return rows[0]?.field ?? layer;
  }

  async setOn(on: boolean): Promise<void> {
    if (!on) { this.stop(); this.ui.setState({ on: false, highlight: [] }); this.data.clearExplanation(); return; }
    this.ui.setState({ on: true, t: 1, highlight: [] });
    await this.data.refreshVersions().catch(() => undefined);
    await this.reload();
  }

  /** Loads the summary and the diff of the chosen field for the chosen pair. */
  async reload(): Promise<void> {
    const s = this.data.state;
    if (s.from === null || s.to === null) return;
    this.loadedKey = `${s.from}>${s.to}|${s.minRel}`;
    this.data.clearExplanation();
    this.ui.setState({ highlight: [] });
    await this.data.loadSummary();
    const f = this.diffField();
    if (f) await this.data.loadDiff(f);
  }

  async selectPair(from: number, to: number): Promise<void> { this.data.select(from, to); await this.reload(); }
  async followLatest(): Promise<void> { this.data.followLatest(); await this.data.refreshVersions().catch(() => undefined); await this.reload(); }

  async setField(field: string): Promise<void> {
    this.ui.setState({ field, highlight: [] });
    this.data.clearExplanation();
    await this.data.loadDiff(field);
  }

  async setHideSmall(on: boolean): Promise<void> {
    this.ui.setState({ hideSmall: on });
    this.data.setMinRel(on ? 0.01 : 0);
    await this.reload();
  }

  // ---- animation --------------------------------------------------------------------------------------------------------------
  play(): void {
    this.stop();
    const start = this.ui.getState().t >= 1 ? 0 : this.ui.getState().t;
    this.ui.setState({ t: start, playing: true });
    this.timer = setInterval(() => {
      const r = advancePlayhead(this.ui.getState().t, TICK_MS, PLAY_MS);
      this.ui.setState({ t: r.t });
      if (r.done) this.stop();
    }, TICK_MS);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    if (this.ui.getState().playing) this.ui.setState({ playing: false });
  }

  setT(t: number): void { this.stop(); this.ui.setState({ t: Math.min(1, Math.max(0, t)) }); }
  setMode(mode: CompareMode): void { this.ui.setState({ mode }); }
  /** Flip button: before <-> after (anything else goes to `after`). */
  flip(): void { this.ui.setState({ mode: this.ui.getState().mode === 'after' ? 'before' : 'after' }); }
  setSwipe(swipe: number): void { this.ui.setState({ swipe: Math.min(1, Math.max(0, swipe)) }); }
  setRange(rangeFrac: number): void { this.ui.setState({ rangeFrac: Math.min(1, Math.max(0.01, rangeFrac)) }); }

  // ---- explainer --------------------------------------------------------------------------------------------------------------
  /** A click on the map while the diff is on: ask the core why this cell changed (for the diffed field). */
  async onPick(cell: number | null): Promise<void> {
    if (!this.ui.getState().on) return;
    this.ui.setState({ highlight: [] });
    const f = this.diffField();
    if (cell === null || !f) { this.data.clearExplanation(); return; }
    await this.data.explain(cell, f);
  }

  /** Click on a step of the explanation: highlight its source cells. Same step again clears. */
  highlightStep(index: number | null): void {
    const ex = this.data.state.explanation;
    if (!ex || index === null) { this.ui.setState({ highlight: [] }); return; }
    const row = explainRows(ex)[index];
    this.ui.setState({ highlight: row ? row.highlight : [] });
  }
}
