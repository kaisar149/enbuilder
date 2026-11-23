//! Part 5b (stage E): the vegetation feedback loop.
//!
//! NOT COMPILED when written (no Rust toolchain in the sandbox). `reference/mirror_5b.py` mirrors the loop arithmetic and
//! produced the hand numbers of `tests/part5b.rs`.
//!
//! The biome of a cell changes how the surface behaves, and the surface changes the climate that picks the biome:
//!   * `veg_albedo`        -> `albedo` (land surface albedo, read by `AlbedoModel` when the field exists)
//!   * `veg_evap_factor`   -> `land_supply` (scales the land availability of the moisture source)
//!   * `veg_roughness_m`   -> `wind` / `wind_stepped` (canopy friction)
//! These three fields are the *relaxed* surface state. They are the only coupling, so with the fields absent every older module behaves
//! exactly as before.
//!
//! The loop is an OUTER loop (a year of seasons per pass, because the biome needs annual statistics, not one season):
//!   1. `climate(domain, fields)` runs one climate year (seasonal fields `season_*` are produced);
//!   2. `classify_biomes` turns them into `biome`, `biome_albedo`, `biome_evap_factor`, `biome_roughness_m`;
//!   3. on LAND cells `veg_x <- veg_x + r_k (biome_x - veg_x)` with `r_k = relax * relax_decay^k`; ocean cells keep their value;
//!   4. stop when the largest change of every field is under its tolerance.
//! The geometric decay of `r_k` is what guarantees convergence even where a cell sits on a biome threshold and flips every pass: the
//! step is at most `r_k * |target - state|`, which is summable. The start state is the "no vegetation" baseline (`baseline_*`), so pass 0
//! is the bare-land climate.
use crate::biomes::{classify_biomes, run_climate_seasons, BiomeSummary, BiomeTable};
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::scheduler::Scheduler;

/// The three relaxed surface fields the climate modules read.
pub const VEG_FIELDS: [&str; 3] = ["veg_albedo", "veg_evap_factor", "veg_roughness_m"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegetationParams {
    /// Blend of the new biome effect into the surface state on pass 0 (0.3-0.5 is the master-prompt range; default 0.5).
    pub relax: f64,
    /// Per-pass multiplier of `relax` (default 0.7; 1 = constant relaxation, no convergence guarantee on flip-flopping cells).
    pub relax_decay: f64,
    pub max_iters: usize,
    /// Convergence: largest land change per pass of `veg_albedo` (default 0.005), `veg_evap_factor` (0.02), `veg_roughness_m` (0.5 m).
    pub tol_albedo: f64,
    pub tol_evap: f64,
    pub tol_rough_m: f64,
    /// The bare-land state of pass 0 (defaults: the `albedo` module's `albedo_land` 0.22, factor 1, no canopy).
    pub baseline_albedo: f64,
    pub baseline_evap: f64,
    pub baseline_rough_m: f64,
}

impl Default for VegetationParams {
    fn default() -> Self {
        VegetationParams {
            relax: 0.5,
            relax_decay: 0.7,
            max_iters: 12,
            tol_albedo: 0.005,
            tol_evap: 0.02,
            tol_rough_m: 0.5,
            baseline_albedo: 0.22,
            baseline_evap: 1.0,
            baseline_rough_m: 0.0,
        }
    }
}

impl VegetationParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParam(format!("vegetation: {m}")));
        if !(self.relax > 0.0 && self.relax <= 1.0) {
            return bad("relax must be in (0, 1]");
        }
        if !(self.relax_decay > 0.0 && self.relax_decay <= 1.0) {
            return bad("relax_decay must be in (0, 1]");
        }
        if self.max_iters == 0 {
            return bad("max_iters must be >= 1");
        }
        if !(self.tol_albedo > 0.0 && self.tol_evap > 0.0 && self.tol_rough_m > 0.0) {
            return bad("tolerances must be > 0");
        }
        if !(0.0..=1.0).contains(&self.baseline_albedo) || !(self.baseline_evap >= 0.0 && self.baseline_evap.is_finite()) || !(self.baseline_rough_m >= 0.0 && self.baseline_rough_m.is_finite()) {
            return bad("baseline values out of range");
        }
        Ok(())
    }
    /// Relaxation factor of pass `k`.
    pub fn relax_at(&self, k: usize) -> f64 { self.relax * self.relax_decay.powi(k as i32) }
}

/// One pass of the loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegIteration {
    pub iter: usize,
    pub relax: f64,
    /// Share of the land cells whose biome differs from the previous pass (1 on pass 0, where there is no previous map).
    pub changed_frac: f64,
    /// Largest land change of the three surface fields in this pass.
    pub d_albedo_max: f64,
    pub d_evap_max: f64,
    pub d_rough_max: f64,
    /// Area-weighted land means of the surface state AFTER this pass's update.
    pub land_albedo_mean: f64,
    pub land_evap_mean: f64,
    pub land_rough_mean_m: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VegetationReport {
    pub iterations: Vec<VegIteration>,
    pub converged: bool,
    /// Biome areas of the LAST classification (the climate of the last pass).
    pub summary: BiomeSummary,
}

impl VegetationReport {
    pub fn passes(&self) -> usize { self.iterations.len() }
    pub fn last(&self) -> &VegIteration { self.iterations.last().expect("a report has at least one pass") }
}

/// Put the baseline state into the three `veg_*` fields (one layer, every cell). Existing fields of the right size are kept
/// (warm start), so a second call after an edit continues from the previous vegetation.
pub fn seed_vegetation(fields: &mut FieldStore, p: &VegetationParams) -> Result<()> {
    p.validate()?;
    let n = fields.n_cells();
    let vals = [p.baseline_albedo, p.baseline_evap, p.baseline_rough_m];
    for (name, v) in VEG_FIELDS.iter().zip(vals) {
        let keep = fields.has(name) && fields.layers(name)? == 1 && fields.data(name)?.len() == n;
        if !keep {
            fields.put(name, 1, vec![v; n])?;
        }
    }
    Ok(())
}

/// Reset the three fields to the baseline (the "no vegetation" climate).
pub fn clear_vegetation(fields: &mut FieldStore, p: &VegetationParams) -> Result<()> {
    p.validate()?;
    let n = fields.n_cells();
    let vals = [p.baseline_albedo, p.baseline_evap, p.baseline_rough_m];
    for (name, v) in VEG_FIELDS.iter().zip(vals) {
        fields.put(name, 1, vec![v; n])?;
    }
    Ok(())
}

fn area_mean(area: &[f64], land: &[bool], v: &[f64]) -> f64 {
    let (mut s, mut a) = (0.0, 0.0);
    for i in 0..v.len() {
        if land[i] {
            s += area[i] * v[i];
            a += area[i];
        }
    }
    if a > 0.0 { s / a } else { 0.0 }
}

/// Run the vegetation feedback until the surface state stops changing.
///
/// `climate` must run one climate year on `fields` and leave the seasonal fields (`season_temperature`, `season_precipitation`,
/// `season_soil_moisture`) plus `ocean_mask` and whatever else the biome table reads. For the real pipeline use `seasonal_climate`.
/// Returns a report; `converged == false` means `max_iters` ran out (not an error: the state is still the best relaxed one).
pub fn run_vegetation_loop(
    domain: &Domain,
    fields: &mut FieldStore,
    table: &BiomeTable,
    p: &VegetationParams,
    climate: &mut dyn FnMut(&Domain, &mut FieldStore) -> Result<()>,
) -> Result<VegetationReport> {
    p.validate()?;
    seed_vegetation(fields, p)?;
    let n = domain.n_cells();
    let area = &domain.mesh.area;
    let mut prev_id: Option<Vec<f64>> = None;
    let mut iterations: Vec<VegIteration> = Vec::new();
    let mut converged = false;
    let mut last_summary: Option<BiomeSummary> = None;
    for k in 0..p.max_iters {
        climate(domain, fields)?;
        let summary = classify_biomes(domain, fields, table)?;
        let ocean = fields.data("ocean_mask")?.to_vec();
        let land: Vec<bool> = ocean.iter().map(|&o| o < 0.5).collect();
        let id = fields.data("biome")?.to_vec();
        let target = [
            fields.data("biome_albedo")?.to_vec(),
            fields.data("biome_evap_factor")?.to_vec(),
            fields.data("biome_roughness_m")?.to_vec(),
        ];
        let mut state: Vec<Vec<f64>> = Vec::with_capacity(3);
        for name in VEG_FIELDS.iter() {
            let d = fields.data(name)?;
            if d.len() != n {
                return Err(Error::InvalidParam(format!("vegetation: field '{name}' must have one value per cell")));
            }
            state.push(d.to_vec());
        }
        let r = p.relax_at(k);
        let mut dmax = [0.0f64; 3];
        for i in 0..n {
            if !land[i] {
                continue;
            }
            for f in 0..3 {
                let step = r * (target[f][i] - state[f][i]);
                state[f][i] += step;
                dmax[f] = dmax[f].max(step.abs());
            }
        }
        let land_cells = (0..n).filter(|&i| land[i]).count();
        let changed = match &prev_id {
            None => 1.0,
            Some(old) => {
                let c = (0..n).filter(|&i| land[i] && old[i] != id[i]).count();
                if land_cells > 0 { c as f64 / land_cells as f64 } else { 0.0 }
            }
        };
        iterations.push(VegIteration {
            iter: k,
            relax: r,
            changed_frac: changed,
            d_albedo_max: dmax[0],
            d_evap_max: dmax[1],
            d_rough_max: dmax[2],
            land_albedo_mean: area_mean(area, &land, &state[0]),
            land_evap_mean: area_mean(area, &land, &state[1]),
            land_rough_mean_m: area_mean(area, &land, &state[2]),
        });
        for (f, name) in VEG_FIELDS.iter().enumerate() {
            fields.put(name, 1, std::mem::take(&mut state[f]))?;
        }
        prev_id = Some(id);
        last_summary = Some(summary);
        if dmax[0] <= p.tol_albedo && dmax[1] <= p.tol_evap && dmax[2] <= p.tol_rough_m {
            converged = true;
            break;
        }
    }
    Ok(VegetationReport { iterations, converged, summary: last_summary.expect("max_iters >= 1 runs at least one pass") })
}

/// The climate closure of the real pipeline: run `years` years of `seasons` through `run_climate_seasons` on `scheduler` (which must
/// hold `add_land_water_pipeline`, a seeded land state and `set_land_step` for the season length). The first call of the loop is the
/// bare-land climate; the land state (soil, snow, ice) carries over between calls, so each pass starts warm.
pub fn seasonal_climate<'a>(scheduler: &'a mut Scheduler, seasons: &'a [f64], years: usize) -> impl FnMut(&Domain, &mut FieldStore) -> Result<()> + 'a {
    move |domain: &Domain, fields: &mut FieldStore| {
        for _ in 0..years.max(1) {
            run_climate_seasons(domain, fields, scheduler, seasons)?;
        }
        Ok(())
    }
}

/// Mean land albedo, evaporation factor and canopy of the current surface state (area weighted); `None` before `seed_vegetation`.
pub fn land_surface_means(domain: &Domain, fields: &FieldStore) -> Result<Option<(f64, f64, f64)>> {
    if !VEG_FIELDS.iter().all(|f| fields.has(f)) {
        return Ok(None);
    }
    let ocean = fields.data("ocean_mask")?;
    let land: Vec<bool> = ocean.iter().map(|&o| o < 0.5).collect();
    let a = &domain.mesh.area;
    Ok(Some((
        area_mean(a, &land, fields.data("veg_albedo")?),
        area_mean(a, &land, fields.data("veg_evap_factor")?),
        area_mean(a, &land, fields.data("veg_roughness_m")?),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_relaxation_decays() {
        let p = VegetationParams::default();
        p.validate().unwrap();
        assert_eq!(p.relax_at(0), 0.5);
        assert!((p.relax_at(2) - 0.245).abs() < 1e-12);
        assert!(VegetationParams { relax: 0.0, ..p }.validate().is_err());
        assert!(VegetationParams { relax_decay: 1.5, ..p }.validate().is_err());
        assert!(VegetationParams { max_iters: 0, ..p }.validate().is_err());
    }
}
