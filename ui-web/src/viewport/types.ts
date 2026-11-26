// The Viewport interface (UI_ARCHITECTURE section 4). Every renderer (2D map, globe, cube, flat, sky islands) implements this and nothing else is
// allowed to touch pixels. Tools and brushes talk to a viewport only through `pick`.
import type { BiomeDef, FieldData, LayerDecl, MeshData } from 'wb-client';

/** Renderer-neutral camera. For the 2D map: `x`,`y` = map point at the screen centre in 0..1 map units, `zoom` = 1 shows the whole map. */
export interface CameraState {
  x: number;
  y: number;
  zoom: number;
}

/** Extra data the layer declarations point at (kept out of LayerDecl so declarations stay small and serialisable). */
export interface LayerContext {
  biomes: BiomeDef[];
}

export interface PickEvent {
  /** Cell index, or null when the pointer is outside the map. */
  cell: number | null;
  /** Pointer position in CSS pixels relative to the viewport element. */
  x: number;
  y: number;
}

export interface Viewport {
  setMesh(mesh: MeshData): void;
  /** Draws the layers bottom to top. `fields` is keyed by `LayerDecl.id`. */
  setLayers(layers: LayerDecl[], fields: Record<string, FieldData>, ctx?: LayerContext): void;
  setCamera(camera: CameraState): void;
  getCamera(): CameraState;
  /** Screen position (CSS px, relative to the viewport element) -> cell index. */
  pick(x: number, y: number): number | null;
  resize(): void;
  dispose(): void;
  onPick(listener: (e: PickEvent) => void): () => void;
  onHover(listener: (e: PickEvent) => void): () => void;
  onCameraChange(listener: (c: CameraState) => void): () => void;
}

export const HOME_CAMERA: CameraState = { x: 0.5, y: 0.5, zoom: 1 };
