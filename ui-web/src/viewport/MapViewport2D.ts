// MapViewport2D: draws the mesh cells as a flat map on a <canvas>. The projection (equirectangular, cube cross, plane) is picked from the mesh kind.
// Costs: the pixel -> cell table is built once per mesh; a layer switch only repaints from that table. Pan and zoom never repaint, they only redraw
// the cached bitmap with a transform.
import type { FieldData, LayerDecl, MeshData } from 'wb-client';
import { buildPixelCells, makeProjection, type Projection } from './projections';
import { paintPixels } from './raster';
import { HOME_CAMERA, isOrbit, type AnyCamera, type CameraState, type LayerContext, type PickEvent, type Viewport } from './types';

export interface MapViewportOptions {
  /** Raster width in pixels of the whole map (default 720, the same default as `wb-snap`). */
  rasterWidth?: number;
  minZoom?: number;
  maxZoom?: number;
}

type Listener<T> = (e: T) => void;

export class MapViewport2D implements Viewport {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private bitmap: HTMLCanvasElement;
  private bctx: CanvasRenderingContext2D;
  private mesh: MeshData | null = null;
  private proj: Projection | null = null;
  private pixelCells: Int32Array | null = null;
  private camera: CameraState = { ...HOME_CAMERA };
  private ro: ResizeObserver | null = null;
  private pickL = new Set<Listener<PickEvent>>();
  private hoverL = new Set<Listener<PickEvent>>();
  private camL = new Set<Listener<AnyCamera>>();
  private cleanup: (() => void)[] = [];
  private cssW = 1;
  private cssH = 1;
  private dpr = 1;
  private raf = 0;
  private lastHover: number | null | undefined;
  readonly rasterWidth: number;
  private minZoom: number;
  private maxZoom: number;

  constructor(private host: HTMLElement, opts: MapViewportOptions = {}) {
    this.rasterWidth = opts.rasterWidth ?? 720;
    this.minZoom = opts.minZoom ?? 1;
    this.maxZoom = opts.maxZoom ?? 24;
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'map-canvas';
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute('role', 'img');
    this.canvas.setAttribute('aria-label', 'World map');
    this.canvas.style.cssText = 'display:block;width:100%;height:100%;touch-action:none;cursor:grab';
    this.host.appendChild(this.canvas);
    this.ctx = this.canvas.getContext('2d')!;
    this.bitmap = document.createElement('canvas');
    this.bctx = this.bitmap.getContext('2d')!;
    this.bindInput();
    if (typeof ResizeObserver !== 'undefined') {
      this.ro = new ResizeObserver(() => this.resize());
      this.ro.observe(this.host);
    }
    this.resize();
  }

  // ---- Viewport ---------------------------------------------------------------------------------------------------------
  setMesh(mesh: MeshData): void {
    if (this.mesh?.meshId === mesh.meshId) return;
    this.mesh = mesh;
    this.proj = makeProjection(mesh, this.rasterWidth);
    this.pixelCells = buildPixelCells(mesh, this.proj);
    this.bitmap.width = this.proj.width;
    this.bitmap.height = this.proj.height;
    this.bctx.clearRect(0, 0, this.bitmap.width, this.bitmap.height);
    this.camera = { ...HOME_CAMERA };
    this.requestDraw();
  }

  setLayers(layers: LayerDecl[], fields: Record<string, FieldData>, ctx: LayerContext = { biomes: [] }): void {
    if (!this.proj || !this.pixelCells) return;
    const { width, height } = this.proj;
    this.bctx.globalCompositeOperation = 'source-over';
    this.bctx.clearRect(0, 0, width, height);
    const scratch = document.createElement('canvas');
    scratch.width = width;
    scratch.height = height;
    const sctx = scratch.getContext('2d')!;
    for (const decl of layers) {
      const f = fields[decl.id];
      if (!f) continue;
      const img = sctx.createImageData(width, height);
      paintPixels(this.pixelCells, f, decl, ctx, img.data);
      sctx.putImageData(img, 0, 0);
      this.bctx.drawImage(scratch, 0, 0); // layers composite bottom to top, each with its own opacity
    }
    this.requestDraw();
  }

  setCamera(camera: AnyCamera): void {
    if (isOrbit(camera)) return; // an orbit camera belongs to a 3D viewport
    this.camera = this.clampCamera(camera);
    this.requestDraw();
    this.camL.forEach((l) => l(this.camera));
  }

  getCamera(): CameraState { return { ...this.camera }; }

  pick(x: number, y: number): number | null {
    if (!this.proj || !this.pixelCells) return null;
    const m = this.screenToMap(x, y);
    if (m.u < 0 || m.u >= 1 || m.v < 0 || m.v >= 1) return null;
    const px = Math.floor(m.u * this.proj.width), py = Math.floor(m.v * this.proj.height);
    const c = this.pixelCells[py * this.proj.width + px];
    return c >= 0 ? c : null;
  }

  resize(): void {
    const r = this.host.getBoundingClientRect();
    this.cssW = Math.max(1, Math.round(r.width));
    this.cssH = Math.max(1, Math.round(r.height));
    this.dpr = (typeof window !== 'undefined' && window.devicePixelRatio) || 1;
    this.canvas.width = Math.round(this.cssW * this.dpr);
    this.canvas.height = Math.round(this.cssH * this.dpr);
    this.camera = this.clampCamera(this.camera);
    this.requestDraw();
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    this.ro?.disconnect();
    this.cleanup.forEach((c) => c());
    this.canvas.remove();
    this.pickL.clear(); this.hoverL.clear(); this.camL.clear();
  }

  onPick(l: Listener<PickEvent>) { this.pickL.add(l); return () => this.pickL.delete(l); }
  onHover(l: Listener<PickEvent>) { this.hoverL.add(l); return () => this.hoverL.delete(l); }
  onCameraChange(l: Listener<AnyCamera>) { this.camL.add(l); return () => this.camL.delete(l); }

  // ---- geometry ---------------------------------------------------------------------------------------------------------
  /** Size of the map on screen at zoom 1: as large as fits, aspect ratio kept. */
  private fit(): { w: number; h: number } {
    const aspect = this.proj ? this.proj.width / this.proj.height : 2;
    let w = this.cssW, h = w / aspect;
    if (h > this.cssH) { h = this.cssH; w = h * aspect; }
    return { w, h };
  }

  private screenToMap(x: number, y: number): { u: number; v: number } {
    const { w, h } = this.fit();
    const s = this.camera.zoom;
    return { u: (x - this.cssW / 2) / (w * s) + this.camera.x, v: (y - this.cssH / 2) / (h * s) + this.camera.y };
  }

  private clampCamera(c: CameraState): CameraState {
    const zoom = Math.min(this.maxZoom, Math.max(this.minZoom, c.zoom));
    const { w, h } = this.fit();
    // keep the map covering the view: the centre may move only while an edge of the map is still on screen
    const halfU = Math.min(0.5, this.cssW / (2 * w * zoom));
    const halfV = Math.min(0.5, this.cssH / (2 * h * zoom));
    const clamp = (v: number, half: number) => (half >= 0.5 ? 0.5 : Math.min(1 - half, Math.max(half, v)));
    return { zoom, x: clamp(c.x, halfU), y: clamp(c.y, halfV) };
  }

  private requestDraw(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => { this.raf = 0; this.draw(); });
  }

  private draw(): void {
    const { ctx } = this;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    if (!this.proj) return;
    const { w, h } = this.fit();
    const s = this.camera.zoom;
    const dw = w * s, dh = h * s;
    const dx = this.cssW / 2 - this.camera.x * dw, dy = this.cssH / 2 - this.camera.y * dh;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.imageSmoothingEnabled = false; // cells are flat-coloured: show them crisp when zoomed
    ctx.drawImage(this.bitmap, dx, dy, dw, dh);
  }

  // ---- input ------------------------------------------------------------------------------------------------------------
  private bindInput(): void {
    const c = this.canvas;
    const local = (e: { clientX: number; clientY: number }) => {
      const r = c.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    let drag: { x: number; y: number; cx: number; cy: number; moved: boolean; id: number } | null = null;

    const down = (e: PointerEvent) => {
      const p = local(e);
      drag = { x: p.x, y: p.y, cx: this.camera.x, cy: this.camera.y, moved: false, id: e.pointerId };
      c.setPointerCapture?.(e.pointerId);
      c.style.cursor = 'grabbing';
    };
    const move = (e: PointerEvent) => {
      const p = local(e);
      if (drag && drag.id === e.pointerId) {
        const dxp = p.x - drag.x, dyp = p.y - drag.y;
        if (Math.abs(dxp) + Math.abs(dyp) > 3) drag.moved = true;
        if (drag.moved) {
          const { w, h } = this.fit();
          const s = this.camera.zoom;
          this.setCamera({ zoom: s, x: drag.cx - dxp / (w * s), y: drag.cy - dyp / (h * s) });
        }
      }
      const cell = this.pick(p.x, p.y);
      if (cell !== this.lastHover || drag?.moved === false) {
        this.lastHover = cell;
        this.hoverL.forEach((l) => l({ cell, ...p }));
      }
    };
    const up = (e: PointerEvent) => {
      const p = local(e);
      if (drag && !drag.moved) this.pickL.forEach((l) => l({ cell: this.pick(p.x, p.y), ...p }));
      drag = null;
      c.style.cursor = 'grab';
    };
    const leave = () => {
      if (this.lastHover !== null) { this.lastHover = null; this.hoverL.forEach((l) => l({ cell: null, x: -1, y: -1 })); }
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const p = local(e);
      const before = this.screenToMap(p.x, p.y);
      const zoom = Math.min(this.maxZoom, Math.max(this.minZoom, this.camera.zoom * Math.exp(-e.deltaY * 0.0015)));
      // keep the map point under the cursor fixed
      const { w, h } = this.fit();
      const x = before.u - (p.x - this.cssW / 2) / (w * zoom);
      const y = before.v - (p.y - this.cssH / 2) / (h * zoom);
      this.setCamera({ zoom, x, y });
    };
    const dbl = () => this.setCamera({ ...HOME_CAMERA });
    const key = (e: KeyboardEvent) => {
      const step = 0.1 / this.camera.zoom;
      const k: Record<string, () => void> = {
        ArrowLeft: () => this.setCamera({ ...this.camera, x: this.camera.x - step }),
        ArrowRight: () => this.setCamera({ ...this.camera, x: this.camera.x + step }),
        ArrowUp: () => this.setCamera({ ...this.camera, y: this.camera.y - step }),
        ArrowDown: () => this.setCamera({ ...this.camera, y: this.camera.y + step }),
        '+': () => this.setCamera({ ...this.camera, zoom: this.camera.zoom * 1.4 }),
        '=': () => this.setCamera({ ...this.camera, zoom: this.camera.zoom * 1.4 }),
        '-': () => this.setCamera({ ...this.camera, zoom: this.camera.zoom / 1.4 }),
        '0': () => this.setCamera({ ...HOME_CAMERA }),
      };
      const fn = k[e.key];
      if (fn) { e.preventDefault(); fn(); }
    };

    const on = <K extends keyof HTMLElementEventMap>(t: K, fn: (e: HTMLElementEventMap[K]) => void, opts?: AddEventListenerOptions) => {
      c.addEventListener(t, fn as EventListener, opts);
      this.cleanup.push(() => c.removeEventListener(t, fn as EventListener));
    };
    on('pointerdown', down);
    on('pointermove', move);
    on('pointerup', up);
    on('pointercancel', () => { drag = null; });
    on('pointerleave', leave);
    on('wheel', wheel, { passive: false });
    on('dblclick', dbl);
    on('keydown', key);
  }
}
