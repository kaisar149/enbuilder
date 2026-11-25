//! Part 7a: river hydraulics (Chezy-style flow speed). Gives gravity a visible job in the surface-water stage.
//!
//! Module `river_velocity`: inputs `discharge` (m^3/s), `river_mask` (0/1), `slope` (m/m); outputs `river_depth_m`, `river_velocity` (m/s).
//! Depth `h = depth_coef * Q^depth_exp` (Leopold-Maddock style hydraulic geometry), speed `v = sqrt(g h S)` with `S = max(slope, min_slope)`
//! and `g = |g|` of the cell (the Domain's gravity, so a gravity anomaly speeds rivers up with no special case). Non-river cells are 0.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

const PARAMS: [(&str, f64); 3] = [("depth_coef", 0.3), ("depth_exp", 0.4), ("min_slope", 1e-4)];

pub struct RiverVelocity {
    params: ParamSet,
}

impl Default for RiverVelocity {
    fn default() -> Self { Self::new() }
}

impl RiverVelocity {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        RiverVelocity { params }
    }
}

impl Module for RiverVelocity {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "river_velocity".into(),
            inputs: ["discharge", "river_mask", "slope"].iter().map(|s| s.to_string()).collect(),
            outputs: ["river_depth_m", "river_velocity"].iter().map(|s| s.to_string()).collect(),
            anomaly_hooks: vec!["gravity".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() || value <= 0.0 {
            return Err(Error::InvalidParam(format!("river_velocity: bad parameter '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let q = fields.data("discharge")?.to_vec();
        let m = fields.data("river_mask")?.to_vec();
        let s = fields.data("slope")?.to_vec();
        if q.len() != n || m.len() != n || s.len() != n {
            return Err(Error::InvalidParam("river_velocity: inputs must have one layer".into()));
        }
        let (c, e, smin) = (self.params.get_or("depth_coef", 0.3), self.params.get_or("depth_exp", 0.4), self.params.get_or("min_slope", 1e-4));
        let (mut h, mut v) = (vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            if m[i] > 0.5 && q[i] > 0.0 {
                h[i] = c * q[i].powf(e);
                v[i] = (domain.gravity_magnitude(i) * h[i] * s[i].max(smin)).sqrt();
            }
        }
        fields.put("river_depth_m", 1, h)?;
        fields.put("river_velocity", 1, v)
    }
}

pub fn add_river_velocity_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> { s.register(Box::new(RiverVelocity::new())) }
