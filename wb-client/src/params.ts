// ParamController: framework-free logic behind the generated parameter panel (Part 9c).
//  - edits are kept as local DRAFTS, so a slider can follow the pointer without any request;
//  - a draft is committed with ONE `set_param` after the user pauses (debounce) or on `flush`;
//  - the core is the only judge of validity: a rejected value is NOT clamped, it stays as the draft and the core's message is kept per parameter;
//  - Solve is never sent from here per edit; `solve()` first flushes pending edits so the solve sees them.
import { ApiClientError } from './client';
import { debounce, realTimers, type Debounced, type Timers } from './debounce';
import { randomSeed } from './selectors';
import { Store, type WbApp } from './store';
import type { ParamDef } from './types';

export interface ParamUiState {
  /** Values the user typed or dragged that the core has not accepted yet. */
  drafts: Record<string, number>;
  /** Core (or local) validation message per parameter. */
  errors: Record<string, string>;
  /** Parameters with a request in flight. */
  sending: Record<string, boolean>;
  /** Preset the user last picked (UI state; used when the world's preset is "custom"). */
  basePreset: string | null;
}

export interface ParamControllerOptions {
  debounceMs?: number;
  timers?: Timers;
  rng?: () => number;
}

export class ParamController {
  readonly ui = new Store<ParamUiState>({ drafts: {}, errors: {}, sending: {}, basePreset: null });
  private debouncers = new Map<string, Debounced<[]>>();
  private queue: Promise<unknown> = Promise.resolve();
  private readonly ms: number;
  private readonly timers: Timers;
  private readonly rng: () => number;

  constructor(readonly app: WbApp, opts: ParamControllerOptions = {}) {
    this.ms = opts.debounceMs ?? 250;
    this.timers = opts.timers ?? realTimers;
    this.rng = opts.rng ?? Math.random;
  }

  private def(name: string): ParamDef | undefined { return this.app.store.getState().schema?.params.find((p) => p.name === name); }

  /** Value to show: the draft if there is one, else the world's value, else the default. */
  value(name: string): number | undefined {
    const d = this.ui.getState().drafts[name];
    if (d !== undefined) return d;
    return this.app.store.getState().world?.params[name] ?? this.def(name)?.default;
  }

  private patch(f: (s: ParamUiState) => Partial<ParamUiState>): void { this.ui.setState(f); }
  private setError(name: string, msg: string | null): void {
    this.patch((s) => {
      const errors = { ...s.errors };
      if (msg === null) delete errors[name];
      else errors[name] = msg;
      return { errors };
    });
  }
  private dropDraft(name: string): void {
    this.patch((s) => {
      const drafts = { ...s.drafts };
      delete drafts[name];
      return { drafts };
    });
  }

  /** Called on every slider move or keystroke. No request is sent until the user pauses. */
  edit(name: string, value: number): void {
    this.patch((s) => ({ drafts: { ...s.drafts, [name]: value } }));
    this.setError(name, null);
    if (!Number.isFinite(value)) {
      this.cancelTimer(name);
      this.setError(name, `'${name}' needs a number`);
      return;
    }
    let d = this.debouncers.get(name);
    if (!d) {
      d = debounce(() => void this.commit(name), this.ms, this.timers);
      this.debouncers.set(name, d);
    }
    d();
  }

  private cancelTimer(name: string): void { this.debouncers.get(name)?.cancel(); }

  /** Sends the draft of `name` now (one `set_param`). Resolves true when the core accepted it. */
  commit(name: string): Promise<boolean> {
    this.cancelTimer(name);
    const run = async (): Promise<boolean> => {
      const draft = this.ui.getState().drafts[name];
      if (draft === undefined || !Number.isFinite(draft)) return true;
      if (this.app.store.getState().world?.params[name] === draft) {
        this.dropDraft(name);
        return true;
      }
      this.patch((s) => ({ sending: { ...s.sending, [name]: true } }));
      try {
        await this.app.command({ type: 'set_param', name, value: draft }, { quiet: true });
        // keep the draft if the user moved the slider again while the request was in flight
        if (this.ui.getState().drafts[name] === draft) this.dropDraft(name);
        return true;
      } catch (e) {
        this.setError(name, e instanceof ApiClientError ? e.message.replace(/^[a-z_]+: /, '') : String((e as Error)?.message ?? e));
        return false;
      } finally {
        this.patch((s) => {
          const sending = { ...s.sending };
          delete sending[name];
          return { sending };
        });
      }
    };
    const p = this.queue.then(run, run);
    this.queue = p.catch(() => undefined);
    return p;
  }

  /** Commits every pending draft that has not been rejected (in the order edited). */
  async flush(): Promise<void> {
    const names = Object.keys(this.ui.getState().drafts).filter((n) => !this.ui.getState().errors[n]);
    for (const n of names) await this.commit(n);
    await this.queue;
  }

  /** Resets one parameter to the schema default (clears its draft and error). */
  async reset(name: string): Promise<void> {
    const def = this.def(name);
    if (!def) return;
    this.cancelTimer(name);
    this.dropDraft(name);
    this.setError(name, null);
    if (this.app.store.getState().world?.params[name] === def.default) return;
    this.edit(name, def.default);
    await this.commit(name);
  }

  /** Resets every parameter of a group to its default (one undo step per parameter). */
  async resetGroup(group: string): Promise<void> {
    const defs = (this.app.store.getState().schema?.params ?? []).filter((p) => p.group === group);
    for (const d of defs) await this.reset(d.name);
  }

  /** Drops all drafts and messages (after undo, redo or a preset, which replace the values wholesale). */
  discard(): void {
    for (const d of this.debouncers.values()) d.cancel();
    this.patch(() => ({ drafts: {}, errors: {}, sending: {} }));
  }

  async applyPreset(name: string): Promise<void> {
    await this.queue;
    this.discard();
    await this.app.command({ type: 'load_preset', name });
    this.patch(() => ({ basePreset: name }));
  }

  async undo(): Promise<void> {
    await this.flush();
    this.discard();
    await this.app.command({ type: 'undo' });
  }

  async redo(): Promise<void> {
    this.discard();
    await this.app.command({ type: 'redo' });
  }

  async randomSeed(): Promise<number> {
    const v = randomSeed(this.app.store.getState().schema, this.rng);
    this.edit('seed', v);
    await this.commit('seed');
    return v;
  }

  /** Flushes edits, then starts a solve. */
  async solve(): Promise<void> {
    await this.flush();
    await this.app.command({ type: 'solve' });
  }

  /** Opening a variation = set its seed and solve (same mesh, same parameters; the thumbnail is exactly what you get). */
  async openVariation(index: number): Promise<number> {
    const info = this.app.store.getState().variations;
    const item = info?.items.find((i) => i.index === index);
    if (!item) throw new Error(`no variation ${index}`);
    await this.flush();
    this.discard();
    await this.app.command({ type: 'set_param', name: 'seed', value: item.seed });
    await this.app.command({ type: 'solve' });
    return item.seed;
  }

  dispose(): void { this.discard(); }
}
