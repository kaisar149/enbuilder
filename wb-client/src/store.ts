// A tiny observable store plus `WbApp`, the controller that keeps the store in sync with the server (commands -> queries -> events).
// UI-only state (camera, open panels, theme, which layer is shown) is NOT here; it lives in the UI package.
import { WbClient, type ConnectOptions } from './client';
import type { Ack, Command, FieldData, JobStatus, MeshData, Schema, VariationField, VariationsInfo, WbEvent, WorldInfo } from './types';

export class Store<S> {
  private listeners = new Set<() => void>();
  constructor(private state: S) {}
  getState = (): S => this.state;
  setState(patch: Partial<S> | ((s: S) => Partial<S>)): void {
    const p = typeof patch === 'function' ? patch(this.state) : patch;
    this.state = { ...this.state, ...p };
    this.listeners.forEach((l) => l());
  }
  /** Returns an unsubscribe function. Compatible with React's useSyncExternalStore. */
  subscribe = (l: () => void): (() => void) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };
}

export interface JobState {
  id: number;
  fraction: number;
  stage: string;
  status: 'running' | JobStatus;
  /** What the job is doing (known for jobs this client started). */
  kind?: 'solve' | 'variations';
}

export interface WbState {
  connection: 'connecting' | 'open' | 'closed';
  schema: Schema | null;
  world: WorldInfo | null;
  mesh: MeshData | null;
  /** Fetched fields, key = `name#layer`. */
  fields: Record<string, FieldData>;
  jobs: { current: JobState | null };
  lastError: string | null;
  /** Part 9c: seeds of the last variation batch (null = none, or the mesh changed). */
  variations: VariationsInfo | null;
  /** Fetched variation fields, key = `index:name`. */
  variationFields: Record<string, VariationField>;
}

export const fieldKey = (name: string, layer = 0) => `${name}#${layer}`;

export function initialState(): WbState {
  return { connection: 'connecting', schema: null, world: null, mesh: null, fields: {}, jobs: { current: null }, lastError: null, variations: null, variationFields: {} };
}

export class WbApp {
  readonly store = new Store<WbState>(initialState());
  /** Fields somebody asked for; they are refetched when the server says they changed. */
  private watched = new Set<string>();
  private inflight = new Map<string, Promise<FieldData>>();
  private variationJobs = new Set<number>();
  private doneJobs = new Map<number, JobStatus>();
  private jobWaiters = new Map<number, ((s: JobStatus) => void)[]>();
  private unsub: (() => void)[] = [];
  private eventChain: Promise<void> = Promise.resolve();

  constructor(readonly client: WbClient) {
    this.store.setState({ connection: 'open' });
    this.unsub.push(client.onEvent((e) => this.enqueue(e)));
    this.unsub.push(client.onClose(() => this.store.setState({ connection: 'closed' })));
  }

  static async connect(url: string, opts?: ConnectOptions): Promise<WbApp> {
    const app = new WbApp(await WbClient.connect(url, opts));
    await app.init();
    return app;
  }

  /** Loads schema, world info and mesh. */
  async init(): Promise<void> {
    const [schema, world, mesh] = await Promise.all([this.client.getSchema(), this.client.getWorldInfo(), this.client.getMesh()]);
    this.store.setState({ schema, world, mesh });
  }

  dispose(): void {
    this.unsub.forEach((u) => u());
    this.client.close();
  }

  // ---- commands ---------------------------------------------------------------------------------------------------------
  /** Sends a command; the store is refreshed by the events the server pushes, so no polling is needed. */
  async command(cmd: Command, opts: { quiet?: boolean } = {}): Promise<Ack> {
    try {
      const ack = await this.client.command(cmd);
      if ((cmd.type === 'solve' || cmd.type === 'variations') && ack.job != null) {
        const kind = cmd.type === 'solve' ? 'solve' : 'variations';
        if (kind === 'variations') this.variationJobs.add(ack.job);
        this.store.setState({ jobs: { current: { id: ack.job, fraction: 0, stage: 'starting', status: 'running', kind } } });
      }
      // undo/redo/load_preset/set_param change the parameters but may not produce an event when nothing was solved
      if (cmd.type !== 'solve' && cmd.type !== 'cancel' && cmd.type !== 'variations') await this.refreshWorld();
      return ack;
    } catch (e) {
      if (!opts.quiet) this.store.setState({ lastError: (e as Error).message });
      throw e;
    }
  }

  /** Starts the variations job (`count` alternative seeds, coarse) and resolves with its final status; the seeds are then in `store.variations`. */
  async generateVariations(count = 6, coarse = true): Promise<JobStatus> {
    const ack = await this.command({ type: 'variations', count, coarse });
    return this.waitJob(ack.job as number);
  }

  /** One kept field of one variation (cached; thumbnails call this lazily). */
  async ensureVariation(index: number, name: string): Promise<VariationField> {
    const key = `${index}:${name}`;
    const cached = this.store.getState().variationFields[key];
    if (cached) return cached;
    const f = await this.client.getVariation(index, name);
    this.store.setState((s) => ({ variationFields: { ...s.variationFields, [key]: f } }));
    return f;
  }

  /** Starts a solve and resolves with the job's final status. */
  async solveAndWait(): Promise<JobStatus> {
    const ack = await this.command({ type: 'solve' });
    return this.waitJob(ack.job as number);
  }

  waitJob(id: number): Promise<JobStatus> {
    const done = this.doneJobs.get(id);
    if (done) return Promise.resolve(done);
    return new Promise((res) => this.jobWaiters.set(id, [...(this.jobWaiters.get(id) ?? []), res]));
  }

  async refreshWorld(): Promise<WorldInfo> {
    const world = await this.client.getWorldInfo();
    // the core drops a variation batch when the mesh changes
    const v = this.store.getState().variations;
    this.store.setState(v && v.mesh_id !== world.domain.mesh_id ? { world, variations: null, variationFields: {} } : { world });
    return world;
  }

  // ---- fields -----------------------------------------------------------------------------------------------------------
  /** True when the server has this field right now (elevation always; the rest after a solve). */
  isAvailable(name: string): boolean { return !!this.store.getState().world?.fields.some((f) => f.name === name); }

  /**
   * Returns the field, fetching it with ONE `get_field` query only when the cached copy is missing or older than the server's version.
   * This is the only thing a layer switch does.
   */
  ensureField(name: string, layer = 0): Promise<FieldData> {
    const key = fieldKey(name, layer);
    this.watched.add(key);
    const cached = this.store.getState().fields[key];
    const info = this.store.getState().world?.fields.find((f) => f.name === name);
    if (cached && info && cached.version === info.version && cached.stale === info.stale) return Promise.resolve(cached);
    const running = this.inflight.get(key);
    if (running) return running;
    const p = this.client
      .getField(name, layer)
      .then((f) => {
        this.store.setState((s) => ({ fields: { ...s.fields, [key]: f } }));
        return f;
      })
      .finally(() => this.inflight.delete(key));
    this.inflight.set(key, p);
    return p;
  }

  unwatch(name: string, layer = 0): void { this.watched.delete(fieldKey(name, layer)); }

  // ---- events -----------------------------------------------------------------------------------------------------------
  private enqueue(e: WbEvent): void {
    // handled strictly in order, so a world_loaded is processed before the field_changed that follows it
    this.eventChain = this.eventChain.then(() => this.onEvent(e)).catch((err) => this.store.setState({ lastError: String(err?.message ?? err) }));
  }

  /** Resolves when every event received so far has been processed (for tests and for "wait until the UI is up to date"). */
  idle(): Promise<void> { return this.eventChain; }

  private async refetchWatched(only?: string[]): Promise<void> {
    const world = this.store.getState().world;
    const jobs: Promise<unknown>[] = [];
    for (const key of this.watched) {
      const [name, layer] = key.split('#');
      if (only && !only.includes(name)) continue;
      if (!world?.fields.some((f) => f.name === name)) continue;
      jobs.push(this.ensureField(name, Number(layer)).catch(() => undefined));
    }
    await Promise.all(jobs);
  }

  private async onEvent(e: WbEvent): Promise<void> {
    switch (e.type) {
      case 'job_progress':
        this.store.setState({ jobs: { current: { id: e.job, fraction: e.fraction, stage: e.stage, status: 'running', kind: this.variationJobs.has(e.job) ? 'variations' : 'solve' } } });
        break;
      case 'job_done': {
        this.doneJobs.set(e.job, e.status);
        const cur = this.store.getState().jobs.current;
        this.store.setState({ jobs: { current: { id: e.job, fraction: e.status === 'completed' ? 1 : cur?.fraction ?? 0, stage: cur?.stage ?? '', status: e.status, kind: this.variationJobs.has(e.job) ? 'variations' : 'solve' } } });
        if (this.variationJobs.delete(e.job) && e.status === 'completed') {
          const variations = await this.client.getVariations();
          this.store.setState({ variations, variationFields: {} });
        }
        (this.jobWaiters.get(e.job) ?? []).forEach((r) => r(e.status));
        this.jobWaiters.delete(e.job);
        break;
      }
      case 'world_loaded': {
        const mesh = await this.client.getMesh();
        this.store.setState({ mesh, fields: {}, variations: null, variationFields: {} });
        await this.refreshWorld();
        await this.refetchWatched();
        break;
      }
      case 'field_changed':
        await this.refreshWorld();
        await this.refetchWatched(e.names);
        break;
      case 'error':
        this.store.setState({ lastError: e.message });
        break;
    }
  }
}
