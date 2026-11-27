// BaseViewport3D: everything a 3D viewport does that is NOT drawing: DOM canvas, pointer/wheel input, orbit camera state and animation, picking,
// colour per cell, relief, selection, frame statistics. Renderers (Three.js in MeshViewport3D, nothing in StubViewport) extend it and only
// implement the `on*` hooks below, so replacing the renderer touches one file. No Three.js import here.
import type { FieldData, LayerDecl, MeshData } from 'wb-client';
import { cellColors, MISSING } from './raster';
import {
  buildCellGeometry, cellRelief, cellTop, displaceCorners, shapeKind, type CellGeometry, type ShapeKind,
} from './geometry3d';
import { cameraFrame, clampCamera, focusOn, homeCamera, lerpCamera, orbitBy, panBy, rayFromPixel, zoomBy } from './orbit';
import { pickRay } from './pick3d';
import { arrowTarget, buildOverlay3D, type Overlay3D } from './overlay3d';
import {
  DEFAULT_VIEW3D, isOrbit, type AnyCamera, type LayerContext, type OrbitCamera, type OverlayItem, type OverlayOptions, type PickEvent, type View3DOptions, type Viewport3D,
} from './types';

type Listener<T> = (e: T) => void;

export abstract class BaseViewport3D implements Viewport3D {
  protected canvas: HTMLCanvasElement;
  protected mesh: MeshData | null = null;
  protected geom: CellGeometry | null = null;
  protected kind: ShapeKind = 'sphere';
  protected camera: OrbitCamera = homeCamera('sphere');
  protected opts: View3DOptions = { ...DEFAULT_VIEW3D };
  protected elevation: Float32Array | null = null;
  protected relief: Float32Array | null = null;
  protected reliefMax = 0;
  /** RGB per cell (the very array the 2D map paints from), composited bottom to top. */
  protected cellRGB: Uint8ClampedArray | null = null;
  protected selection: number | null = null;
  protected cssW = 1;
  protected cssH = 1;
  protected dpr = 1;
  private ro: ResizeObserver | null = null;
  private raf = 0;
  private anim = 0;
  private pickL = new Set<Listener<PickEvent>>();
  private hoverL = new Set<Listener<PickEvent>>();
  private camL = new Set<Listener<AnyCamera>>();
  private cleanup: (() => void)[] = [];
  private lastHit = -1;
  private lastHover: number | null | undefined;
  private stats = { frames: 0, sum: 0, max: 0 };
  private disposed = false;

  constructor(protected host: HTMLElement) {
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'map-canvas map-canvas-3d';
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute('role', 'img');
    this.canvas.setAttribute('aria-label', '3D world view');
    this.canvas.style.cssText = 'display:block;width:100%;height:100%;touch-action:none;cursor:grab';
    host.appendChild(this.canvas);
    this.bindInput();
    if (typeof ResizeObserver !== 'undefined') { this.ro = new ResizeObserver(() => this.resize()); this.ro.observe(host); }
  }

  // ---- renderer hooks (the only things a renderer implements) ----
  /** New mesh: (re)build static geometry from `this.geom` (indices, corner counts). */
  protected abstract onGeometry(): void;
  /** Corner positions changed (new relief or new mesh): re-upload `displaced`. */
  protected abstract onPositions(displaced: Float32Array): void;
  /** Cell colours changed: `this.cellRGB` holds the new ones. */
  protected abstract onColors(): void;
  /** Lighting / view options changed. */
  protected abstract onView(): void;
  protected abstract onSelection(corners: Float32Array | null): void;
  /** Part 9e: overlay geometry changed (new items, new relief, or arrows re-thinned for the camera distance). `list` is bottom to top. */
  protected abstract onOverlays(list: Overlay3D[]): void;
  protected abstract onResize(w: number, h: number, dpr: number): void;
  /** Draw one frame with `this.camera`. */
  protected abstract draw(): void;
  protected abstract onDispose(): void;

  // ---- Viewport ----
  protected displaced = new Float32Array(0);

  setMesh(mesh: MeshData): void {
    this.mesh = mesh;
    this.geom = buildCellGeometry(mesh);
    this.kind = shapeKind(mesh);
    this.camera = homeCamera(this.kind);
    this.lastHit = -1;
    this.displaced = new Float32Array(3 * this.geom.nCorners);
    this.elevation = null; this.cellRGB = null; this.relief = null; this.reliefMax = 0; this.selection = null;
    this.overlayItems = []; this.arrowTargetUsed = 0; // overlays belong to the previous mesh; the host sets them again
    this.onGeometry();
    this.rebuildRelief(false);
    this.emitCamera();
    this.requestRender();
  }

  setLayers(layers: LayerDecl[], fields: Record<string, FieldData>, ctx: LayerContext = { biomes: [] }): void {
    if (!this.geom) return;
    const n = this.geom.nCells;
    let out: Uint8ClampedArray | null = null;
    for (const decl of layers) {
      const f = fields[decl.id];
      if (!f || f.values.length !== n) continue;
      const c = cellColors(decl, f.values, ctx);
      if (!out) {
        out = c;
        // a faded (or hidden) base layer shows the background colour through it, like the 2D map does
        if (decl.opacity < 1) for (let i = 0; i < out.length; i++) out[i] = MISSING[i % 3] * (1 - decl.opacity) + c[i] * decl.opacity;
        continue;
      }
      const a = decl.opacity;
      for (let i = 0; i < out.length; i++) out[i] = out[i] * (1 - a) + c[i] * a;
    }
    if (!out) return;
    this.cellRGB = out;
    this.onColors();
    this.requestRender();
  }

  protected overlayItems: OverlayItem[] = [];
  protected overlayOpts: OverlayOptions = { animate: false };
  private arrowTargetUsed = 0;

  /** Streaks are a 2D feature (the 3D view draws the arrows); `opts.animate` is accepted and ignored here. */
  setOverlays(items: OverlayItem[], opts: OverlayOptions = { animate: false }): void {
    this.overlayItems = items;
    this.overlayOpts = opts;
    this.rebuildOverlays();
    this.requestRender();
  }

  /** Builds the neutral geometry of every item (overlay3d.ts) and hands it to the renderer. */
  protected rebuildOverlays(): void {
    if (!this.mesh || !this.geom) return;
    this.arrowTargetUsed = arrowTarget(this.camera.distance);
    const b = { mesh: this.mesh, geom: this.geom, relief: this.relief, arrows: this.arrowTargetUsed };
    this.onOverlays(this.overlayItems.map((it) => buildOverlay3D(it, b)));
  }

  /** Re-thins the arrows when the camera has moved far enough to change the number that fit. */
  private rethinArrows(): void {
    if (!this.overlayItems.some((i) => i.kind === 'vector')) return;
    const t = arrowTarget(this.camera.distance);
    if (this.arrowTargetUsed && Math.abs(Math.log(t / this.arrowTargetUsed)) > 0.5) this.rebuildOverlays();
  }

  setElevation(values: Float32Array | null): void {
    this.elevation = values;
    this.rebuildRelief(true);
  }

  setView(opts: View3DOptions): void {
    const reliefChanged = opts.exaggeration !== this.opts.exaggeration || opts.unitsPerMetre !== this.opts.unitsPerMetre;
    this.opts = { ...opts };
    if (reliefChanged) this.rebuildRelief(true);
    this.onView();
    this.requestRender();
  }

  setSelection(cell: number | null): void {
    this.selection = cell;
    this.pushSelection();
    this.requestRender();
  }

  setCamera(camera: AnyCamera): void {
    if (!isOrbit(camera)) return; // a map camera means nothing here
    this.camera = clampCamera(camera, this.kind);
    this.emitCamera();
    this.requestRender();
  }

  getCamera(): OrbitCamera { return { ...this.camera, target: [...this.camera.target] }; }

  resetView(): void { this.animateTo(homeCamera(this.kind)); }

  focusCell(cell: number): void {
    if (!this.mesh || !this.geom || cell < 0 || cell >= this.geom.nCells) return;
    const p = cellTop(this.geom, this.mesh, cell, this.relief);
    this.animateTo(focusOn(this.camera, this.kind, p, this.geom.radius));
  }

  pick(x: number, y: number): number | null {
    if (!this.mesh || !this.geom || x < 0 || y < 0 || x >= this.cssW || y >= this.cssH) return null;
    const f = cameraFrame(this.camera, this.geom.radius);
    const { origin, dir } = rayFromPixel(f, x, y, this.cssW, this.cssH);
    const hit = pickRay(this.mesh, this.geom, this.relief, this.reliefMax, origin, dir, this.lastHit);
    if (!hit) return null;
    this.lastHit = hit.cell;
    return hit.cell;
  }

  frameStats(): { frames: number; meanMs: number; maxMs: number } | null {
    const s = this.stats;
    if (!s.frames) return null;
    const r = { frames: s.frames, meanMs: s.sum / s.frames, maxMs: s.max };
    this.stats = { frames: 0, sum: 0, max: 0 };
    return r;
  }

  resize(): void {
    this.cssW = Math.max(1, this.host.clientWidth || 1);
    this.cssH = Math.max(1, this.host.clientHeight || 1);
    this.dpr = Math.min(2, (typeof window !== 'undefined' && window.devicePixelRatio) || 1);
    this.onResize(this.cssW, this.cssH, this.dpr);
    this.requestRender();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    cancelAnimationFrame(this.raf); cancelAnimationFrame(this.anim);
    this.cleanup.forEach((f) => f());
    this.ro?.disconnect();
    this.pickL.clear(); this.hoverL.clear(); this.camL.clear();
    this.onDispose();
    this.canvas.remove();
  }

  onPick(l: Listener<PickEvent>) { this.pickL.add(l); return () => this.pickL.delete(l); }
  onHover(l: Listener<PickEvent>) { this.hoverL.add(l); return () => this.hoverL.delete(l); }
  onCameraChange(l: Listener<AnyCamera>) { this.camL.add(l); return () => this.camL.delete(l); }

  // ---- internals ----
  protected requestRender(): void {
    if (this.raf || this.disposed) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      if (this.disposed || !this.geom) return;
      const t0 = performance.now();
      this.draw();
      const dt = performance.now() - t0;
      this.stats.frames++; this.stats.sum += dt; this.stats.max = Math.max(this.stats.max, dt);
    });
  }

  private rebuildRelief(render: boolean): void {
    if (!this.geom) return;
    this.relief = cellRelief(this.elevation, this.opts.exaggeration, this.opts.unitsPerMetre);
    this.reliefMax = 0;
    if (this.relief) for (let i = 0; i < this.relief.length; i++) if (this.relief[i] > this.reliefMax) this.reliefMax = this.relief[i];
    displaceCorners(this.geom, this.relief, this.displaced);
    this.onPositions(this.displaced);
    this.pushSelection();
    this.rebuildOverlays();
    if (render) this.requestRender();
  }

  private pushSelection(): void {
    const g = this.geom;
    if (!g || this.selection === null || this.selection < 0 || this.selection >= g.nCells) { this.onSelection(null); return; }
    const a = g.cornerStart[this.selection], b = g.cornerStart[this.selection + 1];
    const lift = 0.004 * g.radius;
    const out = new Float32Array(3 * (b - a));
    for (let j = a; j < b; j++) {
      const x = this.displaced[3 * j], y = this.displaced[3 * j + 1], z = this.displaced[3 * j + 2];
      let dx = 0, dy = 0, dz = 1;
      if (g.kind !== 'plane') { const l = Math.hypot(x, y, z) || 1; dx = x / l; dy = y / l; dz = z / l; }
      out[3 * (j - a)] = x + dx * lift; out[3 * (j - a) + 1] = y + dy * lift; out[3 * (j - a) + 2] = z + dz * lift;
    }
    this.onSelection(out);
  }

  private emitCamera(): void { const c = this.getCamera(); this.camL.forEach((l) => l(c)); this.rethinArrows(); }

  private animateTo(target: OrbitCamera): void {
    cancelAnimationFrame(this.anim);
    const from = this.getCamera();
    const t0 = performance.now(), dur = 450;
    const step = () => {
      const t = Math.min(1, (performance.now() - t0) / dur);
      this.camera = clampCamera(lerpCamera(from, target, t), this.kind);
      this.emitCamera();
      this.requestRender();
      if (t < 1 && !this.disposed) this.anim = requestAnimationFrame(step);
    };
    this.anim = requestAnimationFrame(step);
  }

  private bindInput(): void {
    const c = this.canvas;
    const on = <K extends keyof HTMLElementEventMap>(t: K, f: (e: HTMLElementEventMap[K]) => void, o?: AddEventListenerOptions) => {
      c.addEventListener(t, f as EventListener, o);
      this.cleanup.push(() => c.removeEventListener(t, f as EventListener, o));
    };
    const rel = (e: MouseEvent) => { const r = c.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    let drag: { x: number; y: number; id: number; moved: boolean; pan: boolean } | null = null;
    let hoverRaf = 0, hoverPt = { x: 0, y: 0 };

    on('pointerdown', (e) => {
      const p = rel(e);
      cancelAnimationFrame(this.anim);
      drag = { x: p.x, y: p.y, id: e.pointerId, moved: false, pan: e.button === 2 || e.button === 1 || e.shiftKey };
      c.setPointerCapture?.(e.pointerId);
      c.style.cursor = 'grabbing';
    });
    on('pointermove', (e) => {
      const p = rel(e);
      if (drag && drag.id === e.pointerId) {
        const dx = p.x - drag.x, dy = p.y - drag.y;
        if (!drag.moved && Math.hypot(dx, dy) < 3) return;
        drag.moved = true;
        drag.x = p.x; drag.y = p.y;
        this.camera = drag.pan && this.geom
          ? panBy(this.camera, this.kind, this.geom.radius, dx, dy, this.cssH)
          : orbitBy(this.camera, this.kind, dx, dy);
        this.emitCamera();
        this.requestRender();
        return;
      }
      hoverPt = p;
      if (hoverRaf) return; // at most one pick per frame
      hoverRaf = requestAnimationFrame(() => {
        hoverRaf = 0;
        const cell = this.pick(hoverPt.x, hoverPt.y);
        if (cell !== this.lastHover) { this.lastHover = cell; this.hoverL.forEach((l) => l({ cell, ...hoverPt })); }
      });
    });
    on('pointerup', (e) => {
      if (!drag || drag.id !== e.pointerId) return;
      const was = drag; drag = null;
      c.style.cursor = 'grab';
      if (!was.moved && !was.pan) { const p = rel(e); const cell = this.pick(p.x, p.y); this.pickL.forEach((l) => l({ cell, ...p })); }
    });
    on('pointercancel', () => { drag = null; c.style.cursor = 'grab'; });
    on('pointerleave', () => {
      if (this.lastHover !== null && this.lastHover !== undefined) { this.lastHover = null; this.hoverL.forEach((l) => l({ cell: null, x: -1, y: -1 })); }
    });
    on('wheel', (e) => {
      e.preventDefault();
      const d = e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY;
      this.camera = zoomBy(this.camera, this.kind, d);
      this.emitCamera();
      this.requestRender();
    }, { passive: false });
    on('dblclick', (e) => { const p = rel(e); const cell = this.pick(p.x, p.y); if (cell !== null) this.focusCell(cell); });
    on('contextmenu', (e) => e.preventDefault());
    on('keydown', (e) => {
      const step = e.shiftKey ? 40 : 15;
      const k: Record<string, [number, number]> = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
      if (k[e.key]) { e.preventDefault(); this.camera = orbitBy(this.camera, this.kind, k[e.key][0], k[e.key][1]); this.emitCamera(); this.requestRender(); }
      else if (e.key === '+' || e.key === '=') { this.camera = zoomBy(this.camera, this.kind, -120); this.emitCamera(); this.requestRender(); }
      else if (e.key === '-') { this.camera = zoomBy(this.camera, this.kind, 120); this.emitCamera(); this.requestRender(); }
      else if (e.key === 'Home') this.resetView();
    });
  }
}
