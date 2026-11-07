//! Part 4a-4: the secondary ocean fields. Four independent modules, each with a switch that zeroes only its own outputs.
//!
//!   `upwelling`    : `wind_stress`, `coriolis_f`, `ocean_mask`            -> `upwelling`, `ekman_pumping`, `coastal_upwelling` (m/s, up positive)
//!   `thermohaline` : `precipitation`, `evaporation`, `temperature_sl`, `ocean_mask`
//!                                                                         -> `salinity` (psu), `ocean_density`, `deep_water_formation` (0/1), `thermohaline_flux` (W/m^2)
//!   `sea_ice`      : `sst`, `salinity`, `ocean_mask`                      -> `sea_ice` (m), `sea_ice_fraction`
//!   `nutrients`    : `upwelling`, `coast_distance`, `ocean_mask`          -> `nutrients` (0..1)
//!
//! `salinity` and `ocean_density` are always computed (only the deep-water flag and the flux obey `thermohaline_on`), so switching one
//! process off never changes another process's output. `nutrients` depends on `upwelling` by definition.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::{cross, dot, scale, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::ocean_currents::coast_vectors;
use crate::ocean_forcing::{curl_of_masked, unpack_vec3};
use crate::scheduler::Scheduler;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

fn make_params(defaults: &[(&str, f64)]) -> ParamSet {
    let mut p = ParamSet::new();
    for (k, v) in defaults {
        p.set(k, *v);
    }
    p
}

fn check_set(module: &str, defaults: &[(&str, f64)], name: &str, value: f64) -> Result<()> {
    if !defaults.iter().any(|(k, _)| *k == name) || !value.is_finite() {
        return Err(Error::InvalidParam(format!("{module}: bad parameter '{name}'")));
    }
    if name.ends_with("_on") && !(value == 0.0 || value == 1.0) {
        return Err(Error::InvalidParam(format!("{module}: switch '{name}' must be 0 or 1")));
    }
    Ok(())
}

fn field1(fields: &FieldStore, name: &str, n: usize) -> Result<Vec<f64>> {
    let d = fields.data(name)?;
    if d.len() != n {
        return Err(Error::InvalidParam(format!("'{name}' must have 1 layer")));
    }
    if d.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite(name.to_string()));
    }
    Ok(d.to_vec())
}

fn mask_of(fields: &FieldStore, n: usize) -> Result<Vec<bool>> { Ok(field1(fields, "ocean_mask", n)?.iter().map(|&m| m > 0.5).collect()) }

// ============================================================================================ upwelling
const UP_PARAMS: [(&str, f64); 6] = [
    ("upwelling_on", 1.0),
    ("coastal_on", 1.0),
    ("rho_water", 1025.0),
    ("f_floor", 2.0e-5),       // 1/s: |f| below this (about 8 degrees at Earth rotation) gets no Ekman terms (equatorial dynamics are not modelled)
    ("max_upwelling", 1.0e-3), // m/s clamp
    ("ekman_on", 1.0),
];

/// Ekman pumping `w_E = curl(tau / (rho f))` plus coastal upwelling from the offshore Ekman transport across the coast.
/// Coastal term (cell mean): `w_c = -(M . S) / A`, `M = (tau / (rho f)) x up` (Ekman transport, 90 degrees to the right of the stress for
/// `f > 0`, to the left for `f < 0`) and `S` the sum over land edges of `edge_len * n` (`coast_vectors`). It is positive when the transport
/// leaves the coast, i.e. when the stress runs along the coast with the coast on its left (`f > 0`) or right (`f < 0`); the sign comes
/// from the field `coriolis_f`, never from latitude. Downwelling (negative) appears on the opposite coasts.
pub struct Upwelling { params: ParamSet }
impl Default for Upwelling { fn default() -> Self { Self::new() } }
impl Upwelling {
    pub fn new() -> Self { Upwelling { params: make_params(&UP_PARAMS) } }
}
impl Module for Upwelling {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "upwelling".into(), inputs: names(&["wind_stress", "coriolis_f", "ocean_mask"]), outputs: names(&["upwelling", "ekman_pumping", "coastal_upwelling"]), anomaly_hooks: names(&["upwelling"]) }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_set("upwelling", &UP_PARAMS, name, value)?;
        if matches!(name, "rho_water" | "f_floor" | "max_upwelling") && value <= 0.0 {
            return Err(Error::InvalidParam(format!("upwelling: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = |k: &str| self.params.get_or(k, 0.0);
        let ws = fields.data("wind_stress")?;
        if ws.len() != 3 * n || ws.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidParam("upwelling: 'wind_stress' must have 3 finite layers".into()));
        }
        let tau = unpack_vec3(ws);
        let f = field1(fields, "coriolis_f", n)?;
        let ocean = mask_of(fields, n)?;
        let (mut ek, mut co, mut tot) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        if p("upwelling_on") > 0.5 {
            let (rho, ff, wmax) = (p("rho_water"), p("f_floor"), p("max_upwelling"));
            // v = tau / (rho f), zero where |f| is below the floor and off the ocean.
            let v: Vec<Vec3> = (0..n).map(|i| if ocean[i] && f[i].abs() >= ff { scale(tau[i], 1.0 / (rho * f[i])) } else { [0.0; 3] }).collect();
            if p("ekman_on") > 0.5 {
                let c = curl_of_masked(domain, &v, &ocean);
                for i in 0..n {
                    if ocean[i] && f[i].abs() >= ff {
                        ek[i] = c[i].clamp(-wmax, wmax);
                    }
                }
            }
            if p("coastal_on") > 0.5 {
                let cv = coast_vectors(domain, &ocean);
                for i in 0..n {
                    if ocean[i] && f[i].abs() >= ff && cv[i].1 > 0.0 {
                        let m = cross(v[i], domain.frames[i].up);
                        co[i] = (-dot(m, cv[i].0) / domain.mesh.area[i]).clamp(-wmax, wmax);
                    }
                }
            }
            for i in 0..n {
                tot[i] = (ek[i] + co[i]).clamp(-wmax, wmax);
            }
        }
        fields.put("upwelling", 1, tot)?;
        fields.put("ekman_pumping", 1, ek)?;
        fields.put("coastal_upwelling", 1, co)
    }
}

// ============================================================================================ thermohaline
const TH_PARAMS: [(&str, f64); 14] = [
    ("thermohaline_on", 1.0),
    ("s_ref", 35.0),             // psu
    ("sal_relax_s", 3.0e7),      // s: how long the (E - P) anomaly acts on the mixed layer
    ("mixed_depth_m", 50.0),
    ("rho_water", 1025.0),
    ("sal_dev_max", 5.0),        // psu clamp of the raw anomaly (before the mean is removed)
    ("diffusion_steps", 20.0),
    ("rho_ref", 1027.0),         // linear equation of state
    ("t_ref", 283.0),
    ("alpha_t", 0.2),            // kg/m^3 per K
    ("beta_s", 0.78),            // kg/m^3 per psu
    ("dw_density", 1029.0),      // kg/m^3: formation threshold (about T < 273 K at S = 35)
    ("thermohaline_flux_wm2", 30.0),
    ("dw_min_cells", 1.0),
];

/// Salinity proxy and deep-water formation flag (reduced; no overturning).
/// `sigma = s_ref (E - P) sal_relax_s / (rho H)` per ocean cell, clamped to +-`sal_dev_max`, area-mean removed over the whole ocean
/// (freshwater returns through rivers), then `diffusion_steps` conservative diffusion steps on the ocean subgraph (a pseudo-time step of
/// `0.25 min(A / sum w)`), so the area-mean salinity stays `s_ref` and, by the maximum principle, salinity stays within `s_ref +- 2 sal_dev_max`.
/// Density `rho_ref + beta_s (S - 35) - alpha_t (T_sl - t_ref)`. Deep water forms where `rho >= dw_density`. `thermohaline_flux` is
/// `+q` in formation cells and a uniform negative value elsewhere in the ocean with zero area integral. It uses `temperature_sl`
/// (the atmosphere's sea-level temperature) because `sst` is not yet known when the atmosphere stage runs.
pub struct Thermohaline { params: ParamSet }
impl Default for Thermohaline { fn default() -> Self { Self::new() } }
impl Thermohaline {
    pub fn new() -> Self { Thermohaline { params: make_params(&TH_PARAMS) } }
}
impl Module for Thermohaline {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "thermohaline".into(),
            inputs: names(&["precipitation", "evaporation", "temperature_sl", "ocean_mask"]),
            outputs: names(&["salinity", "ocean_density", "deep_water_formation", "thermohaline_flux"]),
            anomaly_hooks: names(&["salinity_source"]),
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_set("thermohaline", &TH_PARAMS, name, value)?;
        if (matches!(name, "sal_relax_s" | "mixed_depth_m" | "rho_water" | "s_ref") && value <= 0.0) || (matches!(name, "sal_dev_max" | "diffusion_steps" | "thermohaline_flux_wm2") && value < 0.0) {
            return Err(Error::InvalidParam(format!("thermohaline: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let p = |k: &str| self.params.get_or(k, 0.0);
        let (pr, ev, t) = (field1(fields, "precipitation", n)?, field1(fields, "evaporation", n)?, field1(fields, "temperature_sl", n)?);
        let ocean = mask_of(fields, n)?;
        let s0 = p("s_ref");
        let dev = p("sal_dev_max");
        let k_sig = s0 * p("sal_relax_s") / (p("rho_water") * p("mixed_depth_m"));
        let mut sal = vec![0.0; n];
        let (mut area_o, mut mean) = (0.0, 0.0);
        for i in 0..n {
            if ocean[i] {
                sal[i] = (k_sig * (ev[i] - pr[i])).clamp(-dev, dev);
                area_o += mesh.area[i];
                mean += mesh.area[i] * sal[i];
            }
        }
        if area_o > 0.0 {
            mean /= area_o;
        }
        for i in 0..n {
            sal[i] = if ocean[i] { s0 + sal[i] - mean } else { 0.0 };
        }
        // Conservative diffusion on ocean-ocean edges.
        let w: Vec<f64> = (0..mesh.adj.len()).map(|k| mesh.edge_len[k] / mesh.edge_dist[k]).collect();
        let on = |i: usize, k: usize| ocean[i] && ocean[mesh.adj[k] as usize];
        let mut theta = f64::INFINITY;
        for i in 0..n {
            if !ocean[i] {
                continue;
            }
            let ws: f64 = (mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize).filter(|&k| on(i, k)).map(|k| w[k]).sum();
            if ws > 0.0 {
                theta = theta.min(0.25 * mesh.area[i] / ws);
            }
        }
        if theta.is_finite() {
            let mut next = sal.clone();
            for _ in 0..(p("diffusion_steps") as usize) {
                for i in 0..n {
                    if !ocean[i] {
                        continue;
                    }
                    let mut s = 0.0;
                    for k in mesh.adj_offsets[i] as usize..mesh.adj_offsets[i + 1] as usize {
                        if on(i, k) {
                            s += w[k] * (sal[mesh.adj[k] as usize] - sal[i]);
                        }
                    }
                    next[i] = sal[i] + theta * s / mesh.area[i];
                }
                std::mem::swap(&mut sal, &mut next);
            }
        }
        // Density, flag, flux.
        let mut rho = vec![0.0; n];
        let (mut flag, mut flux) = (vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            if ocean[i] {
                rho[i] = p("rho_ref") + p("beta_s") * (sal[i] - 35.0) - p("alpha_t") * (t[i] - p("t_ref"));
            }
        }
        if p("thermohaline_on") > 0.5 {
            let (mut af, mut ar) = (0.0, 0.0);
            let mut nf = 0usize;
            for i in 0..n {
                if ocean[i] {
                    if rho[i] >= p("dw_density") {
                        flag[i] = 1.0;
                        af += mesh.area[i];
                        nf += 1;
                    } else {
                        ar += mesh.area[i];
                    }
                }
            }
            if nf as f64 >= p("dw_min_cells") && af > 0.0 && ar > 0.0 {
                let q = p("thermohaline_flux_wm2");
                let neg = -q * af / ar;
                for i in 0..n {
                    if ocean[i] {
                        flux[i] = if flag[i] > 0.5 { q } else { neg };
                    }
                }
            }
        }
        fields.put("salinity", 1, sal)?;
        fields.put("ocean_density", 1, rho)?;
        fields.put("deep_water_formation", 1, flag)?;
        fields.put("thermohaline_flux", 1, flux)
    }
}

// ============================================================================================ sea ice
const SI_PARAMS: [(&str, f64); 7] = [
    ("sea_ice_on", 1.0),
    ("t_freeze_ref", 271.35), // K, -1.8 C at the reference salinity
    ("s_ref", 35.0),
    ("freeze_slope", -0.054), // K per psu
    ("ice_per_k", 0.5),       // m of ice per K below the freezing point
    ("h_max", 5.0),           // m
    ("h_full", 0.5),          // m: thickness at which the cell is fully covered
];

/// Sea ice thickness from the freezing deficit: `h = clamp(ice_per_k (T_f - sst), 0, h_max)`, `T_f = t_freeze_ref + freeze_slope (S - s_ref)`,
/// fraction `min(1, h / h_full)`. It is a diagnostic of how far the SST is below freezing; ice does not yet insulate the ocean or release latent
/// heat (open). The variant `AlbedoModel::with_sea_ice` reads `sea_ice_fraction`, and `wind_stress` reads `ice_fraction` as before.
pub struct SeaIce { params: ParamSet }
impl Default for SeaIce { fn default() -> Self { Self::new() } }
impl SeaIce {
    pub fn new() -> Self { SeaIce { params: make_params(&SI_PARAMS) } }
}
impl Module for SeaIce {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "sea_ice".into(), inputs: names(&["sst", "salinity", "ocean_mask"]), outputs: names(&["sea_ice", "sea_ice_fraction"]), anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_set("sea_ice", &SI_PARAMS, name, value)?;
        if matches!(name, "ice_per_k" | "h_max" | "h_full" | "t_freeze_ref") && value <= 0.0 {
            return Err(Error::InvalidParam(format!("sea_ice: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = |k: &str| self.params.get_or(k, 0.0);
        let (sst, sal) = (field1(fields, "sst", n)?, field1(fields, "salinity", n)?);
        let ocean = mask_of(fields, n)?;
        let (mut h, mut fr) = (vec![0.0; n], vec![0.0; n]);
        if p("sea_ice_on") > 0.5 {
            for i in 0..n {
                if ocean[i] {
                    let tf = p("t_freeze_ref") + p("freeze_slope") * (sal[i] - p("s_ref"));
                    h[i] = (p("ice_per_k") * (tf - sst[i])).clamp(0.0, p("h_max"));
                    fr[i] = (h[i] / p("h_full")).min(1.0);
                }
            }
        }
        fields.put("sea_ice", 1, h)?;
        fields.put("sea_ice_fraction", 1, fr)
    }
}

// ============================================================================================ nutrients
const NU_PARAMS: [(&str, f64); 4] = [
    ("upwelling_ref", 1.0e-5), // m/s: upwelling that gives 1 - 1/e on its own
    ("coast_weight", 0.3),     // equivalent upwelling units added at the coast
    ("coast_length_m", 3.0e5), // e-folding distance of the coastal (shelf) supply
    ("river_input", 0.0),      // placeholder, zero until Part 4b supplies river nutrient input
];

/// Relative nutrient index in [0, 1): `1 - exp(-(max(w, 0)/w_ref + coast_weight exp(-d/L) + river_input))`. Downwelling gives no supply.
pub struct Nutrients { params: ParamSet }
impl Default for Nutrients { fn default() -> Self { Self::new() } }
impl Nutrients {
    pub fn new() -> Self { Nutrients { params: make_params(&NU_PARAMS) } }
}
impl Module for Nutrients {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "nutrients".into(), inputs: names(&["upwelling", "coast_distance", "ocean_mask"]), outputs: names(&["nutrients"]), anomaly_hooks: names(&["life_growth"]) }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_set("nutrients", &NU_PARAMS, name, value)?;
        if value < 0.0 || (matches!(name, "upwelling_ref" | "coast_length_m") && value == 0.0) {
            return Err(Error::InvalidParam(format!("nutrients: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = |k: &str| self.params.get_or(k, 0.0);
        let w = field1(fields, "upwelling", n)?;
        let cd = fields.data("coast_distance")?.to_vec();
        if cd.len() != n {
            return Err(Error::InvalidParam("nutrients: 'coast_distance' must have 1 layer".into()));
        }
        let ocean = mask_of(fields, n)?;
        let out: Vec<f64> = (0..n)
            .map(|i| {
                if !ocean[i] {
                    return 0.0;
                }
                // Coastless basins have an infinite/undefined distance: no coastal supply.
                let coast = if cd[i].is_finite() && cd[i] >= 0.0 { p("coast_weight") * (-cd[i] / p("coast_length_m")).exp() } else { 0.0 };
                1.0 - (-(w[i].max(0.0) / p("upwelling_ref") + coast + p("river_input"))).exp()
            })
            .collect();
        fields.put("nutrients", 1, out)
    }
}

// ============================================================================================ wiring
/// Scheduler A addition: `upwelling`, `thermohaline`, `nutrients` (needs `wind_stress`, `coriolis_f`, `precipitation`, `evaporation` and the ocean mask stack).
pub fn add_ocean_secondary_stage(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(Upwelling::new()))?;
    s.register(Box::new(Thermohaline::new()))?;
    s.register(Box::new(Nutrients::new()))
}

/// Scheduler A with sea ice and the secondary fields: like `add_atmosphere_ocean_pipeline` but the albedo reads `sea_ice_fraction`, so
/// `ice_fraction` (and with it `wind_stress`) sees the sea ice.
pub fn add_atmosphere_ocean_pipeline_full(s: &mut Scheduler) -> Result<()> {
    crate::energy::add_energy_stage_ocean_flux_with(s, true)?;
    crate::pressure::add_atmosphere_stage(s)?;
    crate::moisture::add_moisture_source_stage(s)?;
    crate::transport::add_moisture_precip_stage(s)?;
    crate::ocean::add_ocean_stage(s)?;
    crate::ocean_forcing::add_gyre_forcing_stage(s)?;
    crate::ocean_currents::add_ocean_currents_stage(s)?;
    add_ocean_secondary_stage(s)
}

/// Scheduler B with sea ice and the thermohaline nudge: loop `ocean_climate` = `[ocean_heat, sea_ice, albedo, energy_balance]`
/// (watch `[temperature_sl, temperature, sst]`, relaxation 0.5, tolerance 2e-3, 30 passes). Needs `salinity` and `thermohaline_flux`
/// (scheduler A or the caller), `current`, `ocean_mask`.
pub fn add_ocean_climate_loop_full(s: &mut Scheduler) -> Result<()> {
    use crate::energy::{AlbedoModel, EnergyBalance};
    use crate::ocean_heat::OceanHeat;
    use crate::scheduler::FeedbackLoop;
    s.register(Box::new(crate::insolation::Insolation::new()))?;
    s.register(Box::new(AlbedoModel::with_sea_ice()))?;
    s.register(Box::new(OceanHeat::with_thermohaline()))?;
    s.register(Box::new(SeaIce::new()))?;
    s.register(Box::new(EnergyBalance::with_ocean_heat()))?;
    s.add_feedback_loop(
        FeedbackLoop::new("ocean_climate", &["ocean_heat", "sea_ice", "albedo", "energy_balance"])
            .watch(&["temperature_sl", "temperature", "sst"])
            .relaxation(0.5)
            .tolerance(2e-3)
            .max_iters(30),
    )
}
