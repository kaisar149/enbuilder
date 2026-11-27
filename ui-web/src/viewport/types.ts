// The Viewport interface (UI_ARCHITECTURE section 4). Every renderer (2D map, globe, cube, flat, sky islands) implements this and nothing else is
// allowed to touch pixels. Tools and brushes talk to a viewport only through `pick`.
import type { BiomeDef, FieldData, LayerDecl, MeshData, OutlineData, RiversData } from 'wb-client';

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

/**
 * Camera of the 3D viewports (globe, cube, flat). Lengths are in units of the world's bounding radius R (sphere radius, cube half side,
 * plane half width), so the same state means the same view at every resolution. Z is up; `yaw` is measured from +X towards +Y, `pitch` from the
 * horizon towards +Z. The camera sits at `target * R + R * distance * (cos p cos y, cos p sin y, sin p)` and looks at `target * R`.
 */
export interface OrbitCamera {
  kind: 'orbit';
  yaw: number;
  pitch: number;
  distance: number;
  target: [number, number, number];
}

export type AnyCamera = CameraState | OrbitCamera;
export const isOrbit = (c: AnyCamera): c is OrbitCamera => (c as OrbitCamera).kind === 'orbit';

/** Options only 3D viewports understand. `exaggeration` 0 = flat surface; otherwise elevation x exaggeration is shown (1 = true scale). */
export interface View3DOptions {
  exaggeration: number;
  lighting: 'flat' | 'sun';
  /** With sun lighting: the night side is dark (true) or only softly shaded (false). */
  dayNight: boolean;
  /** Unit vector from the planet towards the star, in the planet frame. */
  sunDir: [number, number, number];
  /** Mesh length units per metre of elevation (so exaggeration 1 is true scale). */
  unitsPerMetre: number;
}

export const DEFAULT_VIEW3D: View3DOptions = { exaggeration: 20, lighting: 'flat', dayNight: false, sunDir: [1, 0, 0], unitsPerMetre: 1 };

/** Part 9e. Style shared by every overlay item. `color` is a resolved CSS colour (from the theme token `--overlay-<token>`). */
export interface OverlayStyle {
  id: string;
  label: string;
  color: string;
  opacity: number;
  blend: 'normal' | 'multiply' | 'screen';
}

/**
 * What a viewport draws on top of the scalar layers: DATA only (kinds, never field names). Each viewport resolves the cell ids and vector
 * components in its own space. Items come bottom to top.
 */
export type OverlayItem = OverlayStyle & (
  | { kind: 'vector'; /** x, y, z components per cell, in mesh space. */ components: [Float32Array, Float32Array, Float32Array]; animated: boolean }
  | { kind: 'lines'; rivers: RiversData }
  | { kind: 'mask'; mode: 'outline'; outline: OutlineData }
  | { kind: 'mask'; mode: 'fill'; values: Float32Array; threshold: number }
);

/** Options of the overlay drawing that are not data (UI toggles). */
export interface OverlayOptions {
  /** Animated streaks for the vector overlays that allow it. */
  animate: boolean;
}

export interface Viewport {
  /** Part 9e. Draws the overlays bottom to top above the scalar layers. Pass [] to clear. Never changes picking or the scalar layers. */
  setOverlays(items: OverlayItem[], opts?: OverlayOptions): void;
  setMesh(mesh: MeshData): void;
  /** Draws the layers bottom to top. `fields` is keyed by `LayerDecl.id`. */
  setLayers(layers: LayerDecl[], fields: Record<string, FieldData>, ctx?: LayerContext): void;
  /** A viewport ignores a camera of the other family (map camera vs orbit camera). */
  setCamera(camera: AnyCamera): void;
  getCamera(): AnyCamera;
  /** Screen position (CSS px, relative to the viewport element) -> cell index. */
  pick(x: number, y: number): number | null;
  resize(): void;
  dispose(): void;
  onPick(listener: (e: PickEvent) => void): () => void;
  onHover(listener: (e: PickEvent) => void): () => void;
  onCameraChange(listener: (c: AnyCamera) => void): () => void;
}

/** Extra surface of the 3D viewports. The 2D map does not implement it. */
export interface Viewport3D extends Viewport {
  /** Elevation field (metres) used for relief; null = no relief data. */
  setElevation(values: Float32Array | null): void;
  setView(opts: View3DOptions): void;
  /** Marks the selected cell (outline); null clears it. */
  setSelection(cell: number | null): void;
  /** Animated move of the camera so `cell` is centred. */
  focusCell(cell: number): void;
  resetView(): void;
  /** Mean frame time statistics since the last call (null when nothing was drawn). */
  frameStats(): { frames: number; meanMs: number; maxMs: number } | null;
}

export const HOME_CAMERA: CameraState = { x: 0.5, y: 0.5, zoom: 1 };
