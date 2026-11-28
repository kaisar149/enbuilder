// WebSocket client: typed commands and queries, request/response matching, pushed events, binary frames. Framework-free.
import { decodeDiff, decodeFrame, decodeLocks, decodeMesh, decodeOutline, decodePaint, decodeRivers, f32Array } from './frames';
import {
  API_VERSION, WIRE_VERSION,
  type Ack, type ApiErrorBody, type CellInfo, type Command, type FieldData, type FieldHeader, type Hello, type MeshData, type MeshHeader,
  type DiffData, type DiffSummary, type Explanation, type VersionsData, type FieldViewName, type LocksData, type OutlineData, type OutputField, type PaintData, type PaintLayerId, type PaintOp, type Query, type RefreshMode, type RiversData, type Schema, type VariationField, type VariationsInfo, type WbEvent, type WorldInfo,
} from './types';

export class ApiClientError extends Error {
  constructor(public code: string, message: string) {
    super(`${code}: ${message}`);
    this.name = 'ApiClientError';
  }
}

export interface SentRecord {
  id: number;
  kind: 'command' | 'query';
  type: string;
  body: unknown;
}

export interface ConnectOptions {
  /** Defaults to the global `WebSocket` (browsers, Node 22+). */
  WebSocketImpl?: typeof WebSocket;
  /** Per-request timeout in ms (default 60000). */
  timeoutMs?: number;
}

interface Pending {
  resolve: (v: { envelope: any; frame?: ReturnType<typeof decodeFrame> }) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class WbClient {
  readonly hello: Hello;
  /** Every request this client sent (most recent 1000). Lets tests and devtools prove what the UI did. */
  readonly sent: SentRecord[] = [];
  private ws: WebSocket;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private eventListeners = new Set<(e: WbEvent) => void>();
  private closeListeners = new Set<() => void>();
  private timeoutMs: number;
  private closed = false;

  private constructor(ws: WebSocket, hello: Hello, timeoutMs: number) {
    this.ws = ws;
    this.hello = hello;
    this.timeoutMs = timeoutMs;
    ws.onmessage = (m) => this.onMessage(m.data);
    ws.onclose = () => this.handleClose();
    ws.onerror = () => undefined;
  }

  /** Opens the socket, waits for the server's hello and checks the versions. */
  static connect(url: string, opts: ConnectOptions = {}): Promise<WbClient> {
    const Impl = opts.WebSocketImpl ?? (globalThis as any).WebSocket;
    if (!Impl) return Promise.reject(new Error('no WebSocket implementation available'));
    return new Promise((resolve, reject) => {
      const ws: WebSocket = new Impl(url);
      ws.binaryType = 'arraybuffer';
      let done = false;
      const fail = (e: Error) => {
        if (!done) {
          done = true;
          try { ws.close(); } catch { /* ignore */ }
          reject(e);
        }
      };
      ws.onerror = () => fail(new ApiClientError('connect_failed', `cannot connect to ${url}`));
      ws.onclose = () => fail(new ApiClientError('connect_failed', `connection to ${url} closed before the hello`));
      ws.onmessage = (m) => {
        if (done || typeof m.data !== 'string') return;
        const hello = JSON.parse(m.data)?.hello as Hello | undefined;
        if (!hello) return fail(new ApiClientError('bad_hello', 'the first message was not a hello'));
        if (hello.api_version !== API_VERSION || hello.wire_version !== WIRE_VERSION) {
          return fail(new ApiClientError('version_mismatch', `server speaks api ${hello.api_version} / wire ${hello.wire_version}, this client needs ${API_VERSION} / ${WIRE_VERSION}`));
        }
        done = true;
        resolve(new WbClient(ws, hello, opts.timeoutMs ?? 60000));
      };
    });
  }

  get isOpen(): boolean { return !this.closed; }

  onEvent(cb: (e: WbEvent) => void): () => void {
    this.eventListeners.add(cb);
    return () => this.eventListeners.delete(cb);
  }

  onClose(cb: () => void): () => void {
    this.closeListeners.add(cb);
    return () => this.closeListeners.delete(cb);
  }

  private handleClose(): void {
    this.closed = true;
    for (const [, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(new ApiClientError('disconnected', 'the connection was closed'));
    }
    this.pending.clear();
    this.closeListeners.forEach((l) => l());
  }

  close(): void {
    this.closed = true;
    try { this.ws.close(); } catch { /* ignore */ }
  }

  private onMessage(data: unknown): void {
    if (typeof data === 'string') {
      const v = JSON.parse(data);
      if (v.event) this.eventListeners.forEach((l) => l(v.event as WbEvent));
      else this.settle(v);
    } else if (data instanceof ArrayBuffer) {
      const frame = decodeFrame(data);
      this.settle(frame.header, frame);
    }
  }

  private settle(envelope: any, frame?: ReturnType<typeof decodeFrame>): void {
    const p = this.pending.get(envelope.id);
    if (!p) return;
    this.pending.delete(envelope.id);
    clearTimeout(p.timer);
    if (envelope.ok) p.resolve({ envelope, frame });
    else {
      const e = envelope.error as ApiErrorBody;
      p.reject(new ApiClientError(e.code, e.message));
    }
  }

  private request(kind: 'command' | 'query', body: Command | Query): Promise<{ envelope: any; frame?: ReturnType<typeof decodeFrame> }> {
    if (this.closed) return Promise.reject(new ApiClientError('disconnected', 'the connection is closed'));
    const id = this.nextId++;
    this.sent.push({ id, kind, type: body.type, body });
    if (this.sent.length > 1000) this.sent.splice(0, this.sent.length - 1000);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new ApiClientError('timeout', `no answer to ${kind} ${body.type} within ${this.timeoutMs} ms`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.ws.send(JSON.stringify({ id, [kind]: body }));
    });
  }

  // ---- commands ------------------------------------------------------------------------------------------------------------
  async command(cmd: Command): Promise<Ack> { return (await this.request('command', cmd)).envelope.result as Ack; }
  loadPreset(name: string) { return this.command({ type: 'load_preset', name }); }
  setParam(name: string, value: number) { return this.command({ type: 'set_param', name, value }); }
  solve() { return this.command({ type: 'solve' }); }
  cancel() { return this.command({ type: 'cancel' }); }
  undo() { return this.command({ type: 'undo' }); }
  redo() { return this.command({ type: 'redo' }); }
  variations(count: number, coarse = true) { return this.command({ type: 'variations', count, coarse }); }

  // Part 9f
  paintStroke(layer: PaintLayerId, op: PaintOp, cells: ArrayLike<number>, weights: ArrayLike<number>, opts: { value?: number | null; strength?: number } = {}) {
    return this.command({ type: 'paint_stroke', layer, op, cells: Array.from(cells), weights: Array.from(weights), value: opts.value ?? null, strength: opts.strength ?? 1 });
  }
  setLock(field: OutputField, cells: ArrayLike<number>, locked: boolean) { return this.command({ type: 'set_lock', field, cells: Array.from(cells), locked }); }
  setRealism(field: OutputField | 'all', value: number) { return this.command({ type: 'set_realism', field, value }); }
  refresh(mode: RefreshMode) { return this.command({ type: 'refresh', mode }); }

  // ---- queries -------------------------------------------------------------------------------------------------------------
  async queryJson<T>(q: Query): Promise<T> { return (await this.request('query', q)).envelope.result as T; }
  getSchema() { return this.queryJson<Schema>({ type: 'get_schema' }); }
  getWorldInfo() { return this.queryJson<WorldInfo>({ type: 'get_world_info' }); }
  getCell(cell: number) { return this.queryJson<CellInfo>({ type: 'get_cell', cell }); }
  getVariations() { return this.queryJson<VariationsInfo>({ type: 'get_variations' }); }

  /** Part 9e: river polylines as cell ids. Rejects with `not_solved` before a solve. */
  async getRivers(): Promise<RiversData> {
    const { envelope, frame } = await this.request('query', { type: 'get_rivers' });
    if (!frame) throw new ApiClientError('bad_frame', 'get_rivers answered without a binary frame');
    return decodeRivers(envelope.result, frame);
  }

  /** Part 9e: outline of `field > threshold` as (inside, outside) cell pairs. */
  async getOutline(field: string, threshold = 0.5): Promise<OutlineData> {
    const { envelope, frame } = await this.request('query', { type: 'get_outline', field, threshold });
    if (!frame) throw new ApiClientError('bad_frame', 'get_outline answered without a binary frame');
    return decodeOutline(envelope.result, frame);
  }

  async getMesh(): Promise<MeshData> {
    const { envelope, frame } = await this.request('query', { type: 'get_mesh' });
    if (!frame) throw new ApiClientError('bad_frame', 'get_mesh answered without a binary frame');
    return decodeMesh(envelope.result as MeshHeader, frame);
  }

  async getField(name: string, layer = 0): Promise<FieldData> {
    return this.decodeField(await this.request('query', { type: 'get_field', name, layer }), 'get_field');
  }

  /** One kept field of one variation (same binary layout as `get_field`; `seed` and `variation` are in `extra`). */
  async getVariation(index: number, name: string): Promise<VariationField> {
    const r = await this.request('query', { type: 'get_variation', index, name });
    const f = this.decodeField(r, 'get_variation');
    const h = r.envelope.result as { seed: number; variation: number };
    return { ...f, seed: h.seed, variation: h.variation };
  }

  /** Part 9f: an output field as `blended` (default), `computed` (physics only) or `painted` (realism 0). */
  async getFieldView(name: string, view: FieldViewName = 'blended', layer = 0): Promise<FieldData> {
    return this.decodeField(await this.request('query', { type: 'get_field_view', name, layer, view }), 'get_field_view');
  }

  async getPaint(layer: PaintLayerId): Promise<PaintData> {
    const { envelope, frame } = await this.request('query', { type: 'get_paint', layer });
    if (!frame) throw new ApiClientError('bad_frame', 'get_paint answered without a binary frame');
    return decodePaint(envelope.result, frame);
  }

  async getLocks(): Promise<LocksData> {
    const { envelope, frame } = await this.request('query', { type: 'get_locks' });
    if (!frame) throw new ApiClientError('bad_frame', 'get_locks answered without a binary frame');
    return decodeLocks(envelope.result, frame);
  }

  // ---- Part 9g -------------------------------------------------------------------------------------------------------------
  getVersions(): Promise<VersionsData> { return this.queryJson<VersionsData>({ type: 'get_versions' }); }
  getDiffSummary(from: number, to: number, minRel = 0.01): Promise<DiffSummary> { return this.queryJson<DiffSummary>({ type: 'get_diff_summary', from_version: from, to_version: to, min_rel: minRel }); }
  async getDiff(field: string, from: number, to: number, minRel = 0.01): Promise<DiffData> {
    const { envelope, frame } = await this.request('query', { type: 'get_diff', field, from_version: from, to_version: to, min_rel: minRel });
    if (!frame) throw new ApiClientError('bad_frame', 'get_diff answered without a binary frame');
    return decodeDiff(envelope.result, frame);
  }
  /** Why did `field` change at `cell`? Without versions: the latest stored version against the one before it. */
  explain(cell: number, field: string, from?: number, to?: number, minRel = 0.01): Promise<Explanation> {
    return this.queryJson<Explanation>({ type: 'explain', cell, field, from_version: from, to_version: to, min_rel: minRel });
  }

  private decodeField({ envelope, frame }: { envelope: any; frame?: ReturnType<typeof decodeFrame> }, what: string): FieldData {
    if (!frame) throw new ApiClientError('bad_frame', `${what} answered without a binary frame`);
    const h = envelope.result as FieldHeader;
    const { mesh_id, version, stale, layers, min, max, count, layer: l, dtype: _d, ...meta } = h;
    return { name: h.name, layer: l, layers, meshId: mesh_id, version, stale, min, max, meta: meta as FieldData['meta'], values: f32Array(frame, 0, count) };
  }
}
