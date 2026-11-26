// Tiny trailing debounce with cancel and flush. Timer functions are injectable so tests do not need real time.
export interface Timers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}
export const realTimers: Timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) };

export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  /** Drops the pending call. */
  cancel(): void;
  /** Runs the pending call now (no-op when nothing is pending). */
  flush(): void;
  readonly pending: boolean;
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number, timers: Timers = realTimers): Debounced<A> {
  let handle: unknown = null;
  let last: A | null = null;
  const run = () => {
    handle = null;
    const a = last;
    last = null;
    if (a) fn(...a);
  };
  const d = ((...args: A) => {
    last = args;
    if (handle !== null) timers.clear(handle);
    handle = timers.set(run, ms);
  }) as Debounced<A>;
  d.cancel = () => {
    if (handle !== null) timers.clear(handle);
    handle = null;
    last = null;
  };
  d.flush = () => {
    if (handle !== null) {
      timers.clear(handle);
      run();
    }
  };
  Object.defineProperty(d, 'pending', { get: () => handle !== null });
  return d;
}
