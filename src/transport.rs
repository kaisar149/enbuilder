//! Stage C.7b: conservative moisture transport (flux-form upwind advection of column vapour).
//!
//! Method (Option A of PART_03b2): the 3a wind is not mass conserving, so the per-edge flux
//!     F_k = 0.5 (u_i . n + u_j . n) * edge_len        (m^2/s, outward from cell i across edge k)
//! is projected onto a divergence-free flux by a graph-Poisson solve (Jacobi-CG):
//!     A phi = div(F),  A = sum_j w_ij (phi_i - phi_j),  w = edge_len / edge_dist,
//!     F'_k = F_k - w_k (phi_i - phi_j)    =>    sum_k F'_k = 0 in every cell.
//! Vapour (kg/m^2) is advected with F' by first-order upwind, sub-stepped to a CFL limit:
//!     A_i dV_i/dt = sum_k [ -max(F'_k,0) V_i + max(-F'_k,0) V_j ] + A_i E_i
//! Fluxes are antisymmetric (F_ji = -F_ij) by construction, so total mass changes only by the sources.
//! Upwind with CFL <= 1 keeps V >= 0. Edge normals live in the tangent plane at the edge midpoint;
//! wrapped edges are detected like `GradientOp` does. Domain edges (walls/void) carry no flux.
//!
//! Inputs : wind (3 layers, world-space tangent vector), evaporation
//! Outputs: vapor, vapor_start (initial state of this run, for budgets), wind_flux, wind_flux_corrected (layers = max cell degree, aligned with mesh adjacency order,
//!          padded with 0), wind_corrected (3 layers), wind_divergence (raw, 1/s), divergence_residual (dimensionless)
//! Consequence of projecting: large-scale convergence (ITCZ) is removed from the transport wind; the discarded
//! part is kept in `wind_divergence` for the convective rain of the next part.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::gradient::GradientOp;
use crate::math::{dot, length, scale, sub, Vec3};
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::precipitation::{terms, PrecipParams, PrecipState, PRECIP_PARAMS};

fn reverse_index(mesh: &Mesh, i: usize, k: usize) -> Option<usize> {
    let j = mesh.adj[k] as usize;
    let (a, b) = (mesh.adj_offsets[j] as usize, mesh.adj_offsets[j + 1] as usize);
    mesh.adj[a..b].binary_search(&(i as u32)).ok().map(|p| a + p)
}

/// Per-edge flux (m^2/s), parallel to `mesh.adj`, exactly antisymmetric.
pub fn edge_flux(domain: &Domain, u: &[Vec3]) -> Vec<f64> {
    let m = &domain.mesh;
    let mut f = vec![0.0; m.adj.len()];
    for i in 0..m.n_cells {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            let j = m.adj[k] as usize;
            if j <= i {
                continue;
            }
            let dist = m.edge_dist[k];
            let mut d = sub(m.centroid[j], m.centroid[i]);
            if length(d) > 1.5 * dist {
                d = scale(d, -1.0);
            }
            let mid = [m.up[i][0] + m.up[j][0], m.up[i][1] + m.up[j][1], m.up[i][2] + m.up[j][2]];
            let ml = length(mid);
            if ml < 1e-12 {
                continue;
            }
            let mid = scale(mid, 1.0 / ml);
            let t = sub(d, scale(mid, dot(d, mid)));
            let tl = length(t);
            if !(tl > 1e-12 * dist) {
                continue;
            }
            let n = scale(t, 1.0 / tl);
            let val = 0.5 * (dot(u[i], n) + dot(u[j], n)) * m.edge_len[k];
            f[k] = val;
            if let Some(r) = reverse_index(m, i, k) {
                f[r] = -val;
            }
        }
    }
    f
}

/// Net outward flux per cell (m^2/s).
pub fn flux_divergence(mesh: &Mesh, f: &[f64]) -> Vec<f64> {
    (0..mesh.n_cells)
        .map(|i| f[mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize].iter().sum())
        .collect()
}

fn lap_matvec(mesh: &Mesh, x: &[f64], y: &mut [f64]) {
    for i in 0..mesh.n_cells {
        let mut s = 0.0;
        for k in mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize {
            s += mesh.edge_len[k] / mesh.edge_dist[k] * (x[i] - x[mesh.adj[k] as usize]);
        }
        y[i] = s;
    }
}

/// Solve A phi = rhs (graph Laplacian, singular on constants) by Jacobi-preconditioned CG. Returns (phi, iterations).
fn laplace_cg(mesh: &Mesh, rhs: &[f64], tol: f64, max_iters: usize) -> (Vec<f64>, usize) {
    let n = rhs.len();
    let diag: Vec<f64> = (0..n)
        .map(|i| {
            let d: f64 = (mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize)
                .map(|k| mesh.edge_len[k] / mesh.edge_dist[k])
                .sum();
            if d > 0.0 { d } else { 1.0 }
        })
        .collect();
    let mean = rhs.iter().sum::<f64>() / n as f64;
    let mut r: Vec<f64> = rhs.iter().map(|v| v - mean).collect();
    let bn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
    let mut x = vec![0.0; n];
    if bn == 0.0 {
        return (x, 0);
    }
    let mut z: Vec<f64> = (0..n).map(|i| r[i] / diag[i]).collect();
    let mut p = z.clone();
    let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
    let mut ap = vec![0.0; n];
    let mut it = 0;
    while it < max_iters {
        it += 1;
        lap_matvec(mesh, &p, &mut ap);
        let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
        if !(pap > 0.0) {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= tol * bn {
            break;
        }
        for i in 0..n {
            z[i] = r[i] / diag[i];
        }
        let rz2: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let beta = rz2 / rz;
        rz = rz2;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    (x, it)
}

/// Remove the divergence from an antisymmetric edge flux. Returns (phi, corrected flux).
pub fn project_flux(domain: &Domain, f: &[f64], tol: f64, max_iters: usize) -> (Vec<f64>, Vec<f64>) {
    let (phi, fc, _) = project_flux_counted(domain, f, tol, max_iters);
    (phi, fc)
}

/// As `project_flux`, also returning the CG iteration count (used by the 3c-4 parity tests).
pub fn project_flux_counted(domain: &Domain, f: &[f64], tol: f64, max_iters: usize) -> (Vec<f64>, Vec<f64>, usize) {
    let m = &domain.mesh;
    let div = flux_divergence(m, f);
    let (phi, iters) = laplace_cg(m, &div, tol, max_iters);
    let mut fc = f.to_vec();
    for i in 0..m.n_cells {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            fc[k] -= m.edge_len[k] / m.edge_dist[k] * (phi[i] - phi[m.adj[k] as usize]);
        }
    }
    (phi, fc, iters)
}

/// Largest stable explicit step (s) for upwind advection with flux `f` at the given CFL number.
pub fn stable_dt(mesh: &Mesh, f: &[f64], cfl: f64) -> f64 {
    let mut dt = f64::INFINITY;
    for i in 0..mesh.n_cells {
        let out: f64 = f[mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize].iter().map(|v| v.max(0.0)).sum();
        if out > 0.0 {
            dt = dt.min(cfl * mesh.area[i] / out);
        }
    }
    dt
}

/// Advance V (kg/m^2) by `total_s` seconds with flux `f` and source `evap` (kg/m^2/s). Returns (V, substeps).
pub fn advect(mesh: &Mesh, f: &[f64], v0: &[f64], evap: &[f64], total_s: f64, cfl: f64, max_substeps: usize) -> Result<(Vec<f64>, usize)> {
    advect_with_sink(mesh, f, v0, evap, total_s, cfl, max_substeps, None)
}

/// Like `advect`, but after every sub-step `sink(&mut V, dt)` may remove water (operator splitting, used for rain).
pub fn advect_with_sink(mesh: &Mesh, f: &[f64], v0: &[f64], evap: &[f64], total_s: f64, cfl: f64, max_substeps: usize, mut sink: Option<&mut dyn FnMut(&mut [f64], f64)>) -> Result<(Vec<f64>, usize)> {
    let n = mesh.n_cells;
    let mut v = v0.to_vec();
    if total_s <= 0.0 {
        return Ok((v, 0));
    }
    let dt_max = stable_dt(mesh, f, cfl);
    let nsub = if dt_max.is_finite() { (total_s / dt_max).ceil().max(1.0) } else { 1.0 };
    if nsub > max_substeps as f64 {
        return Err(Error::InvalidParam(format!("moisture_transport: needs {nsub} substeps (> max_substeps {max_substeps}); lower transport_time_s or raise max_substeps")));
    }
    let nsub = nsub as usize;
    let dt = total_s / nsub as f64;
    let mut nv = vec![0.0; n];
    for _ in 0..nsub {
        for i in 0..n {
            let mut s = 0.0;
            for k in mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize {
                let fk = f[k];
                s += if fk > 0.0 { -fk * v[i] } else { -fk * v[mesh.adj[k] as usize] };
            }
            nv[i] = (v[i] + dt * (s / mesh.area[i] + evap[i])).max(0.0);
        }
        std::mem::swap(&mut v, &mut nv);
        if let Some(sk) = sink.as_mut() {
            sk(&mut v, dt);
        }
    }
    Ok((v, nsub))
}

const PARAMS: [(&str, f64); 7] = [
    ("transport_time_s", 2_592_000.0), // 30 days per run (P/E ~ 0.8; 5 days left vapour far from steady state)
    ("init_residence_s", 864_000.0), // spin-up guess V0 = E * residence (10 days, near the Earth-like steady state) when there is no warm start
    ("warm_start", 0.0),             // 1 = start from an existing `vapor` field (otherwise ignored)
    ("cfl", 0.5),
    ("max_substeps", 100_000.0),
    ("projection_tol", 1e-12),       // relative CG residual
    ("projection_max_iters", 20_000.0),
];

pub struct MoistureTransport { params: ParamSet, precip: bool }

impl Default for MoistureTransport {
    fn default() -> Self { Self::new() }
}

impl MoistureTransport {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        MoistureTransport { params, precip: false }
    }

    /// Transport coupled with lift, condensation and precipitation (Part 3b-3). Adds inputs `temperature`,
    /// `elevation` and outputs `precipitation`, `precip_oro`, `precip_conv`, `precip_front`, `lift`.
    pub fn with_precipitation() -> Self {
        let mut m = Self::new();
        for (k, v) in PRECIP_PARAMS.iter() {
            m.params.set(k, *v);
        }
        m.precip = true;
        m
    }
}

impl Module for MoistureTransport {
    fn spec(&self) -> ModuleSpec {
        let mut inputs = vec!["wind", "evaporation"];
        let mut outputs = vec!["vapor", "vapor_start", "wind_flux", "wind_flux_corrected", "wind_corrected", "wind_divergence", "divergence_residual"];
        let mut hooks = vec!["wind_forcing".into(), "humidity".into()];
        if self.precip {
            inputs.extend(["temperature", "elevation"]);
            outputs.extend(["precipitation", "precip_oro", "precip_conv", "precip_front", "precip_sat", "lift"]);
            hooks.extend(["precipitation".into(), "gravity".into()]);
        }
        ModuleSpec {
            name: "moisture_transport".into(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
            anomaly_hooks: hooks,
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let is_precip = self.precip && PRECIP_PARAMS.iter().any(|(k, _)| *k == name);
        if !(PARAMS.iter().any(|(k, _)| *k == name) || is_precip) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("moisture_transport: bad parameter '{name}'")));
        }
        let bad = match name {
            "lift_gain" | "dry_adiabatic_scale" | "conv_rate" | "front_gain" | "sat_rh" => value < 0.0,
            "sat_tau_s" => value <= 0.0,
            "vapor_scale_ratio" | "conv_ramp_k" => value <= 0.0,
            "rh_trigger" => !(0.0..1.0).contains(&value),
            "removal_fraction" => !(0.0..=1.0).contains(&value),
            "oro_on" | "conv_on" | "front_on" | "sat_on" => value != 0.0 && value != 1.0,
            "conv_threshold_T" | "conv_div_ref" => value <= 0.0,
            "conv_dyn_min" | "conv_dyn_max" => value < 0.0,
            "transport_time_s" | "init_residence_s" => value < 0.0,
            "warm_start" => value != 0.0 && value != 1.0,
            "cfl" => !(value > 0.0 && value <= 1.0),
            "max_substeps" | "projection_max_iters" => value < 1.0,
            "projection_tol" => !(value > 0.0 && value < 1.0),
            _ => false,
        };
        if bad {
            return Err(Error::InvalidParam(format!("moisture_transport: '{name}' out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let n = domain.n_cells();
        let m = &domain.mesh;
        let maxdeg = (0..n).map(|i| m.degree(i)).max().unwrap_or(0).max(1);
        let pad = |f: &[f64]| -> Vec<f64> {
            let mut out = vec![0.0; n * maxdeg];
            for i in 0..n {
                let a = m.adj_offsets[i] as usize;
                for q in 0..m.degree(i) {
                    out[i * maxdeg + q] = f[a + q];
                }
            }
            out
        };
        let zero_out = |fields: &mut FieldStore| -> Result<()> {
            fields.put("vapor", 1, vec![0.0; n])?;
            fields.put("vapor_start", 1, vec![0.0; n])?;
            fields.put("wind_flux", maxdeg, vec![0.0; n * maxdeg])?;
            fields.put("wind_flux_corrected", maxdeg, vec![0.0; n * maxdeg])?;
            fields.put("wind_corrected", 3, vec![0.0; 3 * n])?;
            fields.put("wind_divergence", 1, vec![0.0; n])?;
            fields.put("divergence_residual", 1, vec![0.0; n])?;
            if self.precip {
                for name in ["precipitation", "precip_oro", "precip_conv", "precip_front", "precip_sat", "lift"] {
                    fields.put(name, 1, vec![0.0; n])?;
                }
            }
            Ok(())
        };
        if domain.media.atmosphere.is_none() {
            return zero_out(fields);
        }

        let w = fields.data("wind")?.to_vec();
        let evap = fields.data("evaporation")?.to_vec();
        let u: Vec<Vec3> = (0..n).map(|i| [w[3 * i], w[3 * i + 1], w[3 * i + 2]]).collect();

        let f = edge_flux(domain, &u);
        let div = flux_divergence(m, &f);
        let (phi, fc) = project_flux(domain, &f, p("projection_tol"), p("projection_max_iters") as usize);
        let div_c = flux_divergence(m, &fc);
        let fscale = (0..n)
            .map(|i| f[m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize].iter().map(|v| v.abs()).sum::<f64>())
            .fold(0.0, f64::max);
        let resid: Vec<f64> = div_c.iter().map(|d| if fscale > 0.0 { d / fscale } else { 0.0 }).collect();
        let graw: Vec<Vec3> = GradientOp::new(domain).gradient(domain, &phi);
        let mut wc = Vec::with_capacity(3 * n);
        for i in 0..n {
            wc.extend_from_slice(&sub(u[i], graw[i]));
        }

        let v0: Vec<f64> = if p("warm_start") == 1.0 && fields.has("vapor") {
            fields.data("vapor")?.to_vec()
        } else {
            evap.iter().map(|e| e * p("init_residence_s")).collect()
        };
        let total_s = p("transport_time_s");
        let mut acc = vec![vec![0.0; n]; 4]; // accumulated oro / conv / front / sat (kg/m^2)
        let mut lift_field = vec![0.0; n];
        let v = if self.precip {
            let par = PrecipParams::from_params(&self.params);
            let sea = p("sea_level");
            let zc: Vec<f64> = fields.data("elevation")?.iter().map(|&z| z.max(sea)).collect();
            let temp = fields.data("temperature")?.to_vec();
            let gz = GradientOp::new(domain).gradient(domain, &zc);
            let uc: Vec<Vec3> = (0..n).map(|i| sub(u[i], graw[i])).collect();
            let rawdiv: Vec<f64> = (0..n).map(|i| div[i] / m.area[i]).collect();
            let st = PrecipState::build(domain, &uc, &gz, &temp, &rawdiv, p("lift_gain"), &par);
            lift_field = st.lift.clone();
            let mut sink = |v: &mut [f64], dt: f64| {
                for i in 0..n {
                    let (a, b, c, e) = terms(&par, &st, i, v[i], dt);
                    acc[0][i] += a;
                    acc[1][i] += b;
                    acc[2][i] += c;
                    acc[3][i] += e;
                    v[i] = (v[i] - a - b - c - e).max(0.0);
                }
            };
            advect_with_sink(m, &fc, &v0, &evap, total_s, p("cfl"), p("max_substeps") as usize, Some(&mut sink))?.0
        } else {
            advect(m, &fc, &v0, &evap, total_s, p("cfl"), p("max_substeps") as usize)?.0
        };
        if self.precip {
            let rate = |a: &[f64]| -> Vec<f64> { a.iter().map(|x| if total_s > 0.0 { x / total_s } else { 0.0 }).collect() };
            let tot: Vec<f64> = (0..n).map(|i| (acc[0][i] + acc[1][i] + acc[2][i] + acc[3][i]) / total_s.max(1e-300)).collect();
            fields.put("precipitation", 1, if total_s > 0.0 { tot } else { vec![0.0; n] })?;
            fields.put("precip_oro", 1, rate(&acc[0]))?;
            fields.put("precip_conv", 1, rate(&acc[1]))?;
            fields.put("precip_front", 1, rate(&acc[2]))?;
            fields.put("precip_sat", 1, rate(&acc[3]))?;
            fields.put("lift", 1, lift_field)?;
        }

        fields.put("vapor", 1, v)?;
        fields.put("vapor_start", 1, v0)?;
        fields.put("wind_flux", maxdeg, pad(&f))?;
        fields.put("wind_flux_corrected", maxdeg, pad(&fc))?;
        fields.put("wind_corrected", 3, wc)?;
        fields.put("wind_divergence", 1, (0..n).map(|i| div[i] / m.area[i]).collect())?;
        fields.put("divergence_residual", 1, resid)
    }
}

/// Registers `moisture_transport`. Needs `wind` (3a) and `evaporation` (3b-1).
pub fn add_moisture_transport_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(MoistureTransport::new()))
}

/// Registers `moisture_transport` coupled with precipitation. Needs `wind`, `evaporation`, `temperature`, `elevation`.
pub fn add_moisture_precip_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(MoistureTransport::with_precipitation()))
}
