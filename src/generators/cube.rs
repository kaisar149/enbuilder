//! Cube world: six square faces of N x N cells. Cells on a face edge are
//! adjacent to cells on the neighbouring face (cross-edge adjacency), found by
//! "folding" the off-face point around the edge. Gravity points at the cube
//! centre, so `up` is radial (tilts at edges/corners), not the face normal.
use crate::domain::{BoundaryKind, Domain, DomainKind};
use crate::error::{Error, Result};
use crate::math::*;
use crate::mesh::MeshBuilder;

#[derive(Debug, Clone)]
pub struct CubeParams {
    /// Half the side length in metres (side = 2 * half_side).
    pub half_side: f64,
    pub cells_per_edge: usize,
    /// Gravity magnitude toward the centre.
    pub surface_gravity: f64,
    pub seed: u64,
}

impl Default for CubeParams {
    fn default() -> Self { CubeParams { half_side: 3.0e6, cells_per_edge: 32, surface_gravity: 9.81, seed: 0 } }
}

/// (normal, u, v). Face order: +x, -x, +y, -y, +z, -z.
const FACES: [(Vec3, Vec3, Vec3); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

fn mix(a: Vec3, ka: f64, b: Vec3, kb: f64, c: Vec3, kc: f64) -> Vec3 {
    add(add(scale(a, ka), scale(b, kb)), scale(c, kc))
}

/// Map a point on the unit-cube surface to its cell index.
fn locate(q: Vec3, n: usize) -> usize {
    let mut a = 0;
    for k in 1..3 {
        if q[k].abs() > q[a].abs() {
            a = k;
        }
    }
    let face = a * 2 + if q[a] > 0.0 { 0 } else { 1 };
    let (_, u, v) = FACES[face];
    let nf = n as f64;
    let hi = n as i64 - 1;
    let ci = (((dot(q, u) + 1.0) * nf / 2.0).floor() as i64).clamp(0, hi) as usize;
    let cj = (((dot(q, v) + 1.0) * nf / 2.0).floor() as i64).clamp(0, hi) as usize;
    face * n * n + cj * n + ci
}

pub fn generate_cube(p: &CubeParams) -> Result<Domain> {
    if !(p.half_side > 0.0 && p.half_side.is_finite()) {
        return Err(Error::InvalidParam("half_side must be positive".into()));
    }
    if p.cells_per_edge == 0 || p.cells_per_edge > 2048 {
        return Err(Error::InvalidParam("cells_per_edge must be in 1..=2048".into()));
    }
    let n = p.cells_per_edge;
    let h = 2.0 / n as f64; // cell size in unit-cube coordinates
    let hw = p.half_side;
    let cell_size = h * hw;
    let mut mb = MeshBuilder::new(6 * n * n);
    let mut gravity = Vec::with_capacity(6 * n * n);
    for f in 0..6 {
        let (nrm, u, v) = FACES[f];
        for j in 0..n {
            for i in 0..n {
                let idx = f * n * n + j * n + i;
                let s = -1.0 + (2 * i + 1) as f64 / n as f64;
                let t = -1.0 + (2 * j + 1) as f64 / n as f64;
                let q = mix(nrm, 1.0, u, s, v, t);
                let up = normalize(q);
                mb.set_cell(idx, cell_size * cell_size, scale(q, hw), up);
                gravity.push(scale(up, -p.surface_gravity));
                for &(du, dv) in &[(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let s2 = s + du * h;
                    let t2 = t + dv * h;
                    let pt = if s2.abs() < 1.0 && t2.abs() < 1.0 {
                        mix(nrm, 1.0, u, s2, v, t2)
                    } else if s2 > 1.0 {
                        mix(u, 1.0, nrm, 1.0 - (s2 - 1.0), v, t)
                    } else if s2 < -1.0 {
                        mix(u, -1.0, nrm, 1.0 - (-1.0 - s2), v, t)
                    } else if t2 > 1.0 {
                        mix(v, 1.0, nrm, 1.0 - (t2 - 1.0), u, s)
                    } else {
                        mix(v, -1.0, nrm, 1.0 - (-1.0 - t2), u, s)
                    };
                    mb.add_neighbor(idx, locate(pt, n), cell_size, cell_size);
                }
            }
        }
    }
    let mesh = mb.build();
    Domain::new(DomainKind::Cube, mesh, gravity, BoundaryKind::Closed, [0.0, 0.0, 1.0], p.seed)
}
