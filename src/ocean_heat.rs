//! Part 4a-3: ocean heat transport and the SST feedback to the energy balance.
//!
//! Module `ocean_heat`:  `temperature_sl`, `current`, `ocean_mask`  ->  `sst` (K, 0 off the ocean), `ocean_heat_flux` (W/m^2, 0 off the ocean).
//!
//! Model (one mixed layer of heat capacity `C = rho cp H` J/m^2/K, per ocean cell):
//!   `C dT/dt = C [ -div(u T) + kappa lap(T) ] + C (T_atm - T) / tau`
//! The first bracket is the TRANSPORT tendency `tend_i` (K/s), written as conservative upwind edge fluxes with the divergence-free edge flux of
//! the current (masked to ocean-ocean edges and, if `project = 1`, projected on the ocean subgraph), plus a two-point diffusion. It is integrated
//! to steady state by explicit sub-steps. The air-sea flux is the relaxation term. The atmosphere receives
//!   `ocean_heat_flux_i = C * tend_i`      (W/m^2, positive heats the atmosphere)
//! evaluated on the final `T`. Because `tend` is a sum of antisymmetric edge fluxes, `sum_i A_i * flux_i = 0` at ANY `T`, converged or not:
//! heat is moved, never created. At steady state `C tend = C (T - T_atm)/tau`, i.e. the same number is the air-sea exchange (output-only
//! `air_sea_mismatch` measures how close the run got).
//!
//! The scheduler cannot hold `[ocean_currents, ocean_heat, energy_balance]` in one loop (see ARCHITECTURE.md, 4a-3), so the feedback is
//! driven by `run_ocean_climate`: scheduler A (climate pipeline + currents) and scheduler B (loop `ocean_climate`), alternated for `outer_passes`.
use crate::domain::Domain;
use crate::energy::{AlbedoModel, EnergyBalance};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::insolation::Insolation;
use crate::math::Vec3;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::ocean_forcing::unpack_vec3;
use crate::scheduler::{FeedbackLoop, Scheduler};
use crate::transport::edge_flux;

/// rho cp H for rho = 1025 kg/m^3, cp = 3990 J/kg/K, H = 20 m (the same layer thickness the current stage uses, `DEFAULT_H_MIXED_M`).
pub const DEFAULT_MIXED_LAYER_HEAT_CAPACITY: f64 = 1025.0 * 3990.0 * 20.0;
/// Relaxation time of the SST toward the atmosphere (s); with the default capacity this is a 16 W/m^2/K air-sea coupling.
pub const DEFAULT_SST_RELAX_S: f64 = 5.0e6;

const OH_PARAMS: [(&str, f64); 9] = [
    ("mixed_layer_heat_capacity", DEFAULT_MIXED_LAYER_HEAT_CAPACITY), // J/m^2/K
    ("sst_relax_s", DEFAULT_SST_RELAX_S),
    ("diffusivity", 1000.0), // m^2/s, lateral eddy diffusivity
    ("cfl", 0.5),
    ("max_steps", 200_000.0),
    ("steady_tol_k", 1e-5), // stop when the largest per-step change is below this (K)
    ("project", 1.0),       // 1 = project the masked edge flux to zero divergence on the ocean subgraph
    ("t_freeze", 271.35),   // K, lower bound of the reported `sst` (sea ice itself is not modelled here)
    ("warm_start", 1.0),    // 1 = start from an existing `sst` field
];
/// Output-only (set by `run`, rejected by `set_param`).
const OH_OUT: [&str; 6] = ["last_steps", "last_residual_k", "flux_closure", "div_raw_rel", "air_sea_mismatch", "ocean_cells"];

pub struct OceanHeat {
    params: ParamSet,
}

impl Default for OceanHeat {
    fn default() -> Self { Self::new() }
}

impl OceanHeat {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in OH_PARAMS.iter() {
            params.set(k, *v);
        }
        for k in OH_OUT.iter() {
            params.set(k, 0.0);
        }
        OceanHeat { params }
    }
}

/// Zero every edge flux whose two cells are not both ocean (keeps the flux antisymmetric: both directions are zeroed).
pub fn mask_flux(domain: &Domain, ocean: &[bool], f: &mut [f64]) {
    let m = &domain.mesh;
    for i in 0..m.n_cells {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            if !(ocean[i] && ocean[m.adj[k] as usize]) {
                f[k] = 0.0;
            }
        }
    }
}

/// Remove the divergence of a masked edge flux with a Jacobi-CG Poisson solve on the OCEAN subgraph (ocean-ocean edges only).
/// Returns `(raw_divergence_rel, iterations)`, where `raw_divergence_rel = max|div| / max_i sum_k |F_ik|` before the projection.
/// Each connected ocean component is a consistent Neumann problem because the flux is antisymmetric, so the constant modes are never excited.
pub fn project_ocean_flux(domain: &Domain, ocean: &[bool], f: &mut [f64], tol: f64, max_iters: usize) -> (f64, usize) {
    let m = &domain.mesh;
    let n = m.n_cells;
    let w: Vec<f64> = (0..m.adj.len()).map(|k| m.edge_len[k] / m.edge_dist[k]).collect();
    let on = |i: usize, k: usize| ocean[i] && ocean[m.adj[k] as usize];
    let mut div = vec![0.0; n];
    let mut scale: f64 = 0.0;
    for i in 0..n {
        let mut s = 0.0;
        let mut a = 0.0;
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            s += f[k];
            a += f[k].abs();
        }
        div[i] = s;
        scale = scale.max(a);
    }
    let raw = if scale > 0.0 { div.iter().fold(0.0f64, |mx, v| mx.max(v.abs())) / scale } else { 0.0 };
    let bn = div.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !(bn > 0.0) {
        return (raw, 0);
    }
    let diag: Vec<f64> = (0..n).map(|i| (m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize).filter(|&k| on(i, k)).map(|k| w[k]).sum()).collect();
    let matvec = |x: &[f64], y: &mut [f64]| {
        for i in 0..n {
            let mut s = 0.0;
            for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
                if on(i, k) {
                    s += w[k] * (x[i] - x[m.adj[k] as usize]);
                }
            }
            y[i] = s;
        }
    };
    let prec = |r: &[f64], z: &mut [f64]| {
        for i in 0..n {
            z[i] = if diag[i] > 0.0 { r[i] / diag[i] } else { 0.0 };
        }
    };
    let mut x = vec![0.0; n];
    let mut r = div.clone();
    let mut z = vec![0.0; n];
    prec(&r, &mut z);
    let mut p = z.clone();
    let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
    let mut ap = vec![0.0; n];
    let mut it = 0;
    while it < max_iters {
        matvec(&p, &mut ap);
        let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
        if !(pap > 0.0) {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        it += 1;
        if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= tol * bn {
            break;
        }
        prec(&r, &mut z);
        let rz2: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let beta = rz2 / rz;
        rz = rz2;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    for i in 0..n {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            if on(i, k) {
                f[k] -= w[k] * (x[i] - x[m.adj[k] as usize]);
            }
        }
    }
    (raw, it)
}

/// Transport tendency (K/s) of tracer `t` for every cell: conservative upwind advection with the antisymmetric edge flux `f` (m^2/s)
/// plus diffusion `dk` (m^2/s per edge, parallel to `mesh.adj`, zero off the ocean). `sum_i A_i tend_i = 0` to rounding.
pub fn transport_tendency(domain: &Domain, f: &[f64], dk: &[f64], t: &[f64], out: &mut [f64]) {
    let m = &domain.mesh;
    for i in 0..m.n_cells {
        let mut s = 0.0;
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            let tj = t[m.adj[k] as usize];
            let fk = f[k];
            s += if fk > 0.0 { -fk * t[i] } else { -fk * tj };
            s += dk[k] * (tj - t[i]);
        }
        out[i] = s / m.area[i];
    }
}

impl Module for OceanHeat {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "ocean_heat".into(),
            inputs: ["temperature_sl", "current", "ocean_mask"].iter().map(|s| s.to_string()).collect(),
            outputs: ["sst", "ocean_heat_flux"].iter().map(|s| s.to_string()).collect(),
            anomaly_hooks: vec!["sea_temperature_source".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if OH_OUT.contains(&name) {
            return Err(Error::InvalidParam(format!("ocean_heat: '{name}' is output-only")));
        }
        if !OH_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("ocean_heat: bad parameter '{name}'")));
        }
        if (matches!(name, "mixed_layer_heat_capacity" | "sst_relax_s" | "max_steps" | "steady_tol_k" | "t_freeze") && value <= 0.0)
            || (name == "diffusivity" && value < 0.0)
            || (name == "cfl" && !(value > 0.0 && value <= 1.0))
            || (matches!(name, "project" | "warm_start") && !(value == 0.0 || value == 1.0))
        {
            return Err(Error::InvalidParam(format!("ocean_heat: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let p = |k: &str| self.params.get_or(k, 0.0);
        let t_atm = fields.data("temperature_sl")?.to_vec();
        let cur_flat = fields.data("current")?;
        if t_atm.len() != n || cur_flat.len() != 3 * n {
            return Err(Error::InvalidParam("ocean_heat: 'temperature_sl' must have 1 layer and 'current' 3 layers".into()));
        }
        if t_atm.iter().any(|v| !v.is_finite()) || cur_flat.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("ocean_heat input".into()));
        }
        let cur: Vec<Vec3> = unpack_vec3(cur_flat);
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        if ocean.len() != n {
            return Err(Error::InvalidParam("ocean_heat: 'ocean_mask' must have 1 layer".into()));
        }
        let (cap, tau, kappa) = (p("mixed_layer_heat_capacity"), p("sst_relax_s"), p("diffusivity"));

        // Masked, optionally projected edge flux (m^2/s).
        let mut f = edge_flux(domain, &cur);
        mask_flux(domain, &ocean, &mut f);
        let div_raw = if p("project") > 0.5 { project_ocean_flux(domain, &ocean, &mut f, 1e-12, 20_000).0 } else { project_ocean_flux(domain, &ocean, &mut f.clone(), 1e-12, 0).0 };
        // Diffusion coefficient per edge (m^2/s), ocean-ocean edges only.
        let mut dk = vec![0.0; mesh.adj.len()];
        for i in 0..n {
            for k in mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize {
                if ocean[i] && ocean[mesh.adj[k] as usize] {
                    dk[k] = kappa * mesh.edge_len[k] / mesh.edge_dist[k];
                }
            }
        }
        // Stable explicit step: every coefficient of T_i stays non-negative at this cfl.
        let mut rate = 0.0f64;
        for i in 0..n {
            if !ocean[i] {
                continue;
            }
            let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
            let out: f64 = (a..b).map(|k| f[k].max(0.0) + dk[k]).sum::<f64>() / mesh.area[i] + 1.0 / tau;
            rate = rate.max(out);
        }
        let dt = p("cfl") / rate.max(1e-300);

        // State.
        let mut t: Vec<f64> = t_atm.clone();
        if p("warm_start") > 0.5 {
            if let Some(old) = fields.get("sst") {
                if old.data.len() == n {
                    for i in 0..n {
                        if ocean[i] && old.data[i] > 0.0 {
                            t[i] = old.data[i];
                        }
                    }
                }
            }
        }
        let mut tend = vec![0.0; n];
        let (tol, max_steps) = (p("steady_tol_k"), p("max_steps") as usize);
        let (mut steps, mut last) = (0usize, f64::INFINITY);
        let n_ocean = ocean.iter().filter(|&&o| o).count();
        if n_ocean > 0 {
            while steps < max_steps {
                transport_tendency(domain, &f, &dk, &t, &mut tend);
                let mut biggest = 0.0f64;
                for i in 0..n {
                    if !ocean[i] {
                        continue;
                    }
                    let dtv = dt * (tend[i] + (t_atm[i] - t[i]) / tau);
                    t[i] += dtv;
                    biggest = biggest.max(dtv.abs());
                }
                steps += 1;
                last = biggest;
                if !t.iter().all(|v| v.is_finite()) {
                    return Err(Error::NonFinite("ocean_heat: SST".into()));
                }
                if biggest < tol {
                    break;
                }
            }
            if last >= tol {
                return Err(Error::NoConvergence { solver: "ocean_heat".into(), iterations: steps, residual: last });
            }
        } else {
            last = 0.0;
        }

        // Flux to the atmosphere from the final state: C * transport tendency.
        transport_tendency(domain, &f, &dk, &t, &mut tend);
        let mut q = vec![0.0; n];
        let (mut sq, mut sa, mut qmax, mut mism) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            if !ocean[i] {
                continue;
            }
            q[i] = cap * tend[i];
            sq += mesh.area[i] * q[i];
            sa += mesh.area[i] * q[i].abs();
            qmax = qmax.max(q[i].abs());
            mism = mism.max((cap * ((t[i] - t_atm[i]) / tau) - q[i]).abs());
        }
        let tf = p("t_freeze");
        let sst: Vec<f64> = (0..n).map(|i| if ocean[i] { t[i].max(tf) } else { 0.0 }).collect();

        self.params.set("last_steps", steps as f64);
        self.params.set("last_residual_k", if last.is_finite() { last } else { 0.0 });
        self.params.set("flux_closure", if sa > 0.0 { sq.abs() / sa } else { 0.0 });
        self.params.set("div_raw_rel", div_raw);
        self.params.set("air_sea_mismatch", if qmax > 0.0 { mism / qmax } else { 0.0 });
        self.params.set("ocean_cells", n_ocean as f64);
        fields.put("sst", 1, sst)?;
        fields.put("ocean_heat_flux", 1, q)?;
        Ok(())
    }
}

// ---------------------------------------------------------------- wiring and driver

/// Zero `ocean_heat_flux`, which the `with_ocean_heat` energy balance needs to exist before the first pass (no-op if present).
pub fn seed_ocean_heat_flux(fields: &mut FieldStore) -> Result<()> {
    if !fields.has("ocean_heat_flux") {
        let n = fields.n_cells();
        fields.put("ocean_heat_flux", 1, vec![0.0; n])?;
    }
    Ok(())
}

/// Scheduler B: insolation + albedo + `ocean_heat` + the `with_ocean_heat` energy balance in the loop `ocean_climate`
/// (members `[albedo, ocean_heat, energy_balance]`, watch `[temperature_sl, temperature, sst]`, relaxation 0.5, tolerance 2e-3, 30 passes).
/// `current`, `ocean_mask` come from the fields (scheduler A). Needs `seed_temperature` and `seed_ocean_heat_flux`.
pub fn add_ocean_climate_loop(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(Insolation::new()))?;
    s.register(Box::new(AlbedoModel::new()))?;
    s.register(Box::new(OceanHeat::new()))?;
    s.register(Box::new(EnergyBalance::with_ocean_heat()))?;
    s.add_feedback_loop(
        FeedbackLoop::new("ocean_climate", &["albedo", "ocean_heat", "energy_balance"])
            .watch(&["temperature_sl", "temperature", "sst"])
            .relaxation(0.5)
            .tolerance(2e-3)
            .max_iters(30),
    )
}

/// Scheduler A: the full climate chain with the ocean-flux energy balance, then the ocean mask, wind stress, gyre forcing and currents.
/// Same stages as `add_climate_pipeline` + `add_ocean_stage` + `add_gyre_forcing_stage` + `add_ocean_currents_stage`.
pub fn add_atmosphere_ocean_pipeline(s: &mut Scheduler) -> Result<()> {
    crate::energy::add_energy_stage_ocean_flux(s)?;
    crate::pressure::add_atmosphere_stage(s)?;
    crate::moisture::add_moisture_source_stage(s)?;
    crate::transport::add_moisture_precip_stage(s)?;
    crate::ocean::add_ocean_stage(s)?;
    crate::ocean_forcing::add_gyre_forcing_stage(s)?;
    crate::ocean_currents::add_ocean_currents_stage(s)
}

#[derive(Debug, Clone, PartialEq)]
pub struct OceanClimateReport {
    pub outer_passes: usize,
    /// Passes of loop `ocean_climate` and of loop `ice_albedo` per outer pass.
    pub heat_loop_passes: Vec<usize>,
    pub heat_loop_converged: Vec<bool>,
    pub ice_loop_passes: Vec<usize>,
    /// After each outer pass: max |change of `sst`| (K) since the previous pass (first entry: vs. no SST, i.e. 0).
    pub sst_change_k: Vec<f64>,
}

/// One season of the coupled climate: A, B, then (outer_passes - 1) times A, B again with the new temperature. With `outer_passes = 1` the
/// currents come from the climate that has no ocean heat transport (the winds see the ocean-less temperature); more passes let wind, stress
/// and currents respond to the SST-corrected temperature (a Picard iteration over the one cycle the scheduler cannot hold).
pub fn run_ocean_climate(domain: &Domain, fields: &mut FieldStore, atmos: &mut Scheduler, heat: &mut Scheduler, season: f64, outer_passes: usize) -> Result<OceanClimateReport> {
    if outer_passes == 0 {
        return Err(Error::InvalidParam("run_ocean_climate: outer_passes must be >= 1".into()));
    }
    seed_ocean_heat_flux(fields)?;
    let mut rep = OceanClimateReport { outer_passes, heat_loop_passes: vec![], heat_loop_converged: vec![], ice_loop_passes: vec![], sst_change_k: vec![] };
    for _ in 0..outer_passes {
        let before: Option<Vec<f64>> = fields.get("sst").map(|f| f.data.clone());
        atmos.set_param("insolation", "season", season)?;
        let ra = atmos.run(domain, fields, &DirtyRegion::All)?;
        rep.ice_loop_passes.push(ra.loops.iter().find(|l| l.name == "ice_albedo").map_or(0, |l| l.iterations));
        heat.set_param("insolation", "season", season)?;
        let rb = heat.run(domain, fields, &DirtyRegion::All)?;
        let l = rb.loops.iter().find(|l| l.name == "ocean_climate");
        rep.heat_loop_passes.push(l.map_or(0, |l| l.iterations));
        rep.heat_loop_converged.push(l.map_or(false, |l| l.converged));
        let after = fields.data("sst")?;
        rep.sst_change_k.push(match before {
            Some(b) if b.len() == after.len() => b.iter().zip(after.iter()).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())),
            _ => 0.0,
        });
    }
    Ok(rep)
}
