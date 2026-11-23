//! Part 4b-3: soil moisture, snow and glacier accumulation versus melt, land sublimation supply, and the runoff that
//! replaces the constant `runoff_coeff` of 4b-2.
//!
//! Modules (names in quotes)
//!   "land_supply" : ocean_mask, soil_moisture_prev, snow_water_equivalent_prev, glacier_ice_prev
//!                   -> land_availability (0..1), soil_evap_cap, sublimation_cap (kg/m^2/s), snow_cover (0..1)
//!                   Runs BEFORE the moisture loop, from the state at the START of the step only.
//!   "moisture_source" (variant `MoistureSource::with_land_state`, in `moisture.rs`) reads those four fields.
//!   "land_water"  : precipitation, evaporation, evaporation_sublimation, temperature, ocean_mask, elevation, flow_dir, *_prev state
//!                   -> soil_moisture, snow_water_equivalent, glacier_ice, snow_age_s (end-of-step state), glacier_mask,
//!                      runoff, snowfall, snowmelt, ice_melt (kg/m^2/s), land_storage_m
//!                   Runs AFTER the moisture loop. `RiversLakes::with_land_runoff` then routes `runoff`.
//!
//! State handling (decision of 4b-3): the state lives in two sets of fields. `*_prev` is the committed state at the start of a step;
//! the unsuffixed fields are the state at the end. Modules only READ `*_prev` and WRITE the end fields, so any module can be rerun
//! (feedback loop passes, retries) without advancing time twice. The driver calls `commit_land_state` once per step
//! (`run_land_cycle` does it). `seed_land_state` creates the first `*_prev` set.
//!
//! Per land cell and step of `step_s` seconds (all depths in m of liquid water equivalent unless stated):
//!   snowfall = sf(T) * P * dt / rho,  rain = P dt / rho - snowfall,  sf = clamp((snow_T + ramp/2 - T) / ramp, 0, 1)   (a ramp)
//!   sublimation (E_sub dt / rho) first from the snow pack, then from glacier ice
//!   melt (degree-day): potential = melt_rate * max(T - melt_T, 0) * dt; snow melts first, then glacier ice
//!   soil: S1 = S_prev + rain + melt; S2 = S1 - E_soil dt / rho; runoff q = max(S2 - capacity, 0); S_end = S2 - q
//!   snow age: counts consecutive time with snow_water_equivalent > snow_min_m; snow above `glacier_threshold_m` becomes glacier ice
//!   once the age reaches `glacier_persist_s` (ice thickness = excess / ice_density_ratio)
//!   glacier flow: thickness moves along `flow_dir`, fraction phi = min(max_frac, coeff * h * slope * dt) per step, computed from the
//!   thickness at the start of the flow stage (order independent), ice VOLUME conserved; ice that would enter an ocean cell melts
//!   at the snout and becomes runoff of the source cell
//! Evaporation is limited at the source (`soil_evap_cap`, `sublimation_cap` are what the stores held at the START of the step), so
//! the ledger cannot go negative. `unsupplied_m3s` reports any evaporation the stores could not supply (0 in the coupled pipeline;
//! nonzero only if someone feeds `land_water` an evaporation field that ignores the caps).
//! Closure: d(storage) = precipitation - evaporation - runoff (+ unsupplied), tested to 1e-8, observed at rounding level.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::{FeedbackLoop, Scheduler};

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

/// Shared parameter table of `land_supply` and `land_water` (both accept every name so one helper sets both).
pub const LAND_PARAMS: [(&str, f64); 15] = [
    ("soil_capacity_m", 0.15),          // bucket depth (m of water)
    ("snow_T", 273.15),                 // K, centre of the rain/snow ramp
    ("snow_ramp_K", 3.0),               // ramp width: all snow below snow_T - 1.5 K, all rain above snow_T + 1.5 K
    ("melt_T", 273.15),                 // K, degree-day threshold
    ("melt_rate_m_per_K_s", 4.6e-8),    // ~4 mm w.e. per K per day
    ("glacier_threshold_m", 0.5),       // m w.e. of snow that may stay as snow; the excess can become ice
    ("glacier_persist_s", 31_557_600.0),// snow must have been present this long (one year) before it converts
    ("snow_min_m", 1.0e-3),             // m w.e.: below this the cell counts as snow free (resets the age)
    ("glacier_flow_coeff", 1.0e-5),     // 1/(m s): phi = coeff * thickness * slope * dt
    ("glacier_flow_max_frac", 0.5),     // cap on the fraction of a cell's ice that moves in one step
    ("glacier_mask_min_m", 0.1),        // m of ice for glacier_mask = 1
    ("ice_density_ratio", 0.917),       // rho_ice / rho_water: ice thickness * ratio = water equivalent
    ("rho_water", 1000.0),
    ("step_s", 2_592_000.0),            // length of one land step; must equal moisture_transport.transport_time_s (use `set_land_step`)
    ("snow_cover_ref_m", 0.01),         // snow_cover = w / (w + ref), w = snow + ice water equivalent (m)
];

/// Validate one parameter of the shared table.
fn check_land_param(module: &str, name: &str, value: f64) -> Result<()> {
    if !LAND_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
        return Err(Error::InvalidParam(format!("{module}: bad parameter '{name}'")));
    }
    let bad = match name {
        "soil_capacity_m" | "snow_ramp_K" | "rho_water" | "step_s" | "snow_cover_ref_m" => value <= 0.0,
        "melt_rate_m_per_K_s" | "glacier_threshold_m" | "glacier_persist_s" | "snow_min_m" | "glacier_flow_coeff" | "glacier_mask_min_m" => value < 0.0,
        "glacier_flow_max_frac" => !(0.0..=1.0).contains(&value),
        "ice_density_ratio" => !(value > 0.0 && value <= 1.0),
        _ => false,
    };
    if bad {
        return Err(Error::InvalidParam(format!("{module}: '{name}' out of range (got {value})")));
    }
    Ok(())
}

fn land_param_set() -> ParamSet {
    let mut ps = ParamSet::new();
    for (k, v) in LAND_PARAMS.iter() {
        ps.set(k, *v);
    }
    ps
}

/// Typed copy of the parameter table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LandParams {
    pub soil_capacity_m: f64,
    pub snow_t: f64,
    pub snow_ramp_k: f64,
    pub melt_t: f64,
    pub melt_rate: f64,
    pub glacier_threshold_m: f64,
    pub glacier_persist_s: f64,
    pub snow_min_m: f64,
    pub glacier_flow_coeff: f64,
    pub glacier_flow_max_frac: f64,
    pub glacier_mask_min_m: f64,
    pub ice_density_ratio: f64,
    pub rho_water: f64,
    pub step_s: f64,
    pub snow_cover_ref_m: f64,
}

impl Default for LandParams {
    fn default() -> Self { Self::from_params(&land_param_set()) }
}

impl LandParams {
    pub fn from_params(ps: &ParamSet) -> Self {
        let g = |k: &str| -> f64 {
            let d = LAND_PARAMS.iter().find(|(n, _)| *n == k).map(|(_, v)| *v).unwrap_or(0.0);
            ps.get_or(k, d)
        };
        LandParams {
            soil_capacity_m: g("soil_capacity_m"),
            snow_t: g("snow_T"),
            snow_ramp_k: g("snow_ramp_K"),
            melt_t: g("melt_T"),
            melt_rate: g("melt_rate_m_per_K_s"),
            glacier_threshold_m: g("glacier_threshold_m"),
            glacier_persist_s: g("glacier_persist_s"),
            snow_min_m: g("snow_min_m"),
            glacier_flow_coeff: g("glacier_flow_coeff"),
            glacier_flow_max_frac: g("glacier_flow_max_frac"),
            glacier_mask_min_m: g("glacier_mask_min_m"),
            ice_density_ratio: g("ice_density_ratio"),
            rho_water: g("rho_water"),
            step_s: g("step_s"),
            snow_cover_ref_m: g("snow_cover_ref_m"),
        }
    }

    /// Read the current values from the registered `land_water` module (defaults if it is not registered).
    pub fn from_scheduler(s: &Scheduler) -> Self {
        let mut ps = land_param_set();
        for (k, _) in LAND_PARAMS.iter() {
            if let Some(v) = s.module_param("land_water", k) {
                ps.set(k, v);
            }
        }
        Self::from_params(&ps)
    }
}

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// Share of precipitation that falls as snow: 1 below `snow_t - ramp/2`, 0 above `snow_t + ramp/2`, linear between (0.5 at `snow_t`).
pub fn snow_fraction(t: f64, snow_t: f64, ramp_k: f64) -> f64 {
    if !(ramp_k > 0.0) {
        return if t < snow_t { 1.0 } else { 0.0 };
    }
    ((snow_t + 0.5 * ramp_k - t) / ramp_k).clamp(0.0, 1.0)
}

/// Forcing of one land cell for one step: precipitation, evaporation and its sublimation part (kg/m^2/s), temperature (K).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellIn {
    pub precip: f64,
    pub evap: f64,
    pub evap_sub: f64,
    pub temp: f64,
}

/// State of one land cell: soil water (m), snow water equivalent (m), glacier ice thickness (m of ice), snow age (s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellState {
    pub soil: f64,
    pub swe: f64,
    pub ice: f64,
    pub age: f64,
}

/// New state and the fluxes of one step (all `*_we` in m of liquid water equivalent over the cell).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellOut {
    pub state: CellState,
    pub runoff_we: f64,
    pub snowfall_we: f64,
    pub rain_we: f64,
    pub melt_snow_we: f64,
    pub melt_ice_we: f64,
    pub sub_we: f64,
    pub soil_evap_we: f64,
    pub unsupplied_we: f64,
}

/// One step of the bucket, snow and glacier ledger for one land cell (no flow between cells; see `glacier_flow`).
pub fn land_cell_step(p: &LandParams, i: &CellIn, s: &CellState) -> CellOut {
    let ts = p.step_s;
    let rho = p.rho_water;
    let r = p.ice_density_ratio;
    let pr = i.precip.max(0.0);
    let e = i.evap.max(0.0);
    let es = i.evap_sub.max(0.0).min(e);
    let esoil = e - es;

    let sf = snow_fraction(i.temp, p.snow_t, p.snow_ramp_k);
    let total_in = pr * ts / rho;
    let snowfall = sf * total_in;
    let rain = total_in - snowfall;

    // sublimation: snow first, then ice
    let ice0 = s.ice * r;
    let sub = es * ts / rho;
    let sub_snow = sub.min(s.swe);
    let sub_ice = (sub - sub_snow).min(ice0);
    let unsup_sub = sub - sub_snow - sub_ice;
    let swe1 = s.swe + snowfall - sub_snow;
    let ice1 = ice0 - sub_ice;

    // degree-day melt: snow first, then ice
    let pm = p.melt_rate * (i.temp - p.melt_t).max(0.0) * ts;
    let melt_snow = pm.min(swe1);
    let swe2 = swe1 - melt_snow;
    let melt_ice = (pm - melt_snow).min(ice1);
    let ice2 = ice1 - melt_ice;

    // soil bucket
    let s1 = s.soil + rain + melt_snow + melt_ice;
    let evs = esoil * ts / rho;
    let evs_eff = evs.min(s1);
    let unsup_soil = evs - evs_eff;
    let s2 = s1 - evs_eff;
    let q = (s2 - p.soil_capacity_m).max(0.0);
    let soil_end = s2 - q;

    // persistence and glacier formation
    let age = if swe2 > p.snow_min_m { s.age + ts } else { 0.0 };
    let (swe3, ice3) = if age >= p.glacier_persist_s && swe2 > p.glacier_threshold_m {
        (p.glacier_threshold_m, ice2 + (swe2 - p.glacier_threshold_m))
    } else {
        (swe2, ice2)
    };

    CellOut {
        state: CellState { soil: soil_end, swe: swe3, ice: ice3 / r, age },
        runoff_we: q,
        snowfall_we: snowfall,
        rain_we: rain,
        melt_snow_we: melt_snow,
        melt_ice_we: melt_ice,
        sub_we: sub_snow + sub_ice,
        soil_evap_we: evs_eff,
        unsupplied_we: unsup_sub + unsup_soil,
    }
}

/// Glacier flow along `flow_dir` (`fd[c]` = receiver or -1). `ice` is thickness in m of ice. Returns (new thickness,
/// water equivalent in m over the SOURCE cell that melted at an ocean snout and must be added to that cell's runoff).
/// Ice volume (thickness * area) is conserved except for the snout melt, which is returned as water.
pub fn glacier_flow(mesh: &Mesh, fd: &[i64], z: &[f64], ocean: &[bool], ice: &[f64], p: &LandParams) -> (Vec<f64>, Vec<f64>) {
    let n = mesh.n_cells;
    let mut out = ice.to_vec();
    let mut snout = vec![0.0; n];
    for c in 0..n {
        if ocean[c] || !(ice[c] > 0.0) || fd[c] < 0 {
            continue;
        }
        let r = fd[c] as usize;
        if r >= n || r == c {
            continue;
        }
        let drop = z[c] - z[r];
        if !(drop > 0.0) {
            continue;
        }
        let dist = match mesh.neighbors(c).iter().position(|&j| j as usize == r) {
            Some(k) => mesh.edge_dists(c)[k],
            None => continue,
        };
        let slope = drop / dist;
        let phi = (p.glacier_flow_coeff * ice[c] * slope * p.step_s).min(p.glacier_flow_max_frac).max(0.0);
        let moved = phi * ice[c];
        out[c] -= moved;
        if ocean[r] {
            snout[c] += moved * p.ice_density_ratio;
        } else {
            out[r] += moved * mesh.area[c] / mesh.area[r];
        }
    }
    (out, snout)
}

// ---------------------------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------------------------

/// Names of the committed (start of step) state fields, paired with the end-of-step fields `land_water` writes.
pub const STATE_PAIRS: [(&str, &str); 4] = [
    ("soil_moisture", "soil_moisture_prev"),
    ("snow_water_equivalent", "snow_water_equivalent_prev"),
    ("glacier_ice", "glacier_ice_prev"),
    ("snow_age_s", "snow_age_s_prev"),
];

/// Create the first committed state: `soil_m` metres of water in every cell's bucket, no snow, no ice, snow age 0.
pub fn seed_land_state(fields: &mut FieldStore, soil_m: f64) -> Result<()> {
    let n = fields.n_cells();
    fields.put("soil_moisture_prev", 1, vec![soil_m.max(0.0); n])?;
    fields.put("snow_water_equivalent_prev", 1, vec![0.0; n])?;
    fields.put("glacier_ice_prev", 1, vec![0.0; n])?;
    fields.put("snow_age_s_prev", 1, vec![0.0; n])
}

/// Copy the end-of-step state into `*_prev`: this is what advances land time by one step. Call once per pipeline run.
pub fn commit_land_state(fields: &mut FieldStore) -> Result<()> {
    for (end, prev) in STATE_PAIRS.iter() {
        let d = fields.data(end)?.to_vec();
        fields.put(prev, 1, d)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Module: land_supply
// ---------------------------------------------------------------------------------------------

/// Start-of-step supply fields for the moisture source. Parameters: the shared land table (`LAND_PARAMS`).
pub struct LandSupply {
    params: ParamSet,
}

impl Default for LandSupply {
    fn default() -> Self { Self::new() }
}

impl LandSupply {
    pub fn new() -> Self { LandSupply { params: land_param_set() } }
}

impl Module for LandSupply {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "land_supply".into(),
            inputs: names(&["ocean_mask", "soil_moisture_prev", "snow_water_equivalent_prev", "glacier_ice_prev"]),
            outputs: names(&["land_availability", "soil_evap_cap", "sublimation_cap", "snow_cover"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_land_param("land_supply", name, value)?;
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = LandParams::from_params(&self.params);
        let ocean = fields.data("ocean_mask")?.to_vec();
        let soil = fields.data("soil_moisture_prev")?.to_vec();
        let swe = fields.data("snow_water_equivalent_prev")?.to_vec();
        let ice = fields.data("glacier_ice_prev")?.to_vec();
        if [ocean.len(), soil.len(), swe.len(), ice.len()].iter().any(|&l| l != n) {
            return Err(Error::InvalidParam("land_supply: every input field must have one value per cell".into()));
        }
        // Part 5b: optional vegetation factor on the evaporative demand of land (field `veg_evap_factor`, >= 0). Absent -> 1.
        // It scales the availability only; the stores (`soil_evap_cap`, `sublimation_cap`) still cap what can leave, so water stays conserved.
        let veg: Option<Vec<f64>> = if fields.has("veg_evap_factor") {
            let v = fields.data("veg_evap_factor")?.to_vec();
            if v.len() != n {
                return Err(Error::InvalidParam("land_supply: 'veg_evap_factor' must have one value per cell".into()));
            }
            Some(v)
        } else {
            None
        };
        let mut avail = vec![1.0; n];
        let mut soil_cap = vec![0.0; n];
        let mut sub_cap = vec![0.0; n];
        let mut cover = vec![0.0; n];
        for c in 0..n {
            if ocean[c] > 0.5 {
                continue;
            }
            let s = soil[c].max(0.0);
            let w = swe[c].max(0.0) + p.ice_density_ratio * ice[c].max(0.0);
            let vf = match &veg {
                Some(v) if v[c].is_finite() && v[c] >= 0.0 => v[c],
                _ => 1.0,
            };
            avail[c] = (vf * s / p.soil_capacity_m).clamp(0.0, 1.0);
            soil_cap[c] = p.rho_water * s / p.step_s;
            sub_cap[c] = p.rho_water * w / p.step_s;
            cover[c] = w / (w + p.snow_cover_ref_m);
        }
        fields.put("land_availability", 1, avail)?;
        fields.put("soil_evap_cap", 1, soil_cap)?;
        fields.put("sublimation_cap", 1, sub_cap)?;
        fields.put("snow_cover", 1, cover)
    }
}

// ---------------------------------------------------------------------------------------------
// Module: land_water
// ---------------------------------------------------------------------------------------------

const LW_OUTPUT_ONLY: [&str; 12] = [
    "precip_land_m3s",
    "evap_land_m3s",
    "runoff_m3s",
    "storage_change_m3s",
    "unsupplied_m3s",
    "balance_residual",
    "snowfall_m3s",
    "melt_m3s",
    "n_glacier_cells",
    "glacier_ice_volume_m3",
    "snow_area_m2",
    "soil_fraction_mean",
];

/// The land ledger. Parameters: the shared land table. Output-only (set by `run`; m^3/s of liquid water unless noted):
/// `precip_land_m3s`, `evap_land_m3s`, `runoff_m3s`, `storage_change_m3s`, `unsupplied_m3s`, `balance_residual` (relative,
/// `storage_change - (precip - evap - runoff + unsupplied)` over `max(precip, evap)`), `snowfall_m3s`, `melt_m3s` (snow and ice),
/// `n_glacier_cells`, `glacier_ice_volume_m3` (volume of ice), `snow_area_m2` (cells with snow), `soil_fraction_mean` (area mean of
/// soil_moisture / capacity over land).
pub struct LandWater {
    params: ParamSet,
}

impl Default for LandWater {
    fn default() -> Self { Self::new() }
}

impl LandWater {
    pub fn new() -> Self {
        let mut params = land_param_set();
        for k in LW_OUTPUT_ONLY.iter() {
            params.set(k, 0.0);
        }
        LandWater { params }
    }
}

impl Module for LandWater {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "land_water".into(),
            inputs: names(&[
                "precipitation",
                "evaporation",
                "evaporation_sublimation",
                "temperature",
                "ocean_mask",
                "elevation",
                "flow_dir",
                "soil_moisture_prev",
                "snow_water_equivalent_prev",
                "glacier_ice_prev",
                "snow_age_s_prev",
            ]),
            outputs: names(&[
                "soil_moisture",
                "snow_water_equivalent",
                "glacier_ice",
                "snow_age_s",
                "glacier_mask",
                "runoff",
                "snowfall",
                "snowmelt",
                "ice_melt",
                "land_storage_m",
            ]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        check_land_param("land_water", name, value)?;
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = LandParams::from_params(&self.params);
        let area = &domain.mesh.area;
        let precip = fields.data("precipitation")?.to_vec();
        let evap = fields.data("evaporation")?.to_vec();
        let evsub = fields.data("evaporation_sublimation")?.to_vec();
        let temp = fields.data("temperature")?.to_vec();
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        let z = fields.data("elevation")?.to_vec();
        let fd: Vec<i64> = fields.data("flow_dir")?.iter().map(|&v| v as i64).collect();
        let soil0 = fields.data("soil_moisture_prev")?.to_vec();
        let swe0 = fields.data("snow_water_equivalent_prev")?.to_vec();
        let ice0 = fields.data("glacier_ice_prev")?.to_vec();
        let age0 = fields.data("snow_age_s_prev")?.to_vec();
        let lens = [precip.len(), evap.len(), evsub.len(), temp.len(), ocean.len(), z.len(), fd.len(), soil0.len(), swe0.len(), ice0.len(), age0.len()];
        if lens.iter().any(|&l| l != n) {
            return Err(Error::InvalidParam("land_water: every input field must have one value per cell".into()));
        }
        for (name, v) in [("precipitation", &precip), ("evaporation", &evap), ("evaporation_sublimation", &evsub), ("temperature", &temp)] {
            if v.iter().any(|x| !x.is_finite()) {
                return Err(Error::NonFinite(name.into()));
            }
        }
        for (name, v) in [("soil_moisture_prev", &soil0), ("snow_water_equivalent_prev", &swe0), ("glacier_ice_prev", &ice0), ("snow_age_s_prev", &age0)] {
            if v.iter().any(|x| !x.is_finite()) {
                return Err(Error::NonFinite(name.into()));
            }
        }
        if fd.iter().any(|&r| r >= n as i64) {
            return Err(Error::InvalidParam("land_water: flow_dir out of range".into()));
        }

        let ts = p.step_s;
        let rho = p.rho_water;
        let r = p.ice_density_ratio;
        let mut soil = vec![0.0; n];
        let mut swe = vec![0.0; n];
        let mut ice = vec![0.0; n];
        let mut age = vec![0.0; n];
        let mut runoff_we = vec![0.0; n];
        let mut snowfall = vec![0.0; n];
        let mut snowmelt = vec![0.0; n];
        let mut icemelt = vec![0.0; n];
        let mut unsup = vec![0.0; n];
        for c in 0..n {
            if ocean[c] {
                continue;
            }
            let o = land_cell_step(
                &p,
                &CellIn { precip: precip[c], evap: evap[c], evap_sub: evsub[c], temp: temp[c] },
                &CellState { soil: soil0[c].max(0.0), swe: swe0[c].max(0.0), ice: ice0[c].max(0.0), age: age0[c].max(0.0) },
            );
            soil[c] = o.state.soil;
            swe[c] = o.state.swe;
            ice[c] = o.state.ice;
            age[c] = o.state.age;
            runoff_we[c] = o.runoff_we;
            snowfall[c] = o.snowfall_we * rho / ts;
            snowmelt[c] = o.melt_snow_we * rho / ts;
            icemelt[c] = o.melt_ice_we * rho / ts;
            unsup[c] = o.unsupplied_we;
        }
        let (ice_flowed, snout) = glacier_flow(&domain.mesh, &fd, &z, &ocean, &ice, &p);
        let mut runoff = vec![0.0; n];
        let mut storage = vec![0.0; n];
        let mut mask = vec![0.0; n];
        let mut diag = [0.0f64; 12];
        let (mut n_gl, mut ice_vol, mut snow_area, mut soil_frac_sum, mut land_area) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for c in 0..n {
            if ocean[c] {
                continue;
            }
            ice[c] = ice_flowed[c].max(0.0);
            runoff[c] = rho * (runoff_we[c] + snout[c]) / ts;
            storage[c] = soil[c] + swe[c] + r * ice[c];
            if ice[c] > p.glacier_mask_min_m {
                mask[c] = 1.0;
                n_gl += 1.0;
            }
            let a = area[c];
            diag[0] += pr_m3s(precip[c], a, rho);
            diag[1] += pr_m3s(evap[c], a, rho);
            diag[2] += runoff[c] * a / rho;
            diag[3] += a * (storage[c] - (soil0[c].max(0.0) + swe0[c].max(0.0) + r * ice0[c].max(0.0))) / ts;
            diag[4] += a * unsup[c] / ts;
            diag[6] += snowfall[c] * a / rho;
            diag[7] += (snowmelt[c] + icemelt[c]) * a / rho;
            ice_vol += a * ice[c];
            if swe[c] > p.snow_min_m {
                snow_area += a;
            }
            soil_frac_sum += a * soil[c] / p.soil_capacity_m;
            land_area += a;
        }
        diag[5] = if diag[0].max(diag[1]) > 0.0 { (diag[3] - (diag[0] - diag[1] - diag[2] + diag[4])) / diag[0].max(diag[1]) } else { 0.0 };

        let ps = &mut self.params;
        ps.set("precip_land_m3s", diag[0]);
        ps.set("evap_land_m3s", diag[1]);
        ps.set("runoff_m3s", diag[2]);
        ps.set("storage_change_m3s", diag[3]);
        ps.set("unsupplied_m3s", diag[4]);
        ps.set("balance_residual", diag[5]);
        ps.set("snowfall_m3s", diag[6]);
        ps.set("melt_m3s", diag[7]);
        ps.set("n_glacier_cells", n_gl);
        ps.set("glacier_ice_volume_m3", ice_vol);
        ps.set("snow_area_m2", snow_area);
        ps.set("soil_fraction_mean", if land_area > 0.0 { soil_frac_sum / land_area } else { 0.0 });

        fields.put("soil_moisture", 1, soil)?;
        fields.put("snow_water_equivalent", 1, swe)?;
        fields.put("glacier_ice", 1, ice)?;
        fields.put("snow_age_s", 1, age)?;
        fields.put("glacier_mask", 1, mask)?;
        fields.put("runoff", 1, runoff)?;
        fields.put("snowfall", 1, snowfall)?;
        fields.put("snowmelt", 1, snowmelt)?;
        fields.put("ice_melt", 1, icemelt)?;
        fields.put("land_storage_m", 1, storage)
    }
}

/// kg/m^2/s over an area to m^3/s of liquid water.
fn pr_m3s(rate: f64, area: f64, rho: f64) -> f64 { rate.max(0.0) * area / rho }

// ---------------------------------------------------------------------------------------------
// Budget, wiring, driver
// ---------------------------------------------------------------------------------------------

/// Land water budget of one step, area integrated, in m^3 of liquid water. Recomputed from the stored fields (independent of the
/// module's own bookkeeping): `storage_end - storage_start = precipitation - evaporation - runoff` on land cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LandBudget {
    pub precip_m3: f64,
    pub evap_m3: f64,
    pub runoff_m3: f64,
    pub storage_start_m3: f64,
    pub storage_end_m3: f64,
    /// `(storage_end - storage_start) - (precip - evap - runoff)`.
    pub residual_m3: f64,
    /// `|residual| / max(precip, evap, tiny)`.
    pub residual_rel: f64,
}

impl LandBudget {
    /// Needs `ocean_mask`, `precipitation`, `evaporation`, `runoff` and both state sets (`*_prev` and end). Call BEFORE `commit_land_state`.
    pub fn from_fields(domain: &Domain, fields: &FieldStore, p: &LandParams) -> Result<Self> {
        let area = &domain.mesh.area;
        let ocean = fields.data("ocean_mask")?;
        let pr = fields.data("precipitation")?;
        let ev = fields.data("evaporation")?;
        let ro = fields.data("runoff")?;
        let (s0, w0, i0) = (fields.data("soil_moisture_prev")?, fields.data("snow_water_equivalent_prev")?, fields.data("glacier_ice_prev")?);
        let (s1, w1, i1) = (fields.data("soil_moisture")?, fields.data("snow_water_equivalent")?, fields.data("glacier_ice")?);
        let (ts, rho, r) = (p.step_s, p.rho_water, p.ice_density_ratio);
        let (mut pm, mut em, mut rm, mut st0, mut st1) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for c in 0..domain.n_cells() {
            if ocean[c] > 0.5 {
                continue;
            }
            let a = area[c];
            pm += pr[c].max(0.0) * a * ts / rho;
            em += ev[c].max(0.0) * a * ts / rho;
            rm += ro[c] * a * ts / rho;
            st0 += a * (s0[c].max(0.0) + w0[c].max(0.0) + r * i0[c].max(0.0));
            st1 += a * (s1[c] + w1[c] + r * i1[c]);
        }
        let residual = (st1 - st0) - (pm - em - rm);
        Ok(LandBudget {
            precip_m3: pm,
            evap_m3: em,
            runoff_m3: rm,
            storage_start_m3: st0,
            storage_end_m3: st1,
            residual_m3: residual,
            residual_rel: residual.abs() / pm.max(em).max(1e-300),
        })
    }
}

/// Set a land parameter on both `land_supply` and `land_water` (they share one table).
pub fn set_land_param(s: &mut Scheduler, name: &str, value: f64) -> Result<()> {
    s.set_param("land_supply", name, value)?;
    s.set_param("land_water", name, value)
}

/// Set the step length of the whole water cycle: `moisture_transport.transport_time_s` and `step_s` of both land modules.
pub fn set_land_step(s: &mut Scheduler, step_s: f64) -> Result<()> {
    s.set_param("moisture_transport", "transport_time_s", step_s)?;
    set_land_param(s, "step_s", step_s)
}

/// Registers only `land_supply` and `land_water`.
pub fn add_land_water_modules(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(LandSupply::new()))?;
    s.register(Box::new(LandWater::new()))
}

/// The full water-cycle pipeline of 4b-3: energy loop, pressure and wind, `sea_level` + `ocean_mask`, `flow_routing`, `land_supply`,
/// the land-state `moisture_source`, `moisture_transport` (with precipitation, `warm_start` 1), the loop `water_cycle`
/// (`moisture_source` + `moisture_transport`, watching `vapor` and `precipitation`, relaxation 1, tolerance 1e-4, at most 60 passes),
/// `land_water` and the land-runoff variant of `rivers_lakes`. Needs `elevation` plus the seeded `temperature` (`seed_temperature`)
/// and the seeded land state (`seed_land_state`). Drive it with `run_land_cycle`.
pub fn add_land_water_pipeline(s: &mut Scheduler) -> Result<()> {
    crate::energy::add_energy_stage(s)?;
    crate::pressure::add_atmosphere_stage(s)?;
    crate::ocean::add_ocean_mask_stage(s)?;
    crate::flow_routing::add_flow_routing_module(s)?;
    s.register(Box::new(LandSupply::new()))?;
    crate::moisture::add_moisture_source_land_stage(s)?;
    crate::transport::add_moisture_precip_stage(s)?;
    s.register(Box::new(LandWater::new()))?;
    s.register(Box::new(crate::rivers_lakes::RiversLakes::with_land_runoff()))?;
    s.set_param("moisture_transport", "warm_start", 1.0)?;
    s.add_feedback_loop(
        FeedbackLoop::new("water_cycle", &["moisture_source", "moisture_transport"])
            .watch(&["vapor", "precipitation"])
            .relaxation(1.0)
            .tolerance(1e-4)
            .max_iters(60),
    )
}

/// What one land step of `run_land_cycle` reports.
#[derive(Debug, Clone, Copy)]
pub struct LandSeason {
    pub season: f64,
    pub climate: crate::budget::SeasonReport,
    pub land: LandBudget,
    /// Passes the `water_cycle` loop needed and whether it converged.
    pub water_cycle_passes: usize,
    pub water_cycle_converged: bool,
}

/// Run the pipeline once per entry of `seasons` (fractions of the orbit), compute the land budget of each step and commit the land
/// state after it. Each step lasts `step_s` (set with `set_land_step`); with 12 steps of 30 d the seasons cover about one year.
pub fn run_land_cycle(domain: &Domain, fields: &mut FieldStore, scheduler: &mut Scheduler, seasons: &[f64]) -> Result<Vec<LandSeason>> {
    let mut out = Vec::with_capacity(seasons.len());
    for &season in seasons {
        scheduler.set_param("insolation", "season", season)?;
        let report = scheduler.run(domain, fields, &DirtyRegion::All)?;
        let t = scheduler.module_param("moisture_transport", "transport_time_s").unwrap_or(0.0);
        let passes_energy = report.loops.iter().find(|l| l.name == "ice_albedo").map(|l| l.iterations).unwrap_or(0);
        let wc = report.loops.iter().find(|l| l.name == "water_cycle");
        let climate = crate::budget::SeasonReport {
            season,
            budget: crate::budget::WaterBudget::from_fields(domain, fields, t)?,
            mean_precip_mm_day: crate::budget::mean_precip_mm_day(domain, fields)?,
            mean_evap_mm_day: crate::budget::mean_evap_mm_day(domain, fields)?,
            passes_energy,
        };
        let lp = LandParams::from_scheduler(scheduler);
        let land = LandBudget::from_fields(domain, fields, &lp)?;
        commit_land_state(fields)?;
        out.push(LandSeason {
            season,
            climate,
            land,
            water_cycle_passes: wc.map(|l| l.iterations).unwrap_or(0),
            water_cycle_converged: wc.map(|l| l.converged).unwrap_or(false),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_fraction_ramp() {
        assert_eq!(snow_fraction(260.0, 273.15, 3.0), 1.0);
        assert_eq!(snow_fraction(280.0, 273.15, 3.0), 0.0);
        assert!((snow_fraction(273.15, 273.15, 3.0) - 0.5).abs() < 1e-15);
        assert!((snow_fraction(273.15 - 0.75, 273.15, 3.0) - 0.75).abs() < 1e-12);
        assert_eq!(snow_fraction(272.0, 273.15, 0.0), 1.0);
    }

    #[test]
    fn defaults_are_the_spec_values() {
        let p = LandParams::default();
        assert_eq!(p.soil_capacity_m, 0.15);
        assert_eq!(p.snow_t, 273.15);
        assert_eq!(p.snow_ramp_k, 3.0);
    }
}
