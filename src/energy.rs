//! Stage B.4: energy balance with ice-albedo feedback.
//!
//! Two modules form a declared feedback loop (temperature -> albedo -> temperature):
//!   AlbedoModel   : temperature, elevation -> albedo, ice_fraction
//!   EnergyBalance : insolation, albedo, elevation -> temperature_sl, temperature
//!
//! Per cell, steady state of
//!   A_i [(1-alpha_i) Q_i - eps sigma T_i^4] + K sum_j (l_ij/d_ij) (T_j - T_i) = 0
//! with eps = 1 - greenhouse and K = transport_coeff * L_ref^2, L_ref^2 = total_area/(4 pi)
//! (the equal-area-sphere radius squared, so `transport_coeff` is North's D in W m^-2 K^-1
//! on a sphere and well-defined on any other domain). Solved by Newton iteration with a
//! Jacobi-preconditioned conjugate-gradient inner solve; the matrix is SPD.
//! `temperature_sl` is the sea-level-equivalent temperature; `temperature` is the surface value
//! after a lapse correction  Gamma = lapse_fraction * g / cp_air  (so it depends on gravity and
//! the atmosphere's heat capacity). With no atmosphere: no greenhouse and no lapse.
//! This is an equilibrium model for the chosen season: no thermal inertia yet, so seasonal
//! swings are exaggerated.
use crate::consts::SIGMA;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::insolation::Insolation;
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::{FeedbackLoop, Scheduler};

// ---------------------------------------------------------------- albedo

const ALBEDO_PARAMS: [(&str, f64); 7] = [
    ("sea_level", 0.0),
    ("albedo_land", 0.22),
    ("albedo_water", 0.06),
    ("albedo_ice", 0.60),
    ("cloud_albedo", 0.20), // placeholder: uniform cloud cover, brightens every surface
    ("ice_t_lo", 263.15),   // fully ice below
    ("ice_t_hi", 273.15),   // ice-free above
];

pub struct AlbedoModel { params: ParamSet }

impl Default for AlbedoModel {
    fn default() -> Self { Self::new() }
}

impl AlbedoModel {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in ALBEDO_PARAMS.iter() {
            params.set(k, *v);
        }
        AlbedoModel { params }
    }
}

impl Module for AlbedoModel {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "albedo".into(),
            inputs: vec!["temperature".into(), "elevation".into()],
            outputs: vec!["albedo".into(), "ice_fraction".into()],
            anomaly_hooks: vec!["albedo".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !ALBEDO_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("albedo: bad parameter '{name}'")));
        }
        let unit = matches!(name, "albedo_land" | "albedo_water" | "albedo_ice" | "cloud_albedo");
        if unit && !(0.0..=1.0).contains(&value) {
            return Err(Error::InvalidParam(format!("albedo: '{name}' must be in [0,1]")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, _domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let (lo, hi) = (p("ice_t_lo"), p("ice_t_hi"));
        if !(hi > lo) {
            return Err(Error::InvalidParam("albedo: ice_t_hi must exceed ice_t_lo".into()));
        }
        let t = fields.data("temperature")?.to_vec();
        let e = fields.data("elevation")?.to_vec();
        let n = t.len();
        let (mut alb, mut ice) = (vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            let f = ((hi - t[i]) / (hi - lo)).clamp(0.0, 1.0);
            let surf = if e[i] < p("sea_level") { p("albedo_water") } else { p("albedo_land") };
            let a0 = f * p("albedo_ice") + (1.0 - f) * surf;
            alb[i] = a0 + (1.0 - a0) * p("cloud_albedo");
            ice[i] = f;
        }
        fields.put("albedo", 1, alb)?;
        fields.put("ice_fraction", 1, ice)?;
        Ok(())
    }
}

// ---------------------------------------------------------------- energy balance

const EB_PARAMS: [(&str, f64); 9] = [
    ("transport_coeff", 0.55),
    ("greenhouse", 0.39),
    ("t_init", 288.0),
    ("newton_iters", 8.0),
    ("cg_tol", 1e-8),
    ("cg_max_iters", 3000.0),
    ("lapse_fraction", 0.65),
    ("sea_level", 0.0),
    ("max_step_k", 40.0),
];

pub struct EnergyBalance {
    params: ParamSet,
    insolation_field: String,
    /// Part 4a-3: when true the module also reads `ocean_heat_flux` (W/m^2, positive = heats the atmosphere).
    ocean_flux: bool,
}

impl Default for EnergyBalance {
    fn default() -> Self { Self::new() }
}

impl EnergyBalance {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in EB_PARAMS.iter() {
            params.set(k, *v);
        }
        EnergyBalance { params, insolation_field: "insolation".into(), ocean_flux: false }
    }
    /// Part 4a-3 variant: declares the extra input `ocean_heat_flux` (W/m^2 per cell, area integral zero) and adds it to the
    /// balance. Module specs are fixed at construction, so this is a constructor and not a parameter (same pattern as 3b-3).
    /// With the field identically zero the result is bit-identical to `new()`.
    pub fn with_ocean_heat() -> Self {
        let mut m = Self::new();
        m.ocean_flux = true;
        m
    }
    /// Use another insolation field, e.g. "insolation_annual".
    pub fn with_insolation_field(mut self, name: &str) -> Self {
        self.insolation_field = name.to_string();
        self
    }
}

/// y = diag*x - K * sum_j w_ij x_j  (w = edge_len / edge_dist)
fn matvec(mesh: &Mesh, w: &[f64], kt: f64, diag: &[f64], x: &[f64], y: &mut [f64]) {
    for i in 0..mesh.n_cells {
        let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
        let mut s = 0.0;
        for q in a..b {
            s += w[q] * x[mesh.adj[q] as usize];
        }
        y[i] = diag[i] * x[i] - kt * s;
    }
}

fn cg(mesh: &Mesh, w: &[f64], kt: f64, diag: &[f64], rhs: &[f64], tol: f64, max_iters: usize) -> Vec<f64> {
    let n = rhs.len();
    let mut x = vec![0.0; n];
    let mut r = rhs.to_vec();
    let mut z: Vec<f64> = (0..n).map(|i| r[i] / diag[i]).collect();
    let mut p = z.clone();
    let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
    let bn = r.iter().map(|v| v * v).sum::<f64>().sqrt() + 1e-300;
    let mut ap = vec![0.0; n];
    for _ in 0..max_iters {
        matvec(mesh, w, kt, diag, &p, &mut ap);
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
    x
}

impl Module for EnergyBalance {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "energy_balance".into(),
            inputs: {
                let mut v = vec![self.insolation_field.clone(), "albedo".into(), "elevation".into()];
                if self.ocean_flux {
                    v.push("ocean_heat_flux".into());
                }
                v
            },
            outputs: vec!["temperature_sl".into(), "temperature".into()],
            anomaly_hooks: vec!["temperature_source".into(), "greenhouse".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !EB_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("energy_balance: bad parameter '{name}'")));
        }
        if (name == "greenhouse" && !(0.0..0.95).contains(&value))
            || (name == "transport_coeff" && value < 0.0)
            || (name == "t_init" && value <= 0.0)
            || (name == "newton_iters" && value < 1.0)
            || (name == "max_step_k" && value <= 0.0)
        {
            return Err(Error::InvalidParam(format!("energy_balance: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let p = |k: &str| self.params.get_or(k, 0.0);
        let q_in = fields.data(&self.insolation_field)?.to_vec();
        let alb = fields.data("albedo")?.to_vec();
        let elev = fields.data("elevation")?.to_vec();
        let q_ocean: Option<Vec<f64>> = if self.ocean_flux {
            let q = fields.data("ocean_heat_flux")?;
            if q.len() != n || q.iter().any(|v| !v.is_finite()) {
                return Err(Error::InvalidParam("energy_balance: 'ocean_heat_flux' must be 1 finite layer".into()));
            }
            Some(q.to_vec())
        } else {
            None
        };

        let atm = domain.media.atmosphere.as_ref();
        let eps = if atm.is_some() { 1.0 - p("greenhouse") } else { 1.0 };
        let lapse: Vec<f64> = (0..n)
            .map(|i| match atm {
                Some(f) if f.heat_capacity > 0.0 => p("lapse_fraction") * domain.gravity_magnitude(i) / f.heat_capacity,
                _ => 0.0,
            })
            .collect();

        let w: Vec<f64> = mesh.edge_len.iter().zip(mesh.edge_dist.iter()).map(|(l, d)| l / d).collect();
        let kt = p("transport_coeff") * mesh.total_area() / (4.0 * std::f64::consts::PI);
        let wsum: Vec<f64> = (0..n)
            .map(|i| (mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize).map(|q| w[q]).sum())
            .collect();

        // warm start from the previous solution when present (also used by the feedback loop)
        let mut t: Vec<f64> = match fields.get("temperature_sl") {
            Some(f) if f.data.len() == n => f.data.clone(),
            _ => vec![p("t_init"); n],
        };
        let max_step = p("max_step_k");
        for _ in 0..(p("newton_iters") as usize) {
            let mut resid = vec![0.0; n];
            let mut diag = vec![0.0; n];
            for i in 0..n {
                let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
                let mut lap = 0.0;
                for q in a..b {
                    lap += w[q] * (t[mesh.adj[q] as usize] - t[i]);
                }
                let t3 = t[i] * t[i] * t[i];
                let heat = q_ocean.as_ref().map_or(0.0, |q| q[i]);
                resid[i] = mesh.area[i] * ((1.0 - alb[i]) * q_in[i] - eps * SIGMA * t3 * t[i] + heat) + kt * lap;
                diag[i] = mesh.area[i] * 4.0 * eps * SIGMA * t3 + kt * wsum[i];
            }
            let dt = cg(mesh, &w, kt, &diag, &resid, p("cg_tol"), p("cg_max_iters") as usize);
            let mut biggest = 0.0f64;
            for i in 0..n {
                let step = dt[i].clamp(-max_step, max_step);
                t[i] = (t[i] + step).clamp(50.0, 600.0);
                biggest = biggest.max(step.abs());
            }
            if biggest < 1e-4 {
                break;
            }
        }
        let sea = p("sea_level");
        let surface: Vec<f64> = (0..n).map(|i| t[i] - lapse[i] * (elev[i] - sea).max(0.0)).collect();
        fields.put("temperature_sl", 1, t)?;
        fields.put("temperature", 1, surface)?;
        Ok(())
    }
}

// ---------------------------------------------------------------- helpers

/// Warm-start fields the ice-albedo loop needs before its first pass.
/// (Also the mechanism for warm-starting re-solves after edits.)
pub fn seed_temperature(fields: &mut FieldStore, t: f64) -> Result<()> {
    let n = fields.n_cells();
    fields.put("temperature_sl", 1, vec![t; n])?;
    fields.put("temperature", 1, vec![t; n])
}

/// Registers insolation + albedo + energy balance and declares the ice-albedo loop
/// (relaxation 0.5, tolerance 2e-3, i.e. ~0.6 K, at most 50 passes).
pub fn add_energy_stage(s: &mut Scheduler) -> Result<()> { add_energy_stage_with(s, Insolation::new()) }

pub fn add_energy_stage_with(s: &mut Scheduler, insolation: Insolation) -> Result<()> {
    s.register(Box::new(insolation))?;
    s.register(Box::new(AlbedoModel::new()))?;
    s.register(Box::new(EnergyBalance::new()))?;
    s.add_feedback_loop(
        FeedbackLoop::new("ice_albedo", &["albedo", "energy_balance"])
            .watch(&["temperature_sl", "temperature"])
            .relaxation(0.5)
            .tolerance(2e-3)
            .max_iters(50),
    )
}

/// Part 4a-3: like `add_energy_stage` but the energy balance is the `with_ocean_heat` variant, so it needs the field
/// `ocean_heat_flux` to exist (see `ocean_heat::seed_ocean_heat_flux`). Same `ice_albedo` loop.
pub fn add_energy_stage_ocean_flux(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(Insolation::new()))?;
    s.register(Box::new(AlbedoModel::new()))?;
    s.register(Box::new(EnergyBalance::with_ocean_heat()))?;
    s.add_feedback_loop(
        FeedbackLoop::new("ice_albedo", &["albedo", "energy_balance"])
            .watch(&["temperature_sl", "temperature"])
            .relaxation(0.5)
            .tolerance(2e-3)
            .max_iters(50),
    )
}
