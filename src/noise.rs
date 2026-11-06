//! Deterministic 3D value noise and fBm, evaluated on cell centroids so it is
//! independent of mesh topology (works on spheres, cubes and planes alike).
use crate::math::Vec3;

fn lattice(ix: i64, iy: i64, iz: i64, seed: u64) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (iz as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed.wrapping_mul(0x27D4_EB2F_1656_67C5);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

fn fade(t: f64) -> f64 { t * t * (3.0 - 2.0 * t) }

/// Smooth noise in about [-1, 1].
pub fn value_noise3(p: Vec3, seed: u64) -> f64 {
    let f = [p[0].floor(), p[1].floor(), p[2].floor()];
    let t = [fade(p[0] - f[0]), fade(p[1] - f[1]), fade(p[2] - f[2])];
    let (ix, iy, iz) = (f[0] as i64, f[1] as i64, f[2] as i64);
    let mut out = 0.0;
    for dx in 0..2i64 {
        for dy in 0..2i64 {
            for dz in 0..2i64 {
                let wx = if dx == 1 { t[0] } else { 1.0 - t[0] };
                let wy = if dy == 1 { t[1] } else { 1.0 - t[1] };
                let wz = if dz == 1 { t[2] } else { 1.0 - t[2] };
                out += wx * wy * wz * lattice(ix + dx, iy + dy, iz + dz, seed);
            }
        }
    }
    out
}

/// Fractal sum, normalised by total amplitude (persistence 0.5, lacunarity 2).
pub fn fbm3(p: Vec3, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut tot, mut freq) = (0.0, 1.0, 0.0, 1.0);
    for o in 0..octaves.max(1) {
        sum += amp * value_noise3([p[0] * freq, p[1] * freq, p[2] * freq], seed.wrapping_add(o as u64));
        tot += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / tot
}
