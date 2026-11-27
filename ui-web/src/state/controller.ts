// UI-only state and the controller that turns user intent into wb-client calls. No React in here, so it is testable under Node.
// UI state (selected layer, hover, picked cell) never goes to the core (UI_ARCHITECTURE section 1).
import {
  ParamController, Store, WbApp, defaultLayer, fieldKey, layerChoices, layerDecl, seasonVariant, seasonsAvailable, selectField,
  type CellInfo, type FieldData, type LayerDecl, type WbState,
} from 'wb-client';
import { RELIEF_FIELD, initialView3D, type View3DState } from '../viewport/view3d';
import { dataNeeds, overlayAvailable, buildOverlayItems } from '../layers/items';
import { emptyStack, isDrawn, stackFromPreset, type LayerStack } from '../layers/stack';
import type { OverlayItem } from '../viewport/types';
import type { BenchResult } from '../viewport/bench';

export interface UiState {
  /** Field shown on the map. */
  layer: string | null;
  hoverCell: number | null;
  pickedCell: number | null;
  cellInfo: CellInfo | null;
  cellError: string | null;
  /** Season layer shown on the map (0-based), or null for the yearly normals. UI-only: it never goes to the core. */
  season: number | null;
  /** Part 9d. Which viewport shows the world. UI-only: the layer list, selection and legend are shared by both. */
  view: '2d' | '3d';
  view3d: View3DState;
  /** Hover tooltip (3D): cell info from `get_cell`, position in CSS px inside the viewport. */
  hoverInfo: CellInfo | null;
  hoverPos: { x: number; y: number } | null;
  /** Renderer in use ('three' | 'stub') and its last frame statistics, for the status line. */
  renderer: string | null;
  frame: { fps: number; drawMs: number } | null;
  bench: BenchResult | 'running' | null;
  /** Counters the viewport host watches: a request to reset the camera / to run the benchmark. */
  resetNonce: number;
  benchNonce: number;
  /** Part 9e. Overlay stack (order, visibility, opacity, blend, solo). UI-only: it never goes to the core. */
  stack: LayerStack;
  /** Animated streaks for the vector overlays that allow them. */
  animate: boolean;
  /** Base layer a preset asked for that is not available yet (it is selected as soon as a solve provides it). */
  pendingBase: string | null;
  /** Second cell kept for comparison in the inspector. */
  pinnedCell: number | null;
  pinnedInfo: CellInfo | null;
}

export const initialUi = (): UiState => ({ layer: null, hoverCell: null, pickedCell: null, cellInfo: null, cellError: null, season: null, view: '2d', view3d: initialView3D(), hoverInfo: null, hoverPos: null, renderer: null, frame: null, bench: null, resetNonce: 0, benchNonce: 0, stack: emptyStack(null), animate: false, pendingBase: null, pinnedCell: null, pinnedInfo: null });

export class MapController {
  readonly ui = new Store<UiState>(initialUi());
  /** Parameter editing (drafts, debounce, validation messages, presets, undo, variations). Shared by all parameter panels. */
  readonly params: ParamController;
  private unsub: () => void;
  private shownSeasonKey: string | null = null;
  private stackReady = false;
  private overlayFields = new Set<string>();
  private tipVersion = '';

  constructor(readonly app: WbApp) {
    this.params = new ParamController(app);
    this.unsub = app.store.subscribe(() => this.sync());
    this.sync();
  }

  dispose(): void { this.unsub(); this.params.dispose(); clearTimeout(this.tipTimer); }

  /**
   * Keeps the selection valid: picks the first available layer when none is chosen, and falls back when the chosen field disappears
   * (a different preset or mesh clears the solved fields). Fetches the selected field when the cache is missing or older than the server's version.
   */
  private sync(): void {
    const s = this.app.store.getState();
    if (!s.schema || !s.world) return;
    const ver = `${s.world.domain.mesh_id}:${s.world.fields.map((f) => f.version).join(',')}`;
    if (ver !== this.tipVersion) {
      const first = this.tipVersion === '';
      this.tipVersion = ver; this.tipCache.clear();
      // the inspector shows what the core says NOW: after a re-solve or a new world, ask again for the picked and the pinned cell
      if (!first) void this.refreshCells();
    }
    if (!this.stackReady) {
      // the first stack is the Terrain preset (coastline and glaciers over relief); the user switches with one click
      this.stackReady = true;
      const first = s.schema.layer_presets?.[0]?.id;
      if (first) this.ui.setState({ stack: stackFromPreset(s.schema, first) });
    }
    const pb = this.ui.getState().pendingBase;
    if (pb && s.world.fields.some((f) => f.name === pb)) { this.ui.setState({ pendingBase: null }); void this.selectLayer(pb); }
    this.syncOverlays();
    const choices = layerChoices(s.schema, s.world);
    const current = this.ui.getState().layer;
    const stillOk = current && choices.some((c) => c.field === current && c.available);
    if (!stillOk) {
      const next = defaultLayer(choices);
      if (next !== current) this.ui.setState({ layer: next });
    }
    const layer = this.ui.getState().layer;
    if (layer) void this.app.ensureField(layer).catch(() => undefined);
    // the season layer of the shown field, when seasons are on and available
    const { season } = this.ui.getState();
    const variant = layer && season !== null && seasonsAvailable(s.schema, s.world, layer) ? seasonVariant(s.schema, layer) : null;
    const key = variant ? fieldKey(variant, season!) : null;
    if (this.shownSeasonKey && this.shownSeasonKey !== key) {
      const [n, l] = this.shownSeasonKey.split('#');
      this.app.unwatch(n, Number(l));
    }
    this.shownSeasonKey = key;
    if (variant) void this.app.ensureField(variant, season!).catch(() => undefined);
    // relief needs the height field, only while the 3D view is on
    if (this.ui.getState().view === '3d' && s.world.solved) void this.app.ensureField(RELIEF_FIELD).catch(() => undefined);
  }

  /** Fetches the data the drawn overlays need (and only that): fields once per version, rivers and outlines once per change. Never sends a command. */
  private syncOverlays(): void {
    const s = this.app.store.getState();
    if (!s.schema || !s.world) return;
    const needs = dataNeeds(this.ui.getState().stack, s.schema, (f) => this.app.isAvailable(f));
    const want = new Set<string>();
    for (const f of needs.fields) { want.add(fieldKey(f.name, f.layer)); void this.app.ensureField(f.name, f.layer).catch(() => undefined); }
    for (const key of this.overlayFields) {
      if (want.has(key)) continue;
      const [n, l] = key.split('#');
      if (this.ui.getState().layer !== n) this.app.unwatch(n, Number(l));
    }
    this.overlayFields = want;
    if (needs.rivers) void this.app.ensureRivers().catch(() => undefined);
    for (const o of needs.outlines) void this.app.ensureOutline(o.field, o.threshold).catch(() => undefined);
  }

  /** Part 9e: change the stack (reorder, visibility, opacity, blend, solo). A pure state change: the data stays as it is. */
  updateStack(f: (s: LayerStack) => LayerStack): void {
    this.ui.setState({ stack: f(this.ui.getState().stack) });
    this.syncOverlays();
  }

  /** One click: the preset's overlay stack and its base layer (selected now, or as soon as a solve makes it available). */
  applyPreset(id: string): void {
    const schema = this.app.store.getState().schema;
    const p = schema?.layer_presets?.find((x) => x.id === id);
    if (!schema || !p) return;
    this.ui.setState({ stack: stackFromPreset(schema, id), pendingBase: null });
    const choice = layerChoices(schema, this.app.store.getState().world).find((c) => c.field === p.base);
    if (choice?.available) void this.selectLayer(p.base);
    else this.ui.setState({ pendingBase: p.base });
    this.syncOverlays();
  }

  setAnimate(on: boolean): void { this.ui.setState({ animate: on }); }

  /** Overlays for the viewport, from the current stack and the fetched data. */
  overlayItems(resolveColor: (token: string) => string): OverlayItem[] {
    return buildOverlayItems(this.app.store.getState(), this.app.store.getState().schema, this.ui.getState().stack, resolveColor);
  }

  private async refreshCells(): Promise<void> {
    const { pickedCell, pinnedCell } = this.ui.getState();
    if (pickedCell !== null) void this.pick(pickedCell);
    if (pinnedCell !== null) void this.pinCell(pinnedCell);
  }

  /** Keeps the clicked cell as the second column of the inspector (one `get_cell` query, same as a click). */
  async pinCell(cell: number | null): Promise<void> {
    if (cell === null) { this.ui.setState({ pinnedCell: null, pinnedInfo: null }); return; }
    this.ui.setState({ pinnedCell: cell });
    try {
      const info = await this.app.client.getCell(cell);
      if (this.ui.getState().pinnedCell === cell) this.ui.setState({ pinnedInfo: info });
    } catch { this.ui.setState({ pinnedCell: null, pinnedInfo: null }); }
  }

  /** 2D / 3D switch. Nothing else changes: layer, season, picked cell and legend are the same store values both viewports read. */
  setView(view: '2d' | '3d'): void {
    if (this.ui.getState().view === view) return;
    this.ui.setState({ view, hoverInfo: null, hoverPos: null, frame: null });
    this.sync();
  }

  setView3d(patch: Partial<View3DState>): void { this.ui.setState({ view3d: { ...this.ui.getState().view3d, ...patch } }); }
  resetCamera(): void { this.ui.setState({ resetNonce: this.ui.getState().resetNonce + 1 }); }
  runBenchmark(): void { this.ui.setState({ benchNonce: this.ui.getState().benchNonce + 1, bench: 'running' }); }
  setBench(b: BenchResult | null): void { this.ui.setState({ bench: b }); }
  setRenderer(name: string | null): void { if (this.ui.getState().renderer !== name) this.ui.setState({ renderer: name }); }
  setFrame(f: { fps: number; drawMs: number } | null): void { this.ui.setState({ frame: f }); }

  /** Season slider: pick a stored season layer (`get_field(<twin>, layer=k)`), or null for the yearly normals. No command, no solve. */
  setSeason(season: number | null): void {
    const count = this.app.store.getState().schema?.seasons?.count ?? 0;
    const k = season === null || count === 0 ? null : Math.max(0, Math.min(count - 1, Math.round(season)));
    if (this.ui.getState().season === k) return;
    this.ui.setState({ season: k });
    this.sync();
  }

  /** A layer switch is ONE `get_field` query (or none when the cache is current). It never sends a command. */
  async selectLayer(field: string): Promise<void> {
    const s = this.app.store.getState();
    const choice = layerChoices(s.schema, s.world).find((c) => c.field === field);
    if (!choice || !choice.available) return;
    const prev = this.ui.getState().layer;
    if (prev && prev !== field) this.app.unwatch(prev);
    this.ui.setState({ layer: field });
    await this.app.ensureField(field);
  }

  private tipTimer: ReturnType<typeof setTimeout> | undefined;
  private tipCache = new Map<number, CellInfo>();

  /**
   * Pointer over the map. In 3D a tooltip follows the pointer: it needs one read-only `get_cell` query per hovered cell, so the query is
   * delayed (150 ms of rest) and cached per cell and world version, and a stale answer is dropped.
   */
  hover(cell: number | null, pos?: { x: number; y: number }): void {
    const u = this.ui.getState();
    if (u.hoverCell !== cell) this.ui.setState({ hoverCell: cell });
    clearTimeout(this.tipTimer);
    if (u.view !== '3d' || cell === null) { if (u.hoverInfo || u.hoverPos) this.ui.setState({ hoverInfo: null, hoverPos: null }); return; }
    this.ui.setState({ hoverPos: pos ?? u.hoverPos });
    const cached = this.tipCache.get(cell);
    if (cached) { this.ui.setState({ hoverInfo: cached }); return; }
    this.tipTimer = setTimeout(async () => {
      try {
        const info = await this.app.client.getCell(cell);
        if (this.tipCache.size > 400) this.tipCache.clear();
        this.tipCache.set(cell, info);
        if (this.ui.getState().hoverCell === cell) this.ui.setState({ hoverInfo: info });
      } catch { /* a tooltip is optional: ignore failures */ }
    }, 150);
  }

  /** Click on the map: read-only `get_cell` query for the inspector. */
  async pick(cell: number | null): Promise<void> {
    if (cell === null) { this.ui.setState({ pickedCell: null, cellInfo: null, cellError: null }); return; }
    this.ui.setState({ pickedCell: cell, cellError: null });
    try {
      const info = await this.app.client.getCell(cell);
      if (this.ui.getState().pickedCell === cell) this.ui.setState({ cellInfo: info });
    } catch (e) {
      this.ui.setState({ cellInfo: null, cellError: (e as Error).message });
    }
  }
}

/** What the viewport should draw right now: layer declarations plus the field data they refer to. Pure function of the two stores. */
export function viewLayers(app: WbState, ui: UiState): { layers: LayerDecl[]; fields: Record<string, FieldData> } | null {
  if (!app.schema || !ui.layer) return null;
  let f = selectField(app, ui.layer);
  if (!f) return null;
  if (ui.season !== null && seasonsAvailable(app.schema, app.world, ui.layer)) {
    const twin = selectField(app, seasonVariant(app.schema, ui.layer)!, ui.season);
    // until the season layer arrives the map keeps showing the previous values; the colour range and palette stay those of the base field
    if (twin) f = { ...twin, name: ui.layer, meta: f.meta };
  }
  // the base layer's visibility and opacity come from the layer stack; hidden = fully transparent (the viewport keeps its own background)
  const decl = layerDecl(app.schema, f, isDrawn(ui.stack, 'base') ? ui.stack.base.opacity : 0);
  return { layers: [decl], fields: { [decl.id]: f } };
}
