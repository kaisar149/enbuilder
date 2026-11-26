// Panel registry: layout is DATA (UI_ARCHITECTURE section 3). Moving, hiding or adding a panel is an edit to this list (or to a saved override),
// never a change to App.tsx. `component` names an entry in panels/components.tsx.
export type Dock = 'left' | 'right' | 'bottom' | 'hidden';

export interface PanelDef {
  id: string;
  title: string;
  component: string;
  default_dock: Dock;
}

export const PANEL_REGISTRY: PanelDef[] = [
  { id: 'solve', title: 'Solve', component: 'SolvePanel', default_dock: 'left' },
  { id: 'layers', title: 'Map layer', component: 'LayersPanel', default_dock: 'left' },
  { id: 'world', title: 'World', component: 'WorldPanel', default_dock: 'left' },
  { id: 'cell', title: 'Selected cell', component: 'CellPanel', default_dock: 'right' },
  { id: 'legend', title: 'Legend', component: 'LegendPanel', default_dock: 'bottom' },
];

export type Layout = Record<Exclude<Dock, 'hidden'>, PanelDef[]>;

/** `overrides` maps panel id -> dock (e.g. loaded from localStorage); unknown ids are ignored. */
export function resolveLayout(defs: PanelDef[] = PANEL_REGISTRY, overrides: Record<string, Dock> = {}): Layout {
  const out: Layout = { left: [], right: [], bottom: [] };
  for (const d of defs) {
    const dock = overrides[d.id] ?? d.default_dock;
    if (dock !== 'hidden') out[dock].push(d);
  }
  return out;
}
