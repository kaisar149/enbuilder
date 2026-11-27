// Orbit camera maths (pure, no DOM): state transitions with per-shape limits, camera frame, rays and projection.
// Lengths are multiples of the world radius R (see OrbitCamera in types.ts), Z is up.
import type { OrbitCamera } from './types';
import type { ShapeKind } from './geometry3d';

type V3 = [number, number, number];
export const FOV_DEG = 40;
const TAU = Math.PI * 2;

export interface Limits { minDist: number; maxDist: number; minPitch: number; maxPitch: number; maxTarget: [number, number, number] }

export function limitsFor(kind: ShapeKind): Limits {
  switch (kind) {
    case 'plane': return { minDist: 0.12, maxDist: 6, minPitch: 0.1, maxPitch: 1.5, maxTarget: [1, 1, 0] };
    case 'cube': return { minDist: 1.15, maxDist: 10, minPitch: -1.5, maxPitch: 1.5, maxTarget: [0.7, 0.7, 0.7] };
    default: return { minDist: 1.08, maxDist: 8, minPitch: -1.5, maxPitch: 1.5, maxTarget: [0.4, 0.4, 0.4] };
  }
}

export function homeCamera(kind: ShapeKind): OrbitCamera {
  if (kind === 'plane') return { kind: 'orbit', yaw: -Math.PI / 2, pitch: 0.95, distance: 2.2, target: [0, 0, 0] };
  if (kind === 'cube') return { kind: 'orbit', yaw: 0.75, pitch: 0.55, distance: 4.5, target: [0, 0, 0] };
  return { kind: 'orbit', yaw: 0.3, pitch: 0.35, distance: 3.2, target: [0, 0, 0] };
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function clampCamera(c: OrbitCamera, kind: ShapeKind): OrbitCamera {
  const l = limitsFor(kind);
  let yaw = c.yaw % TAU;
  if (yaw > Math.PI) yaw -= TAU; else if (yaw < -Math.PI) yaw += TAU;
  return {
    kind: 'orbit', yaw, pitch: clamp(c.pitch, l.minPitch, l.maxPitch), distance: clamp(c.distance, l.minDist, l.maxDist),
    target: [clamp(c.target[0], -l.maxTarget[0], l.maxTarget[0]), clamp(c.target[1], -l.maxTarget[1], l.maxTarget[1]), clamp(c.target[2], -l.maxTarget[2], l.maxTarget[2])],
  };
}

/** Left-drag: turn the world. Slower when zoomed in. */
export function orbitBy(c: OrbitCamera, kind: ShapeKind, dxPx: number, dyPx: number): OrbitCamera {
  const k = 0.0055 * (kind === 'plane' ? 1 : clamp((c.distance - 1) * 0.6 + 0.12, 0.1, 1));
  return clampCamera({ ...c, yaw: c.yaw - dxPx * k, pitch: c.pitch + dyPx * k }, kind);
}

export function zoomBy(c: OrbitCamera, kind: ShapeKind, wheelDelta: number): OrbitCamera {
  return clampCamera({ ...c, distance: c.distance * Math.exp(wheelDelta * 0.0015) }, kind);
}

export interface Frame { pos: V3; fwd: V3; right: V3; up: V3; tanHalf: number }

export function cameraFrame(c: OrbitCamera, R: number): Frame {
  const cp = Math.cos(c.pitch), sp = Math.sin(c.pitch), cy = Math.cos(c.yaw), sy = Math.sin(c.yaw);
  const tgt: V3 = [c.target[0] * R, c.target[1] * R, c.target[2] * R];
  const off: V3 = [cp * cy, cp * sy, sp];
  const pos: V3 = [tgt[0] + off[0] * R * c.distance, tgt[1] + off[1] * R * c.distance, tgt[2] + off[2] * R * c.distance];
  const fwd: V3 = [-off[0], -off[1], -off[2]];
  // right = fwd x Z (normalised); up = right x fwd
  let right: V3 = [fwd[1], -fwd[0], 0];
  const rl = Math.hypot(right[0], right[1]);
  right = rl > 1e-9 ? [right[0] / rl, right[1] / rl, 0] : [1, 0, 0];
  const up: V3 = [right[1] * fwd[2] - right[2] * fwd[1], right[2] * fwd[0] - right[0] * fwd[2], right[0] * fwd[1] - right[1] * fwd[0]];
  return { pos, fwd, right, up, tanHalf: Math.tan((FOV_DEG * Math.PI) / 360) };
}

/** Near/far planes that bracket the world without wasting depth precision. */
export function clipPlanes(c: OrbitCamera, R: number): { near: number; far: number } {
  const d = c.distance * R;
  return { near: Math.max(0.002 * R, d - 2.2 * R), far: d + 3 * R };
}

/** Ray through a pixel (CSS px, origin top-left). */
export function rayFromPixel(f: Frame, x: number, y: number, w: number, h: number): { origin: V3; dir: V3 } {
  const nx = ((x + 0.5) / w) * 2 - 1, ny = 1 - ((y + 0.5) / h) * 2;
  const aspect = w / h;
  const sx = nx * f.tanHalf * aspect, sy = ny * f.tanHalf;
  const d: V3 = [f.fwd[0] + f.right[0] * sx + f.up[0] * sy, f.fwd[1] + f.right[1] * sx + f.up[1] * sy, f.fwd[2] + f.right[2] * sx + f.up[2] * sy];
  const l = Math.hypot(d[0], d[1], d[2]);
  return { origin: f.pos, dir: [d[0] / l, d[1] / l, d[2] / l] };
}

/** World point -> pixel; depth > 0 means in front of the camera. */
export function projectPoint(f: Frame, p: V3, w: number, h: number): { x: number; y: number; depth: number } {
  const v: V3 = [p[0] - f.pos[0], p[1] - f.pos[1], p[2] - f.pos[2]];
  const depth = v[0] * f.fwd[0] + v[1] * f.fwd[1] + v[2] * f.fwd[2];
  const rx = v[0] * f.right[0] + v[1] * f.right[1] + v[2] * f.right[2];
  const ry = v[0] * f.up[0] + v[1] * f.up[1] + v[2] * f.up[2];
  const aspect = w / h;
  return { x: ((rx / (depth * f.tanHalf * aspect)) * 0.5 + 0.5) * w - 0.5, y: (0.5 - (ry / (depth * f.tanHalf)) * 0.5) * h - 0.5, depth };
}

/** Camera that looks at world point `p` (double-click focus). Sphere and cube turn to face it; the plane slides over it. */
export function focusOn(c: OrbitCamera, kind: ShapeKind, p: V3, R: number): OrbitCamera {
  if (kind === 'plane') {
    return clampCamera({ ...c, target: [p[0] / R, p[1] / R, 0], distance: Math.min(c.distance, 0.9) }, kind);
  }
  const l = Math.hypot(p[0], p[1], p[2]) || 1;
  return clampCamera({ ...c, yaw: Math.atan2(p[1], p[0]), pitch: Math.asin(p[2] / l), distance: Math.min(c.distance, kind === 'cube' ? 2.6 : 2.2), target: [0, 0, 0] }, kind);
}

/** Shortest-way interpolation for the focus animation (yaw wraps). */
export function lerpCamera(a: OrbitCamera, b: OrbitCamera, t: number): OrbitCamera {
  let dy = b.yaw - a.yaw;
  if (dy > Math.PI) dy -= TAU; else if (dy < -Math.PI) dy += TAU;
  const s = t * t * (3 - 2 * t);
  return {
    kind: 'orbit', yaw: a.yaw + dy * s, pitch: a.pitch + (b.pitch - a.pitch) * s, distance: a.distance + (b.distance - a.distance) * s,
    target: [0, 1, 2].map((k) => a.target[k] + (b.target[k] - a.target[k]) * s) as V3,
  };
}

/** Pan: slide the target in the screen plane (plane/cube; limited on the sphere). */
export function panBy(c: OrbitCamera, kind: ShapeKind, R: number, dxPx: number, dyPx: number, viewH: number): OrbitCamera {
  const f = cameraFrame(c, R);
  const worldPerPx = (2 * f.tanHalf * c.distance * R) / viewH / R; // in units of R
  let dx = -dxPx * worldPerPx, dy = dyPx * worldPerPx;
  const t: V3 = [c.target[0] + f.right[0] * dx + f.up[0] * dy, c.target[1] + f.right[1] * dx + f.up[1] * dy, c.target[2] + f.right[2] * dx + f.up[2] * dy];
  if (kind === 'plane') t[2] = 0;
  return clampCamera({ ...c, target: t }, kind);
}
