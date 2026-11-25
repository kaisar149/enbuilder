//! Part 7b: the storm and electricity module. Charge potential + moisture + convection -> lightning -> fire, nitrogen, rain seeding.
//!
//! Module `storm`. Inputs: `precip_conv` (kg/m^2/s), `precipitation` (kg/m^2/s), `vapor` (kg/m^2), `temperature` (K, first layer), `elevation` (m).
//! Optional anomaly hooks (compiled Source fields): `charge_potential` (1 = strong storm field), `magnetic_field`.
//! Outputs (all per cell; land-only ones are 0 over ocean `elevation < sea_level`):
//!   `charge`              q = background_charge + charge_potential hook (>= 0)
//!   `convective_activity` c = min(1, precip_conv/conv_ref + updraft_gain * q * m * warm), the charge-driven part is the "turbulent updraft"
//!   `lightning_rate`      flashes / km^2 / yr = flash_max * min(q,q_cap) * c * m * warm * (ocean ? ocean_ratio : 1)
//!   `fire_frequency`      ignitions / yr over a 1 km^2 = lightning * ignition_prob * dryness * fuel   (land)
//!   `fire_adapted`        0/1: fire_frequency >= fire_adapted_threshold (feeds biome conditions)
//!   `soil_nitrogen_flux`  kg N / ha / yr = lightning * n_per_flash (land);  `soil_nitrogen` = min(1, flux / n_ref)
//!   `precip_seeding`      kg/m^2/s = seed_gain * vapor * (lightning / flash_max) / seed_tau_s;  `precip_seeded` = precipitation + seeding
//!   `fulgurite_density`   lightning * dryness * (1 - fuel) on land (strikes on bare dry ground)
//!   `compass_disturbance` min(1, magnetic_field + lightning / flash_max)
//! m = clamp(vapor / vapor_ref, 0, 1); warm = clamp((T - 273.15) / warm_ramp_k, 0, 1);
//! dryness = 1 - clamp(precip / wet_ref, 0, 1); fuel = clamp(precip / fuel_ref, 0, 1).
//! DEVIATION (recorded): `precip_seeded` is a diagnostic overlay. The seeded water is not removed from the transport run, so the water budget of
//! the climate pipeline (which uses `precipitation`) is unchanged; the extra rain is bounded by `seed_gain` * vapor / seed_tau_s per cell.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

pub const STORM_PARAMS: [(&str, f64); 16] = [
    ("background_charge", 0.1),
    ("conv_ref", 3.0e-5),
    ("vapor_ref", 20.0),
    ("warm_ramp_k", 15.0),
    ("updraft_gain", 0.5),
    ("flash_max", 50.0),
    ("q_cap", 3.0),
    ("ocean_ratio", 0.1),
    ("ignition_prob", 0.05),
    ("wet_ref", 6.0e-5),
    ("fuel_ref", 1.0e-5),
    ("fire_adapted_threshold", 0.2),
    ("n_per_flash", 0.03),
    ("n_ref", 1.5),
    ("seed_gain", 0.1),
    ("seed_tau_s", 172_800.0),
];
const SEA_LEVEL_DEFAULT: f64 = 0.0;

pub struct Storm {
    params: ParamSet,
}

impl Default for Storm {
    fn default() -> Self { Self::new() }
}

impl Storm {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in STORM_PARAMS.iter() {
            params.set(k, *v);
        }
        params.set("sea_level", SEA_LEVEL_DEFAULT);
        Storm { params }
    }
}

fn first_layer(fields: &FieldStore, name: &str, n: usize) -> Result<Vec<f64>> {
    let l = fields.layers(name)?;
    let d = fields.data(name)?;
    if l == 0 || d.len() != n * l {
        return Err(Error::InvalidParam(format!("storm: field '{name}' must have one value per cell and layer")));
    }
    Ok((0..n).map(|i| d[i * l]).collect())
}

fn ramp01(x: f64) -> f64 { if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 } }

impl Module for Storm {
    fn spec(&self) -> ModuleSpec {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        ModuleSpec {
            name: "storm".into(),
            inputs: s(&["precip_conv", "precipitation", "vapor", "temperature", "elevation"]),
            outputs: s(&[
                "charge", "convective_activity", "lightning_rate", "fire_frequency", "fire_adapted", "soil_nitrogen_flux", "soil_nitrogen",
                "precip_seeding", "precip_seeded", "fulgurite_density", "compass_disturbance",
            ]),
            anomaly_hooks: s(&["charge_potential", "magnetic_field"]),
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let known = name == "sea_level" || STORM_PARAMS.iter().any(|(k, _)| *k == name);
        if !known || !value.is_finite() {
            return Err(Error::InvalidParam(format!("storm: bad parameter '{name}'")));
        }
        let ok = match name {
            "sea_level" => true,
            "ocean_ratio" | "ignition_prob" | "background_charge" | "updraft_gain" | "seed_gain" | "fire_adapted_threshold" => value >= 0.0,
            _ => value > 0.0,
        };
        if !ok || (name == "ocean_ratio" && value > 1.0) || (name == "ignition_prob" && value > 1.0) {
            return Err(Error::InvalidParam(format!("storm.{name} out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = |k: &str| self.params.get_or(k, 0.0);
        let conv = first_layer(fields, "precip_conv", n)?;
        let precip = first_layer(fields, "precipitation", n)?;
        let vapor = first_layer(fields, "vapor", n)?;
        let temp = first_layer(fields, "temperature", n)?;
        let z = first_layer(fields, "elevation", n)?;
        let q_hook = crate::anomaly::source_of(fields, "charge_potential")?;
        let mag = crate::anomaly::source_of(fields, "magnetic_field")?;
        let sea = p("sea_level");
        let (mut charge, mut act, mut lit) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        let (mut fire, mut adapted, mut nflux, mut nrm) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        let (mut seed, mut seeded, mut fulg, mut compass) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            let q = (p("background_charge") + q_hook.as_ref().map_or(0.0, |h| h[i])).max(0.0);
            let m = ramp01(vapor[i] / p("vapor_ref"));
            let warm = ramp01((temp[i] - 273.15) / p("warm_ramp_k"));
            let c = (conv[i].max(0.0) / p("conv_ref") + p("updraft_gain") * q * m * warm).min(1.0);
            let land = z[i] >= sea;
            let l = p("flash_max") * q.min(p("q_cap")) * c * m * warm * if land { 1.0 } else { p("ocean_ratio") };
            let dry = 1.0 - ramp01(precip[i] / p("wet_ref"));
            let fuel = ramp01(precip[i] / p("fuel_ref"));
            charge[i] = q;
            act[i] = c;
            lit[i] = l;
            if land {
                fire[i] = l * p("ignition_prob") * dry * fuel;
                adapted[i] = if fire[i] >= p("fire_adapted_threshold") { 1.0 } else { 0.0 };
                nflux[i] = l * p("n_per_flash");
                nrm[i] = (nflux[i] / p("n_ref")).min(1.0);
                fulg[i] = l * dry * (1.0 - fuel);
            }
            seed[i] = p("seed_gain") * vapor[i].max(0.0) * (l / p("flash_max")) / p("seed_tau_s");
            seeded[i] = precip[i] + seed[i];
            compass[i] = (mag.as_ref().map_or(0.0, |h| h[i]) + l / p("flash_max")).clamp(0.0, 1.0);
        }
        fields.put("charge", 1, charge)?;
        fields.put("convective_activity", 1, act)?;
        fields.put("lightning_rate", 1, lit)?;
        fields.put("fire_frequency", 1, fire)?;
        fields.put("fire_adapted", 1, adapted)?;
        fields.put("soil_nitrogen_flux", 1, nflux)?;
        fields.put("soil_nitrogen", 1, nrm)?;
        fields.put("precip_seeding", 1, seed)?;
        fields.put("precip_seeded", 1, seeded)?;
        fields.put("fulgurite_density", 1, fulg)?;
        fields.put("compass_disturbance", 1, compass)
    }
}

/// Registers module `storm` (after the climate pipeline; before `biomes` / `derived_maps` when those read its outputs).
pub fn add_storm_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> { s.register(Box::new(Storm::new())) }
