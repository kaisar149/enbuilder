// Port of wb-app/src/raster.rs colormaps. Stops must stay identical so the browser map matches the headless PNG.
// A colormap name that is unknown falls back to viridis, as in Rust.
export type RGB = [number, number, number];

const T = (t: number, c: RGB): [number, RGB] => [t, c];
const STOPS: Record<string, [number, RGB][]> = {
  terrain: [T(0, [8, 24, 90]), T(0.3, [30, 90, 170]), T(0.499, [120, 190, 230]), T(0.5, [70, 140, 70]), T(0.62, [150, 180, 80]), T(0.75, [150, 110, 70]), T(0.9, [120, 100, 95]), T(1, [250, 250, 250])],
  temperature: [T(0, [40, 20, 120]), T(0.2, [40, 100, 220]), T(0.4, [150, 215, 240]), T(0.5, [245, 245, 235]), T(0.65, [250, 215, 110]), T(0.82, [225, 90, 40]), T(1, [110, 10, 20])],
  rain: [T(0, [245, 240, 225]), T(0.2, [200, 225, 190]), T(0.45, [90, 175, 190]), T(0.7, [30, 90, 175]), T(1, [15, 25, 100])],
  speed: [T(0, [240, 245, 250]), T(0.35, [150, 200, 220]), T(0.7, [240, 160, 60]), T(1, [150, 20, 30])],
  diverging: [T(0, [33, 102, 172]), T(0.25, [103, 169, 207]), T(0.5, [247, 247, 247]), T(0.75, [239, 138, 98]), T(1, [178, 24, 43])],
  viridis: [T(0, [68, 1, 84]), T(0.25, [59, 82, 139]), T(0.5, [33, 145, 140]), T(0.75, [94, 201, 98]), T(1, [253, 231, 37])],
};

function lerp3(stops: [number, RGB][], t: number): RGB {
  t = Math.min(1, Math.max(0, t));
  for (let i = 0; i + 1 < stops.length; i++) {
    const [t0, c0] = stops[i];
    const [t1, c1] = stops[i + 1];
    if (t <= t1) {
      const f = t1 > t0 ? (t - t0) / (t1 - t0) : 0;
      return [0, 1, 2].map((k) => Math.round(c0[k] + (c1[k] - c0[k]) * f)) as RGB;
    }
  }
  return stops[stops.length - 1][1];
}

export function colormap(name: string, t: number): RGB {
  if (name === 'mask') return t >= 0.5 ? [235, 235, 235] : [30, 30, 36];
  return lerp3(STOPS[name] ?? STOPS.viridis, t);
}

const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const MASK64 = 0xffffffffffffffffn;
const catCache = new Map<number, RGB>();

/** Deterministic colour for category ids without a table: FNV-1a over the 8 little-endian bytes of the id (same as Rust `category_color`). */
export function categoryColor(id: number): RGB {
  const hit = catCache.get(id);
  if (hit) return hit;
  let h = FNV_OFFSET;
  let v = BigInt.asUintN(64, BigInt(id));
  for (let i = 0; i < 8; i++) {
    h ^= v & 0xffn;
    h = (h * FNV_PRIME) & MASK64;
    v >>= 8n;
  }
  const c = [0, 1, 2].map((k) => 60 + (Number((h >> BigInt(k * 8)) & 0xffn) % 190)) as RGB;
  catCache.set(id, c);
  return c;
}

/** CSS gradient for legends, sampled from the same stops. */
export function cssGradient(name: string, steps = 24): string {
  const parts: string[] = [];
  for (let i = 0; i <= steps; i++) {
    const [r, g, b] = colormap(name, i / steps);
    parts.push(`rgb(${r} ${g} ${b}) ${((i / steps) * 100).toFixed(1)}%`);
  }
  return `linear-gradient(to right, ${parts.join(', ')})`;
}
