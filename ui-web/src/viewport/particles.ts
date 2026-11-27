// Part 9e: animated streaks. A small particle system in MAP space, advanced by a per-cell flow table. Pure (no DOM, injectable random source),
// so a test can prove that particles follow the field. Rendering (trails) is the viewport's job.
export interface Particles {
  n: number;
  trail: number;
  /** Positions in map units; the last `trail` positions of every particle (newest first): index = (i * trail + k) * 2. */
  hist: Float32Array;
  age: Float32Array;
}

export function makeParticles(n: number, trail: number, rand: () => number): Particles {
  const p: Particles = { n, trail, hist: new Float32Array(n * trail * 2), age: new Float32Array(n) };
  for (let i = 0; i < n; i++) respawn(p, i, rand, true);
  return p;
}

function respawn(p: Particles, i: number, rand: () => number, spreadAge: boolean): void {
  const u = rand(), v = rand();
  for (let k = 0; k < p.trail; k++) { p.hist[(i * p.trail + k) * 2] = u; p.hist[(i * p.trail + k) * 2 + 1] = v; }
  p.age[i] = spreadAge ? rand() : 0;
}

export interface StepOptions {
  /** Map-widths per second at relative magnitude 1. */
  speed: number;
  /** Seconds a particle lives. */
  life: number;
  /** Map width / map height. */
  aspect: number;
  /** The map repeats in u (equirectangular). */
  wrap: boolean;
}

/**
 * Moves every particle by the flow of the cell under it. `cellAt(u, v)` returns a cell or -1 (outside the map). A particle that leaves the
 * map, sits on a cell without flow for good, or gets too old is respawned at a random place.
 */
export function stepParticles(p: Particles, flow: Float32Array, cellAt: (u: number, v: number) => number, dt: number, o: StepOptions, rand: () => number): void {
  for (let i = 0; i < p.n; i++) {
    const base = i * p.trail * 2;
    let u = p.hist[base], v = p.hist[base + 1];
    p.age[i] += dt / o.life;
    const c = cellAt(((u % 1) + 1) % 1, v);
    if (p.age[i] >= 1 || c < 0) { respawn(p, i, rand, false); continue; }
    u += flow[2 * c] * o.speed * dt;
    v += flow[2 * c + 1] * o.speed * dt * o.aspect;
    if (o.wrap) u = ((u % 1) + 1) % 1;
    if (u < 0 || u >= 1 || v < 0 || v >= 1) { respawn(p, i, rand, false); continue; }
    // shift the trail: newest first
    p.hist.copyWithin(base + 2, base, base + (p.trail - 1) * 2);
    p.hist[base] = u; p.hist[base + 1] = v;
  }
}

/** Deterministic random source (mulberry32) for tests and for a stable first frame. */
export function seededRandom(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
