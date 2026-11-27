// Part 9e: draws OverlayItems on the 2D map canvas. The viewport (MapViewport2D) owns the canvas, camera and pixel->cell table; this class owns the
// overlay geometry caches and the drawing. It handles item KINDS (vector, lines, mask), never field names.
import type { MeshData } from 'wb-client';
import { arrowLength, ringsForPixels, riverWidthPx, thinCells, vectorStats, type Components } from '../overlays/geometry';
import { arrows2D, flow2D, lines2D, segments2D, type Arrows2D, type Lines2D } from './overlay2d';
import { makeParticles, seededRandom, stepParticles, type Particles } from './particles';
import { makeForward, type Forward, type Projection } from './projections';
import type { OverlayItem, OverlayOptions } from './types';

export interface MapView2D {
  /** Screen position (CSS px) of the map's top-left corner, and its on-screen size. */
  dx: number; dy: number; dw: number; dh: number;
  cssW: number; cssH: number;
}

const COMPOSITE: Record<OverlayItem['blend'], GlobalCompositeOperation> = { normal: 'source-over', multiply: 'multiply', screen: 'screen' };
const MIN_ARROW_SPACING_PX = 24;
const PARTICLES = 1800;
const TRAIL = 7;

interface Cache<T> { src: unknown; value: T }

export class OverlayPainter2D {
  private items: OverlayItem[] = [];
  private opts: OverlayOptions = { animate: false };
  private mesh: MeshData | null = null;
  private proj: Projection | null = null;
  private pixelCells: Int32Array | null = null;
  private fwd: Forward | null = null;
  private aspect = 2;
  private segs = new Map<string, Cache<Float32Array>>();
  private lines = new Map<string, Cache<Lines2D>>();
  private stats = new Map<string, Cache<{ max: number; p95: number }>>();
  private arrows = new Map<string, Cache<Map<number, Arrows2D>>>();
  private flows = new Map<string, Cache<Float32Array>>();
  private fills = new Map<string, Cache<HTMLCanvasElement | null>>();
  private particles = new Map<string, Particles>();
  private rand = seededRandom(7);
  /** For tests and the status line: what the last draw did. */
  readonly lastDraw = { arrows: 0, segments: 0, polylines: 0, fills: 0, particles: 0, rings: 0 };

  setMesh(mesh: MeshData, proj: Projection, pixelCells: Int32Array): void {
    this.mesh = mesh; this.proj = proj; this.pixelCells = pixelCells;
    this.fwd = makeForward(mesh);
    this.aspect = proj.width / proj.height;
    for (const m of [this.segs, this.lines, this.stats, this.arrows, this.flows, this.fills] as Map<string, unknown>[]) m.clear();
    this.particles.clear();
  }

  setItems(items: OverlayItem[], opts: OverlayOptions = { animate: false }): void { this.items = items; this.opts = opts; }

  /** True while an animated overlay is on screen (the viewport then keeps redrawing). */
  animating(): boolean { return this.opts.animate && this.items.some((i) => i.kind === 'vector' && i.animated); }
  hasItems(): boolean { return this.items.length > 0; }

  private cached<T>(m: Map<string, Cache<T>>, id: string, src: unknown, make: () => T): T {
    const c = m.get(id);
    if (c && c.src === src) return c.value;
    const value = make();
    m.set(id, { src, value });
    return value;
  }

  /** Advances the streak particles by `dt` seconds. */
  tick(dt: number): void {
    if (!this.mesh || !this.proj || !this.fwd || !this.pixelCells) return;
    for (const it of this.items) {
      if (it.kind !== 'vector' || !it.animated || !this.opts.animate) continue;
      const flow = this.flowOf(it);
      let p = this.particles.get(it.id);
      if (!p) { p = makeParticles(PARTICLES, TRAIL, this.rand); this.particles.set(it.id, p); }
      const { width, height } = this.proj;
      const cells = this.pixelCells;
      const cellAt = (u: number, v: number) => (v < 0 || v >= 1 ? -1 : cells[Math.floor(v * height) * width + Math.floor(u * width)]);
      stepParticles(p, flow, cellAt, Math.min(dt, 0.1), { speed: 0.05, life: 2.5, aspect: this.aspect, wrap: this.proj.kind === 'equirectangular' }, this.rand);
    }
  }

  private flowOf(it: Extract<OverlayItem, { kind: 'vector' }>): Float32Array {
    const ref = this.statsOf(it).p95;
    return this.cached(this.flows, it.id, it.components[0], () => flow2D(this.mesh!, this.fwd!, this.aspect, it.components as Components, ref));
  }

  private statsOf(it: Extract<OverlayItem, { kind: 'vector' }>) {
    return this.cached(this.stats, it.id, it.components[0], () => vectorStats(it.components as Components));
  }

  draw(ctx: CanvasRenderingContext2D, view: MapView2D): void {
    const last = this.lastDraw;
    last.arrows = last.segments = last.polylines = last.fills = last.particles = 0;
    if (!this.mesh || !this.proj || !this.fwd || this.items.length === 0) return;
    const wrap = this.proj.kind === 'equirectangular';
    for (const it of this.items) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(Math.max(0, view.dx), Math.max(0, view.dy), Math.min(view.cssW, view.dx + view.dw) - Math.max(0, view.dx), Math.min(view.cssH, view.dy + view.dh) - Math.max(0, view.dy));
      ctx.clip();
      ctx.globalAlpha = it.opacity;
      ctx.globalCompositeOperation = COMPOSITE[it.blend];
      ctx.strokeStyle = it.color;
      ctx.fillStyle = it.color;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      if (it.kind === 'mask' && it.mode === 'fill') this.drawFill(ctx, it, view);
      else if (it.kind === 'mask') this.drawSegments(ctx, it, view, wrap);
      else if (it.kind === 'lines') this.drawLines(ctx, it, view, wrap);
      else this.drawVectors(ctx, it, view);
      ctx.restore();
    }
  }

  private drawSegments(ctx: CanvasRenderingContext2D, it: Extract<OverlayItem, { kind: 'mask'; mode: 'outline' }>, v: MapView2D, wrap: boolean): void {
    const seg = this.cached(this.segs, it.id, it.outline, () => segments2D(this.mesh!, this.fwd!, it.outline));
    ctx.lineWidth = 1.4;
    ctx.beginPath();
    for (const s of wrap ? [-1, 0, 1] : [0]) {
      for (let i = 0; i < seg.length; i += 4) {
        const x0 = v.dx + (seg[i] + s) * v.dw, y0 = v.dy + seg[i + 1] * v.dh, x1 = v.dx + (seg[i + 2] + s) * v.dw, y1 = v.dy + seg[i + 3] * v.dh;
        if ((x0 < 0 && x1 < 0) || (x0 > v.cssW && x1 > v.cssW) || (y0 < 0 && y1 < 0) || (y0 > v.cssH && y1 > v.cssH)) continue;
        ctx.moveTo(x0, y0); ctx.lineTo(x1, y1);
        this.lastDraw.segments++;
      }
    }
    ctx.stroke();
  }

  private drawLines(ctx: CanvasRenderingContext2D, it: Extract<OverlayItem, { kind: 'lines' }>, v: MapView2D, wrap: boolean): void {
    const L = this.cached(this.lines, it.id, it.rivers, () => lines2D(this.mesh!, this.fwd!, it.rivers));
    // four width classes: one path (one stroke call) per class
    const classes = 4;
    for (let k = 0; k < classes; k++) {
      const lo = k / classes, hi = (k + 1) / classes;
      ctx.lineWidth = riverWidthPx((lo + hi) / 2, 0.9, 4.5);
      ctx.beginPath();
      let any = false;
      for (const s of wrap ? [-1, 0, 1] : [0]) {
        for (let p = 0; p + 1 < L.offsets.length; p++) {
          for (let i = L.offsets[p]; i + 1 < L.offsets[p + 1]; i++) {
            const w = L.width[i + 1];
            if (!(w >= lo && (w < hi || k === classes - 1))) continue;
            const x0 = v.dx + (L.points[2 * i] + s) * v.dw, y0 = v.dy + L.points[2 * i + 1] * v.dh;
            const x1 = v.dx + (L.points[2 * i + 2] + s) * v.dw, y1 = v.dy + L.points[2 * i + 3] * v.dh;
            if ((x0 < 0 && x1 < 0) || (x0 > v.cssW && x1 > v.cssW) || (y0 < 0 && y1 < 0) || (y0 > v.cssH && y1 > v.cssH)) continue;
            ctx.moveTo(x0, y0); ctx.lineTo(x1, y1);
            any = true;
            this.lastDraw.polylines++;
          }
        }
      }
      if (any) ctx.stroke();
    }
  }

  private drawFill(ctx: CanvasRenderingContext2D, it: Extract<OverlayItem, { kind: 'mask'; mode: 'fill' }>, v: MapView2D): void {
    const bmp = this.cached(this.fills, `${it.id}|${it.threshold}|${it.color}`, it.values, () => this.fillBitmap(it));
    if (!bmp) return;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(bmp, v.dx, v.dy, v.dw, v.dh);
    this.lastDraw.fills++;
  }

  /** The mask as one coloured bitmap in map-raster resolution (built from the pixel -> cell table, so it costs one pass per data change). */
  private fillBitmap(it: Extract<OverlayItem, { kind: 'mask'; mode: 'fill' }>): HTMLCanvasElement | null {
    if (!this.proj || !this.pixelCells || typeof document === 'undefined') return null;
    const { width, height } = this.proj;
    const c = document.createElement('canvas');
    c.width = width; c.height = height;
    const g = c.getContext('2d');
    if (!g || typeof g.createImageData !== 'function') return null;
    const img = g.createImageData(width, height);
    for (let i = 0; i < this.pixelCells.length; i++) {
      const cell = this.pixelCells[i];
      if (cell >= 0 && it.values[cell] > it.threshold) img.data[4 * i + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    g.globalCompositeOperation = 'source-in';
    g.fillStyle = it.color;
    g.fillRect(0, 0, width, height);
    return c;
  }

  private drawVectors(ctx: CanvasRenderingContext2D, it: Extract<OverlayItem, { kind: 'vector' }>, v: MapView2D): void {
    const mesh = this.mesh!;
    if (this.opts.animate && it.animated) { this.drawStreaks(ctx, it, v); return; }
    const rings = ringsForPixels(mesh.nCells, v.dw, v.dh, MIN_ARROW_SPACING_PX);
    this.lastDraw.rings = rings;
    const byRing = this.cached(this.arrows, it.id, it.components[0], () => new Map<number, Arrows2D>());
    let a = byRing.get(rings);
    if (!a) { a = arrows2D(mesh, this.fwd!, this.aspect, it.components as Components, thinCells(mesh, rings)); byRing.set(rings, a); }
    const ref = this.statsOf(it).p95;
    ctx.lineWidth = 1.3;
    ctx.beginPath();
    for (let i = 0; i < a.cell.length; i++) {
      const x = v.dx + a.u[i] * v.dw, y = v.dy + a.v[i] * v.dh;
      if (x < -20 || y < -20 || x > v.cssW + 20 || y > v.cssH + 20) continue;
      const len = arrowLength(a.mag[i], ref, 7, 22);
      const dx = a.dx[i], dy = a.dy[i];
      const x0 = x - (dx * len) / 2, y0 = y - (dy * len) / 2, x1 = x + (dx * len) / 2, y1 = y + (dy * len) / 2;
      ctx.moveTo(x0, y0); ctx.lineTo(x1, y1);
      // head: two barbs 150 degrees back from the direction
      const h = Math.min(6, len * 0.4), c = Math.cos(2.62), s = Math.sin(2.62);
      ctx.moveTo(x1, y1); ctx.lineTo(x1 + h * (dx * c - dy * s), y1 + h * (dx * s + dy * c));
      ctx.moveTo(x1, y1); ctx.lineTo(x1 + h * (dx * c + dy * s), y1 + h * (-dx * s + dy * c));
      this.lastDraw.arrows++;
    }
    ctx.stroke();
  }

  private drawStreaks(ctx: CanvasRenderingContext2D, it: Extract<OverlayItem, { kind: 'vector' }>, v: MapView2D): void {
    let p = this.particles.get(it.id);
    if (!p) { p = makeParticles(PARTICLES, TRAIL, this.rand); this.particles.set(it.id, p); }
    ctx.lineWidth = 1.1;
    ctx.beginPath();
    for (let i = 0; i < p.n; i++) {
      const b = i * p.trail * 2;
      const x0 = v.dx + p.hist[b] * v.dw, y0 = v.dy + p.hist[b + 1] * v.dh;
      if (x0 < 0 || y0 < 0 || x0 > v.cssW || y0 > v.cssH) continue;
      ctx.moveTo(x0, y0);
      for (let k = 1; k < p.trail; k++) {
        const ux = p.hist[b + 2 * k], uy = p.hist[b + 2 * k + 1];
        if (Math.abs(ux - p.hist[b + 2 * k - 2]) > 0.5) break; // trail crossed the date line
        ctx.lineTo(v.dx + ux * v.dw, v.dy + uy * v.dh);
      }
      this.lastDraw.particles++;
    }
    ctx.stroke();
  }
}
