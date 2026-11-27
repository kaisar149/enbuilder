// Part 9f: EditController. Owns the editing session on the client: active tool, brush settings, the local (uncommitted) stroke, and the
// follow-up refresh chain (preview, then refined). Framework-free; a UI only calls pointerDown / pointerMove / pointerUp with picked cell ids.
import { Store, type WbApp } from './store';
import { StrokeBuffer, ToolRegistry, defaultBrush, meshGeometry, type BrushSettings, type MeshGeometry, type Tool, type ToolContext } from './tools';
import type { Ack, Command, JobStatus, OutputField, RefreshMode } from './types';

export type EditPhase = 'idle' | 'stroking' | 'committing' | 'previewing' | 'refining' | 'error';

export interface EditState {
  toolId: string | null;
  brush: BrushSettings;
  phase: EditPhase;
  /** The uncommitted stroke (for the UI's stroke preview). Replaced on every change. */
  preview: { cells: Uint32Array; weights: Float32Array } | null;
  /** Commands sent for strokes / locks so far (tests and devtools use it to prove batching). */
  strokesCommitted: number;
  lastDetail: Ack['detail'] | null;
  /** Result of the last refresh chain: which modes completed, in order. */
  lastChain: RefreshMode[];
  error: string | null;
}

export interface EditOptions {
  /** After a preview completes, run a refined refresh (default true). */
  autoRefine?: boolean;
  /** Start a refresh chain after edits that change the solver's inputs (default true). */
  autoRefresh?: boolean;
}

/** Output fields whose paint / locks the solver actually uses; the rest are applied instantly as a view. */
const SOLVER_LAYERS = new Set(['elevation']);
const SOLVER_LOCKS = new Set<OutputField>(['elevation', 'temperature']);

export function needsRefresh(cmd: Command): boolean {
  if (cmd.type === 'paint_stroke') return SOLVER_LAYERS.has(cmd.layer);
  if (cmd.type === 'set_lock') return SOLVER_LOCKS.has(cmd.field);
  return false;
}

export class EditController {
  readonly store: Store<EditState>;
  readonly stroke = new StrokeBuffer();
  private registry: ToolRegistry;
  private geom: MeshGeometry | null = null;
  private geomMesh: object | null = null;
  /** Bumped by every new stroke; a refresh chain that notices a newer value stops. */
  private generation = 0;
  private chain: Promise<void> = Promise.resolve();

  constructor(readonly app: WbApp, private opts: EditOptions = {}) {
    const edit = app.store.getState().schema?.edit;
    this.registry = edit ? ToolRegistry.fromSchema(edit) : new ToolRegistry();
    const first = this.registry.list()[0];
    this.store = new Store<EditState>({
      toolId: first?.id ?? null, brush: defaultBrush(edit), phase: 'idle', preview: null, strokesCommitted: 0, lastDetail: null, lastChain: [], error: null,
    });
  }

  get tools(): Tool[] { return this.registry.list(); }
  get tool(): Tool | undefined { const id = this.store.getState().toolId; return id ? this.registry.get(id) : undefined; }
  get state(): EditState { return this.store.getState(); }

  setTool(id: string): void {
    if (!this.registry.has(id)) throw new Error(`unknown tool '${id}'`);
    this.store.setState({ toolId: id });
  }

  setBrush(patch: Partial<BrushSettings>): void {
    const edit = this.app.store.getState().schema?.edit;
    const b = { ...this.store.getState().brush, ...patch };
    if (edit) {
      b.radiusKm = Math.min(edit.brush.radius_max_km, Math.max(edit.brush.radius_min_km, b.radiusKm));
    }
    b.strength = Math.min(1, Math.max(0, b.strength));
    this.store.setState({ brush: b });
  }

  /** Resolves when the refresh chain started by the latest edit has finished (for tests and for "wait until up to date"). */
  idle(): Promise<void> { return this.chain; }

  private ctx(): ToolContext | null {
    const mesh = this.app.store.getState().mesh;
    if (!mesh) return null;
    if (this.geomMesh !== mesh) { this.geom = meshGeometry(mesh); this.geomMesh = mesh; }
    return { mesh, geom: this.geom as MeshGeometry, brush: this.store.getState().brush, stroke: this.stroke, changed: () => this.publishPreview() };
  }

  private publishPreview(): void {
    this.store.setState({ preview: this.stroke.size ? this.stroke.toArrays() : null });
  }

  // ---- pointer events (cells come from viewport.pick) ---------------------------------------------------------------------------
  pointerDown(cell: number): void {
    const tool = this.tool;
    const ctx = this.ctx();
    if (!tool || !ctx) return;
    this.generation++; // a stroke in progress supersedes any refresh chain still waiting
    this.stroke.clear();
    this.store.setState({ phase: 'stroking', error: null });
    tool.onPick(ctx, cell);
  }

  pointerMove(cell: number): void {
    if (this.store.getState().phase !== 'stroking') return;
    const ctx = this.ctx();
    if (ctx) this.tool?.onDrag(ctx, cell);
  }

  /** Commits the stroke as ONE command; then (for edits the solver cares about) runs preview, then refined. Resolves when the commit is acknowledged. */
  async pointerUp(): Promise<Ack | null> {
    if (this.store.getState().phase !== 'stroking') return null;
    const tool = this.tool;
    const ctx = this.ctx();
    if (!tool || !ctx) return null;
    const cmd = tool.onRelease(ctx);
    this.stroke.clear();
    if (!cmd) {
      this.store.setState({ phase: 'idle', preview: null });
      return null;
    }
    this.store.setState({ phase: 'committing' });
    let ack: Ack;
    try {
      ack = await this.app.command(cmd);
    } catch (e) {
      this.store.setState({ phase: 'error', preview: null, error: (e as Error).message });
      return null;
    }
    this.store.setState((s) => ({ preview: null, strokesCommitted: s.strokesCommitted + 1, lastDetail: ack.detail ?? null, phase: 'idle' }));
    if (this.opts.autoRefresh !== false && needsRefresh(cmd) && this.app.store.getState().world?.solved) {
      const gen = this.generation;
      this.chain = this.runChain(gen);
    }
    return ack;
  }

  /** Abandons the local stroke without sending anything (Escape). */
  cancelStroke(): void {
    this.stroke.clear();
    this.store.setState({ phase: 'idle', preview: null });
  }

  /** Preview first (fast, coarse), then refined; either stops quietly if a newer stroke began. */
  private async runChain(gen: number): Promise<void> {
    const done: RefreshMode[] = [];
    const modes: RefreshMode[] = this.opts.autoRefine === false ? ['preview'] : ['preview', 'refined'];
    for (const mode of modes) {
      if (gen !== this.generation) break;
      this.store.setState({ phase: mode === 'preview' ? 'previewing' : 'refining' });
      let status: JobStatus;
      try {
        status = await this.app.refreshAndWait(mode);
      } catch (e) {
        // `job_running` (a solve is running) or `not_solved`: nothing to refresh, not an error the user must see
        const code = (e as { code?: string }).code;
        if (code !== 'job_running' && code !== 'not_solved') this.store.setState({ phase: 'error', error: (e as Error).message, lastChain: done });
        else this.store.setState({ phase: 'idle', lastChain: done });
        return;
      }
      if (status !== 'completed') break;
      done.push(mode);
    }
    if (gen === this.generation) this.store.setState({ phase: 'idle', lastChain: done });
    else this.store.setState({ lastChain: done });
  }

  // ---- non-brush edits ----------------------------------------------------------------------------------------------------------
  setRealism(field: OutputField | 'all', value: number): Promise<Ack> {
    return this.app.command({ type: 'set_realism', field, value });
  }

  /** Undo / redo one history step (strokes, locks, realism and parameters share one stack). */
  async undo(): Promise<Ack> { this.generation++; return this.app.command({ type: 'undo' }); }
  async redo(): Promise<Ack> { this.generation++; return this.app.command({ type: 'redo' }); }
}
