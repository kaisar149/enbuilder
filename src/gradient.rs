//! Least-squares tangent-plane gradient on any `Domain` (sphere, cube, plane, ...).
//!
//! For each cell the neighbour offsets are projected into the cell's tangent plane (basis
//! east/north from the surface frame) and a weighted 2x2 least-squares fit gives
//!   grad(phi)_i = sum_k coef[k] * (phi[j_k] - phi[i]),   coef[k] a world-space tangent vector.
//! Only mesh adjacency, centroids and the frames are used, so nothing assumes a topology.
//! Wrapped edges (a plane torus) are detected by centroid separation >> edge_dist; the true
//! direction of such an edge is the opposite of the centroid difference.
use crate::domain::Domain;
use crate::math::{add, dot, length, scale, sub, Vec3};

pub struct GradientOp {
    /// Parallel to `mesh.adj`.
    coef: Vec<Vec3>,
}

impl GradientOp {
    pub fn new(domain: &Domain) -> GradientOp {
        let m = &domain.mesh;
        let mut coef: Vec<Vec3> = vec![[0.0; 3]; m.adj.len()];
        for i in 0..m.n_cells {
            let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
            let fr = domain.frames[i];
            let mut items: Vec<(usize, f64, f64, f64)> = Vec::with_capacity(b - a); // (k, dx, dy, w)
            let (mut sxx, mut sxy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
            for k in a..b {
                let j = m.adj[k] as usize;
                let dist = m.edge_dist[k];
                let mut v = sub(m.centroid[j], m.centroid[i]);
                if length(v) > 1.5 * dist {
                    v = scale(v, -1.0);
                }
                v = sub(v, scale(fr.up, dot(v, fr.up)));
                let l = length(v);
                if !(l > 1e-12 * dist) {
                    continue;
                }
                let d = scale(v, dist / l);
                let (dx, dy) = (dot(d, fr.east), dot(d, fr.north));
                let w = 1.0 / (dist * dist);
                sxx += w * dx * dx;
                sxy += w * dx * dy;
                syy += w * dy * dy;
                items.push((k, dx, dy, w));
            }
            let eps = 1e-6 * (sxx + syy);
            sxx += eps;
            syy += eps;
            let det = sxx * syy - sxy * sxy;
            if !(det > 0.0) {
                continue;
            }
            for &(k, dx, dy, w) in &items {
                let cx = w * (syy * dx - sxy * dy) / det;
                let cy = w * (-sxy * dx + sxx * dy) / det;
                coef[k] = add(scale(fr.east, cx), scale(fr.north, cy));
            }
        }
        GradientOp { coef }
    }

    /// Stencil coefficients parallel to `mesh.adj` (uploaded to the GPU by 3c-3).
    pub fn coef(&self) -> &[Vec3] { &self.coef }

    /// World-space tangent gradient of a scalar field.
    pub fn gradient(&self, domain: &Domain, phi: &[f64]) -> Vec<Vec3> {
        let m = &domain.mesh;
        (0..m.n_cells)
            .map(|i| {
                let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
                let mut g: Vec3 = [0.0; 3];
                for k in a..b {
                    let j = m.adj[k] as usize;
                    g = add(g, scale(self.coef[k], phi[j] - phi[i]));
                }
                g
            })
            .collect()
    }
}
