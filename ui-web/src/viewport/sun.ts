// Direction of the star seen from the planet, in the planet frame. A visual aid computed in the client from the world parameters with the same
// geometry as the core's insolation module (equinox direction +X, spin axis Z; the plane spins about +Y, its "north"): it never goes to the core.
export interface SunParams { axial_tilt_deg: number; year_length_days: number; day_length_h: number }
type V3 = [number, number, number];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const lin = (a: V3, x: number, b: V3, y: number): V3 => [a[0] * x + b[0] * y, a[1] * x + b[1] * y, a[2] * x + b[2] * y];

/**
 * @param season fraction of the year 0..1 (0 = equinox, as in the core; ignored when the world has no orbit)
 * @param timeOfDay fraction of the day 0..1 (0.5 = the sun is over the +X meridian); ignored when the world does not rotate (static sun)
 */
export function sunDirection(p: SunParams, plane: boolean, season: number, timeOfDay: number): V3 {
  const W: V3 = plane ? [0, 1, 0] : [0, 0, 1];
  const a: V3 = [1, 0, 0];
  let s = a;
  if (p.year_length_days > 0) {
    const eps = (p.axial_tilt_deg * Math.PI) / 180;
    const N = lin(W, Math.cos(eps), cross(a, W), Math.sin(eps));
    const phi = 2 * Math.PI * season;
    s = lin(a, Math.cos(phi), cross(N, a), Math.sin(phi));
  }
  if (p.day_length_h > 0) {
    const th = -2 * Math.PI * (timeOfDay - 0.5) ; // rotate the planet under the sun
    const c = Math.cos(th), sn = Math.sin(th);
    const k = cross(W, s);
    const d = W[0] * s[0] + W[1] * s[1] + W[2] * s[2];
    s = [s[0] * c + k[0] * sn + W[0] * d * (1 - c), s[1] * c + k[1] * sn + W[1] * d * (1 - c), s[2] * c + k[2] * sn + W[2] * d * (1 - c)];
  }
  const l = Math.hypot(s[0], s[1], s[2]) || 1;
  return [s[0] / l, s[1] / l, s[2] / l];
}
