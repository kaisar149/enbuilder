// Frame-rate benchmark: spins the camera for a fixed time with one draw per animation frame and reports the real interval between frames.
// This is what to record on a real GPU (the stub and jsdom cannot produce a meaningful number).
import { homeCamera } from './orbit';
import type { ShapeKind } from './geometry3d';
import { isOrbit, type Viewport } from './types';

export interface BenchResult { frames: number; seconds: number; meanFps: number; p95Ms: number; worstMs: number }

export function runBenchmark(vp: Viewport, kind: ShapeKind, ms = 3000): Promise<BenchResult> {
  return new Promise((resolve) => {
    const start = performance.now();
    const base = (() => { const c = vp.getCamera(); return isOrbit(c) ? c : homeCamera(kind); })();
    const dts: number[] = [];
    let last = start;
    const step = (t: number) => {
      const el = t - start;
      if (dts.length || t > last) dts.push(t - last);
      last = t;
      if (el >= ms) {
        vp.setCamera(base);
        const sorted = [...dts.slice(1)].sort((a, b) => a - b);
        const sum = sorted.reduce((s, x) => s + x, 0);
        resolve({ frames: sorted.length, seconds: el / 1000, meanFps: sorted.length ? (1000 * sorted.length) / sum : 0, p95Ms: sorted[Math.floor(sorted.length * 0.95)] ?? 0, worstMs: sorted[sorted.length - 1] ?? 0 });
        return;
      }
      vp.setCamera({ ...base, yaw: base.yaw + el * 0.0006, pitch: base.pitch + Math.sin(el * 0.0012) * 0.25 });
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
}
