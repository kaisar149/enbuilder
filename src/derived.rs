//! Part 5c (stage E, optional): derived land maps `soil_fertility`, `habitability`, `travel_difficulty`.
//!
//! NOT COMPILED when written. `reference/mirror_5c.py` holds the formulas and the hand numbers of `tests/part5c.rs`.
//! All three are reduced-order scores built from the climate normals (`clim_*`), the biome canopy, the terrain slope and the masks.
//! Ocean cells are 0 in all three. They are outputs only (no feedback), so they are cheap to recompute after any edit.
//!
//!   ramp(x, lo, hi)      = clamp((x - lo) / (hi - lo), 0, 1)                       (NaN -> 0)
//!   soil_fertility       = ramp(aridity, a_lo, a_hi) * ramp(gdd, g_lo, g_hi) * (0.5 + 0.5 ramp(soil_m, 0, soil_ref_m))
//!   habitability         = min(ramp(t_cold, -35, -5), 1 - ramp(t_warm, 32, 45)) * ramp(aridity, f_lo, f_hi) * (0.4 + 0.6 fertility)
//!   travel_difficulty    = 1 + min(slope_gain slope, 4) + min(canopy / canopy_ref_m, 1) + ramp(-t_cold, 10, 30)
//!                            + 0.5 ramp(t_warm, 35, 45) + 2 glacier             (>= 1 on land; a multiplier on travel cost)
//! `slope` is the mean of |dz| / distance over the neighbours of the cell (rise over run, dimensionless; the sea floor counts as sea level).
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DerivedParams {
    pub aridity_lo: f64,
    pub aridity_hi: f64,
    pub gdd_lo: f64,
    pub gdd_hi: f64,
    pub soil_ref_m: f64,
    pub fresh_lo: f64,
    pub fresh_hi: f64,
    pub slope_gain: f64,
    pub canopy_ref_m: f64,
    pub sea_level: f64,
    /// Part 7: soil fertility boost from lightning-fixed nitrogen, `fert * (1 + nitrogen_gain * soil_nitrogen)`. 0 = off (default, bit-exact baseline).
    pub nitrogen_gain: f64,
}

impl Default for DerivedParams {
    fn default() -> Self {
        DerivedParams { aridity_lo: 0.1, aridity_hi: 1.0, gdd_lo: 300.0, gdd_hi: 2500.0, soil_ref_m: 0.1, fresh_lo: 0.05, fresh_hi: 0.4, slope_gain: 4.0, canopy_ref_m: 30.0, sea_level: 0.0, nitrogen_gain: 0.0 }
    }
}

const PARAM_NAMES: [&str; 11] =
    ["aridity_lo", "aridity_hi", "gdd_lo", "gdd_hi", "soil_ref_m", "fresh_lo", "fresh_hi", "slope_gain", "canopy_ref_m", "sea_level", "nitrogen_gain"];

impl DerivedParams {
    fn to_set(self) -> ParamSet {
        let v = [self.aridity_lo, self.aridity_hi, self.gdd_lo, self.gdd_hi, self.soil_ref_m, self.fresh_lo, self.fresh_hi, self.slope_gain, self.canopy_ref_m, self.sea_level, self.nitrogen_gain];
        let mut s = ParamSet::new();
        for (k, x) in PARAM_NAMES.iter().zip(v) {
            s.set(k, x);
        }
        s
    }
    fn from_set(s: &ParamSet) -> Self {
        let d = DerivedParams::default();
        DerivedParams {
            aridity_lo: s.get_or("aridity_lo", d.aridity_lo),
            aridity_hi: s.get_or("aridity_hi", d.aridity_hi),
            gdd_lo: s.get_or("gdd_lo", d.gdd_lo),
            gdd_hi: s.get_or("gdd_hi", d.gdd_hi),
            soil_ref_m: s.get_or("soil_ref_m", d.soil_ref_m),
            fresh_lo: s.get_or("fresh_lo", d.fresh_lo),
            fresh_hi: s.get_or("fresh_hi", d.fresh_hi),
            slope_gain: s.get_or("slope_gain", d.slope_gain),
            canopy_ref_m: s.get_or("canopy_ref_m", d.canopy_ref_m),
            sea_level: s.get_or("sea_level", d.sea_level),
            nitrogen_gain: s.get_or("nitrogen_gain", d.nitrogen_gain),
        }
    }
}

pub fn ramp(x: f64, lo: f64, hi: f64) -> f64 {
    if x.is_nan() {
        return 0.0;
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

pub fn soil_fertility(p: &DerivedParams, aridity: f64, gdd: f64, soil_m: f64) -> f64 {
    ramp(aridity, p.aridity_lo, p.aridity_hi) * ramp(gdd, p.gdd_lo, p.gdd_hi) * (0.5 + 0.5 * ramp(soil_m, 0.0, p.soil_ref_m))
}

pub fn habitability(p: &DerivedParams, fertility: f64, t_cold_c: f64, t_warm_c: f64, aridity: f64) -> f64 {
    let comfort = ramp(t_cold_c, -35.0, -5.0).min(1.0 - ramp(t_warm_c, 32.0, 45.0));
    comfort * ramp(aridity, p.fresh_lo, p.fresh_hi) * (0.4 + 0.6 * fertility)
}

pub fn travel_difficulty(p: &DerivedParams, slope: f64, canopy_m: f64, t_cold_c: f64, t_warm_c: f64, glacier: bool) -> f64 {
    1.0 + (p.slope_gain * slope).min(4.0)
        + (canopy_m / p.canopy_ref_m).clamp(0.0, 1.0)
        + ramp(-t_cold_c, 10.0, 30.0)
        + 0.5 * ramp(t_warm_c, 35.0, 45.0)
        + if glacier { 2.0 } else { 0.0 }
}

/// Mean rise over run to the neighbours of every cell (`z` clipped at `sea`).
pub fn mean_slope(domain: &Domain, z: &[f64], sea: f64) -> Vec<f64> {
    let m = &domain.mesh;
    (0..domain.n_cells())
        .map(|i| {
            let (nb, dist) = (m.neighbors(i), m.edge_dists(i));
            if nb.is_empty() {
                return 0.0;
            }
            let zi = z[i].max(sea);
            nb.iter().zip(dist).map(|(&j, &d)| (z[j as usize].max(sea) - zi).abs() / d.max(1e-9)).sum::<f64>() / nb.len() as f64
        })
        .collect()
}

pub const DERIVED_INPUTS: [&str; 9] =
    ["clim_t_cold_c", "clim_t_warm_c", "clim_gdd", "clim_aridity", "clim_soil_m", "ocean_mask", "glacier_mask", "biome_roughness_m", "elevation"];

/// Module `derived_maps`. Inputs `DERIVED_INPUTS` (the biome stage supplies all but `elevation`); outputs `soil_fertility`, `habitability`,
/// `travel_difficulty` (one layer). Parameters: see `DerivedParams` (all must be finite; `lo < hi` pairs, positive references).
pub struct DerivedMaps {
    params: ParamSet,
}

impl Default for DerivedMaps {
    fn default() -> Self { Self::new() }
}

impl DerivedMaps {
    pub fn new() -> Self { DerivedMaps { params: DerivedParams::default().to_set() } }
}

impl Module for DerivedMaps {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "derived_maps".into(),
            inputs: {
                // Part 7: with `nitrogen_gain` > 0 the storm module's `soil_nitrogen` becomes a real input (so the scheduler orders storm first)
                let mut v = names(&DERIVED_INPUTS);
                if DerivedParams::from_set(&self.params).nitrogen_gain > 0.0 {
                    v.push("soil_nitrogen".into());
                }
                v
            },
            outputs: names(&["soil_fertility", "habitability", "travel_difficulty"]),
            anomaly_hooks: vec!["life_growth".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAM_NAMES.contains(&name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("derived_maps: bad parameter '{name}'")));
        }
        let mut next = self.params.clone();
        next.set(name, value);
        let p = DerivedParams::from_set(&next);
        let ok = p.aridity_lo < p.aridity_hi && p.gdd_lo < p.gdd_hi && p.fresh_lo < p.fresh_hi && p.soil_ref_m > 0.0 && p.canopy_ref_m > 0.0 && p.slope_gain >= 0.0 && p.nitrogen_gain >= 0.0;
        if !ok {
            return Err(Error::InvalidParam(format!("derived_maps.{name}: value breaks lo < hi, a positive reference or a non-negative gain")));
        }
        self.params = next;
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let p = DerivedParams::from_set(&self.params);
        let get = |name: &str| -> Result<Vec<f64>> {
            let l = fields.layers(name)?;
            let d = fields.data(name)?;
            if l == 0 || d.len() != n * l {
                return Err(Error::InvalidParam(format!("derived_maps: field '{name}' must have one value per cell and layer")));
            }
            Ok((0..n).map(|i| d[i * l]).collect())
        };
        let (tc, tw, gdd, ar, soil) = (get("clim_t_cold_c")?, get("clim_t_warm_c")?, get("clim_gdd")?, get("clim_aridity")?, get("clim_soil_m")?);
        let (ocean, glacier, canopy, z) = (get("ocean_mask")?, get("glacier_mask")?, get("biome_roughness_m")?, get("elevation")?);
        let mut growth = vec![1.0; n];
        let grow = crate::anomaly::apply_modifier(fields, "life_growth", &mut growth)?;
        let nitrogen = if p.nitrogen_gain > 0.0 { Some(get("soil_nitrogen")?) } else { None };
        let slope = mean_slope(domain, &z, p.sea_level);
        let (mut fert, mut hab, mut trav) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            if ocean[i] > 0.5 {
                continue;
            }
            let ice = glacier[i] > 0.5;
            fert[i] = if ice { 0.0 } else { soil_fertility(&p, ar[i], gdd[i], soil[i]) };
            if let Some(nf) = &nitrogen {
                fert[i] = (fert[i] * (1.0 + p.nitrogen_gain * nf[i].clamp(0.0, 1.0))).clamp(0.0, 1.0); // Part 7: lightning-fixed nitrogen
            }
            if grow {
                fert[i] = (fert[i] * growth[i].max(0.0)).clamp(0.0, 1.0); // Part 7: hook `life_growth`
            }
            hab[i] = if ice { 0.0 } else { habitability(&p, fert[i], tc[i], tw[i], ar[i]) };
            trav[i] = travel_difficulty(&p, slope[i], canopy[i], tc[i], tw[i], ice);
        }
        fields.put("soil_fertility", 1, fert)?;
        fields.put("habitability", 1, hab)?;
        fields.put("travel_difficulty", 1, trav)
    }
}

/// Registers `derived_maps` on `s` (after `add_biome_stage`; `elevation` is the one external input).
pub fn add_derived_stage(s: &mut Scheduler) -> Result<()> { s.register(Box::new(DerivedMaps::new())) }
