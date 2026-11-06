//! Part 4a-2b: the two forcing quantities of the gyre equation, on any mesh.
//!
//! Modules and fields
//!   wind_stress_curl : wind_stress, ocean_mask -> stress_curl (N/m^3, ocean cells, exactly 0 elsewhere)
//!   beta_term        : coriolis_f              -> beta         (1/(m s), every cell)
//!
//! Curl = discrete Stokes circulation (frame-free; no east/north components of the stress are ever differentiated):
//!     curl_i = (1/area_i) * sum_k  tau_edge_ik . t_ik * len_ik,     t_ik = up_i x n_ik,
//! with n_ik the unit in-plane direction from cell i towards neighbour k.
//!
//! How the discrete form is actually evaluated (all recorded in ARCHITECTURE.md):
//!   1. `tau_edge` is written relative to the cell's own value, `tau_i + h_ik/d_ik * (tau_k - tau_i)`, where `h_ik` is the
//!      distance from the centroid to the edge. The `tau_i` part is `tau_i . sum(t len)`, the circulation of a constant
//!      field around a closed loop, which is 0; dropping it makes a uniform field give exactly 0 on every cell, whatever
//!      the cell shape. (So the land/boundary rule "use the cell's own value" simply means "no term".)
//!   2. `h_ik` is not stored in the mesh. It is taken as `(s_i/2) d_ik`, with `s_i = 4 area_i / sum(len d)` fixed by area
//!      closure, `area = 1/2 sum len h`. Weights become `2 len_ik t_ik / sum_m(len_im d_im)`.
//!   3. First-order consistency correction. On irregular cells (pentagons, distorted hexagons) the weights of (2) do not
//!      reproduce the curl of a linear field, so the error would NOT shrink with resolution. A minimal weighted-norm
//!      correction (same 2x2 least-squares machinery as `GradientOp`) makes the stencil exact for every linear tangent
//!      field. Regular cells (square grid, regular hexagon) get a correction of exactly 0.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::gradient::GradientOp;
use crate::math::{add, cross, dot, length, scale, sub, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

/// Unpack a 3-layer field (`x0,y0,z0,x1,...`) into vectors.
pub fn unpack_vec3(flat: &[f64]) -> Vec<Vec3> {
    flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

/// Circulation stencil: `curl_i = sum_k dot(coef[k], v[j_k] - v[i])`, `coef` parallel to `mesh.adj`
/// (tangent world vectors, units 1/m).
pub struct CurlOp {
    coef: Vec<Vec3>,
}

impl CurlOp {
    pub fn new(domain: &Domain) -> CurlOp {
        let m = &domain.mesh;
        let mut coef: Vec<Vec3> = vec![[0.0; 3]; m.adj.len()];
        for i in 0..m.n_cells {
            let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
            let fr = domain.frames[i];
            // (k, dx, dy, w, cx, cy): d = edge vector in the (east, north) basis with |d| = edge_dist, c = weight.
            let mut items: Vec<(usize, f64, f64, f64, f64, f64)> = Vec::with_capacity(b - a);
            let mut s_ld = 0.0;
            for k in a..b {
                let j = m.adj[k] as usize;
                let dist = m.edge_dist[k];
                let mut v = sub(m.centroid[j], m.centroid[i]);
                if length(v) > 1.5 * dist {
                    v = scale(v, -1.0); // wrapped edge: the true direction is opposite the centroid difference
                }
                v = sub(v, scale(fr.up, dot(v, fr.up)));
                let l = length(v);
                if !(l > 1e-12 * dist) {
                    continue;
                }
                let nrm = scale(v, 1.0 / l);
                let t = cross(fr.up, nrm);
                let len = m.edge_len[k];
                s_ld += len * dist;
                items.push((k, dist * dot(nrm, fr.east), dist * dot(nrm, fr.north), 1.0 / (dist * dist), len * dot(t, fr.east), len * dot(t, fr.north)));
            }
            if !(s_ld > 0.0) {
                continue;
            }
            let norm = 2.0 / s_ld;
            // Plain circulation weights, then the first-order consistency correction.
            let (mut b00, mut b01, mut b10, mut b11) = (0.0, 0.0, 0.0, 0.0); // B_ab = sum d_a c_b
            let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
            for &(_, dx, dy, w, cx, cy) in &items {
                let (cx, cy) = (cx * norm, cy * norm);
                b00 += dx * cx;
                b01 += dx * cy;
                b10 += dy * cx;
                b11 += dy * cy;
                sxx += w * dx * dx;
                sxy += w * dx * dy;
                syy += w * dy * dy;
            }
            let eps = 1e-10 * (sxx + syy);
            sxx += eps;
            syy += eps;
            let det = sxx * syy - sxy * sxy;
            let mut x = [[0.0f64; 2]; 2];
            if det > 0.0 {
                // Target B = [[0, 1], [-1, 0]] (curl = d_x tau_y - d_y tau_x). X = (T - B)^T S^-1.
                let (m00, m01, m10, m11) = (-b00, 1.0 - b01, -1.0 - b10, -b11);
                x[0][0] = (m00 * syy - m10 * sxy) / det;
                x[0][1] = (-m00 * sxy + m10 * sxx) / det;
                x[1][0] = (m01 * syy - m11 * sxy) / det;
                x[1][1] = (-m01 * sxy + m11 * sxx) / det;
            }
            for &(k, dx, dy, w, cx, cy) in &items {
                let cx = cx * norm + w * (x[0][0] * dx + x[0][1] * dy);
                let cy = cy * norm + w * (x[1][0] * dx + x[1][1] * dy);
                coef[k] = add(scale(fr.east, cx), scale(fr.north, cy));
            }
        }
        CurlOp { coef }
    }

    /// Stencil coefficients parallel to `mesh.adj`.
    pub fn coef(&self) -> &[Vec3] { &self.coef }

    /// Curl of a vector field on every cell (all neighbours used).
    pub fn apply(&self, domain: &Domain, v: &[Vec3]) -> Vec<f64> {
        let m = &domain.mesh;
        assert_eq!(v.len(), m.n_cells, "CurlOp::apply: length mismatch");
        (0..m.n_cells)
            .map(|i| {
                let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
                let mut s = 0.0;
                for k in a..b {
                    s += dot(self.coef[k], sub(v[m.adj[k] as usize], v[i]));
                }
                s
            })
            .collect()
    }

    /// Curl on the cells flagged in `ocean`; 0 elsewhere. A non-ocean neighbour contributes nothing, which is the
    /// zero-gradient rule (the cell's own value is used across that edge). Values stored on non-ocean cells are never read.
    pub fn apply_masked(&self, domain: &Domain, v: &[Vec3], ocean: &[bool]) -> Vec<f64> {
        let m = &domain.mesh;
        assert_eq!(v.len(), m.n_cells, "CurlOp::apply_masked: length mismatch");
        assert_eq!(ocean.len(), m.n_cells, "CurlOp::apply_masked: mask length mismatch");
        (0..m.n_cells)
            .map(|i| {
                if !ocean[i] {
                    return 0.0;
                }
                let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
                let mut s = 0.0;
                for k in a..b {
                    let j = m.adj[k] as usize;
                    if ocean[j] {
                        s += dot(self.coef[k], sub(v[j], v[i]));
                    }
                }
                s
            })
            .collect()
    }
}

/// Curl (1/s for a velocity, N/m^3 for a stress) of a vector field by discrete Stokes circulation, every cell.
/// Edges that leave the domain (wall/void) carry no term (zero gradient).
pub fn curl_of(domain: &Domain, v: &[Vec3]) -> Vec<f64> { CurlOp::new(domain).apply(domain, v) }

/// As `curl_of`, restricted to `ocean` cells; non-ocean neighbours use the cell's own value (zero gradient).
pub fn curl_of_masked(domain: &Domain, v: &[Vec3], ocean: &[bool]) -> Vec<f64> { CurlOp::new(domain).apply_masked(domain, v, ocean) }

/// `beta_i = grad(f)_i . north_i` (1/(m s)), with `GradientOp` on the scalar `coriolis_f`.
pub fn beta_of(domain: &Domain, coriolis_f: &[f64]) -> Vec<f64> {
    assert_eq!(coriolis_f.len(), domain.n_cells(), "beta_of: length mismatch");
    let g = GradientOp::new(domain).gradient(domain, coriolis_f);
    (0..domain.n_cells()).map(|i| dot(g[i], domain.frames[i].north)).collect()
}

// ---------------------------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------------------------

/// `stress_curl = curl(wind_stress)` on ocean cells (N/m^3), 0 on land and lakes.
pub struct WindStressCurl { params: ParamSet }

impl Default for WindStressCurl {
    fn default() -> Self { Self::new() }
}

impl WindStressCurl {
    pub fn new() -> Self { WindStressCurl { params: ParamSet::new() } }
}

impl Module for WindStressCurl {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "wind_stress_curl".into(), inputs: names(&["wind_stress", "ocean_mask"]), outputs: names(&["stress_curl"]), anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, _value: f64) -> Result<()> {
        Err(Error::InvalidParam(format!("wind_stress_curl has no parameters (got '{name}')")))
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let tau = fields.data("wind_stress")?;
        if tau.len() != 3 * n {
            return Err(Error::InvalidParam("wind_stress_curl: 'wind_stress' must have 3 layers".into()));
        }
        if tau.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("wind_stress".into()));
        }
        let v = unpack_vec3(tau);
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        let out = curl_of_masked(domain, &v, &ocean);
        fields.put("stress_curl", 1, out)
    }
}

/// `beta = d f / d y` along the local north.
pub struct BetaTerm { params: ParamSet }

impl Default for BetaTerm {
    fn default() -> Self { Self::new() }
}

impl BetaTerm {
    pub fn new() -> Self { BetaTerm { params: ParamSet::new() } }
}

impl Module for BetaTerm {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "beta_term".into(), inputs: names(&["coriolis_f"]), outputs: names(&["beta"]), anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, _value: f64) -> Result<()> {
        Err(Error::InvalidParam(format!("beta_term has no parameters (got '{name}')")))
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let f = fields.data("coriolis_f")?;
        if f.len() != domain.n_cells() {
            return Err(Error::InvalidParam("beta_term: 'coriolis_f' must have 1 layer".into()));
        }
        if f.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("coriolis_f".into()));
        }
        let out = beta_of(domain, f);
        fields.put("beta", 1, out)
    }
}

/// Registers `wind_stress_curl` (needs `wind_stress`, `ocean_mask`) and `beta_term` (needs `coriolis_f`).
pub fn add_gyre_forcing_stage(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(WindStressCurl::new()))?;
    s.register(Box::new(BetaTerm::new()))
}
