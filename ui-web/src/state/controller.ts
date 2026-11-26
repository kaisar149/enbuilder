// UI-only state and the controller that turns user intent into wb-client calls. No React in here, so it is testable under Node.
// UI state (selected layer, hover, picked cell) never goes to the core (UI_ARCHITECTURE section 1).
import { Store, WbApp, defaultLayer, layerChoices, layerDecl, selectField, type CellInfo, type FieldData, type LayerDecl, type WbState } from 'wb-client';

export interface UiState {
  /** Field shown on the map. */
  layer: string | null;
  hoverCell: number | null;
  pickedCell: number | null;
  cellInfo: CellInfo | null;
  cellError: string | null;
}

export const initialUi = (): UiState => ({ layer: null, hoverCell: null, pickedCell: null, cellInfo: null, cellError: null });

export class MapController {
  readonly ui = new Store<UiState>(initialUi());
  private unsub: () => void;

  constructor(readonly app: WbApp) {
    this.unsub = app.store.subscribe(() => this.sync());
    this.sync();
  }

  dispose(): void { this.unsub(); }

  /**
   * Keeps the selection valid: picks the first available layer when none is chosen, and falls back when the chosen field disappears
   * (a different preset or mesh clears the solved fields). Fetches the selected field when the cache is missing or older than the server's version.
   */
  private sync(): void {
    const s = this.app.store.getState();
    if (!s.schema || !s.world) return;
    const choices = layerChoices(s.schema, s.world);
    const current = this.ui.getState().layer;
    const stillOk = current && choices.some((c) => c.field === current && c.available);
    if (!stillOk) {
      const next = defaultLayer(choices);
      if (next !== current) this.ui.setState({ layer: next });
    }
    const layer = this.ui.getState().layer;
    if (layer) void this.app.ensureField(layer).catch(() => undefined);
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

  hover(cell: number | null): void {
    if (this.ui.getState().hoverCell !== cell) this.ui.setState({ hoverCell: cell });
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
  const f = selectField(app, ui.layer);
  if (!f) return null;
  const decl = layerDecl(app.schema, f);
  return { layers: [decl], fields: { [decl.id]: f } };
}
