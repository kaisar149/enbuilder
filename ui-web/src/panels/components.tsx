// Maps the registry's `component` names to React components. The only place that knows both.
import type { ComponentType } from 'react';
import type { WbApp } from 'wb-client';
import type { MapController } from '../state/controller';
import { CellPanel } from './CellPanel';
import { LayersPanel } from './LayersPanel';
import { LegendPanel } from './LegendPanel';
import { SolvePanel } from './SolvePanel';
import { HistoryPanel } from './HistoryPanel';
import { ParameterPanel } from './ParameterPanel';
import { PresetPanel } from './PresetPanel';
import { SeasonPanel } from './SeasonPanel';
import { VariationsPanel } from './VariationsPanel';
import { View3DPanel } from './View3DPanel';
import { WorldPanel } from './WorldPanel';

export interface PanelProps {
  app: WbApp;
  ctl: MapController;
}

export const PANEL_COMPONENTS: Record<string, ComponentType<PanelProps>> = {
  SolvePanel, LayersPanel, WorldPanel, CellPanel, LegendPanel, PresetPanel, ParameterPanel, VariationsPanel, SeasonPanel, HistoryPanel, View3DPanel,
};
