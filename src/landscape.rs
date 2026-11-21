//! Part 4c-3: the terrain-wind-rain feedback loop (`landscape`), the geological time driver `run_geology` and Part 4 diagnostics.
//!
//! TWO SCHEDULERS, ONE FIELD STORE (same pattern as `run_ocean_climate`):
//!  * CLIMATE scheduler = `add_land_water_pipeline` (energy, pressure, wind, moisture, precipitation, `land_water`, ...). It reads `elevation`
//!    and writes `precipitation`, `evaporation`, `runoff`, glaciers. It is expensive, so it runs only every `climate_every` geological steps
//!    (default 10), over `seasons`; the season means are blended into the fields the geology reads (relaxation `climate_relax`, default 0.5).
//!  * GEOLOGY scheduler = `add_geology_stage`: `sea_level`, `ocean_mask`, `flow_routing`, `rivers_lakes` (land-runoff variant: discharge
//!    from the climate `runoff`), `erosion` (discharge variant), `sediment` (discharge variant) and `terrain` (net variant), declared as the
//!    feedback loop `landscape` (watch `elevation` on LAND cells (cells of the module `landscape_mask`'s `coast_band` are excluded: river-mouth distributary switching and shoreline cells
//!    flip between land and ocean, 5-20 m, and never settle), relaxation 0.5, tolerance `LANDSCAPE_TOL`, at most `LANDSCAPE_MAX_ITERS` passes).
//!    The loop is a fixed point at ONE geological time: `e = F(tectonic + terrain_delta_prev + step(e))` where `F` is `terrain` (noise,
//!    thermal, caps) and `step(e)` the incision + deposition + talus step computed on `e`. The driver commits the erosion and sediment
//!    state after it converged (one commit per step), so reruns inside the loop are idempotent.
//!
//! KNOWN BIAS of the inner fixed point (documented, bounded, tested by `halving_the_geological_step_changes_the_result_only_a_little`):
//! `erosion` measures its cut from the CURRENT iterate `e`, not from the committed start-of-step surface `e0`, so the fixed point of
//! `e = e0 - c/(1+c) (e - r)` (n = 1, c the stream power coefficient of the cell, r the receiver) is `e - r = (e0 - r)(1+c)/(1+2c)`, whereas the
//! exact implicit step is `(e0 - r)/(1+c)`. The two agree to first order in c (`1 - c`); the second-order terms are `2c^2` against `c^2`, so the
//! loop UNDER-cuts by about `c^2` per step (c = 0.1: 1%; c = 0.3: 6%; c = 1: 17%). Keep c below ~0.3 (small `years_per_step`); the default
//! `years_per_step` 2e4 gives c of a few 1e-2 on Earth-like basins. Removing the bias needs `erosion` to read the committed `elevation_prev`.
//!
//! The terrain-wind-rain feedback is closed in BOTH directions: elevation -> wind and rain (climate) and rain -> runoff -> discharge ->
//! incision and sediment capacity -> elevation (geology). `elevation` has exactly one producer (`terrain`), as the scheduler requires.
//!
//! Surface identity checked every step (`drift_m3`): `sum(area * (elevation_end - elevation_start)) = -exports` up to the part that `terrain`
//! itself changes when it re-applies noise, thermal relaxation and caps to `tectonic + terrain_delta` (it is NOT guaranteed to be idempotent;
//! the drift is reported, never hidden). DirtyRegion: every module here does a full recompute; incremental propagation is Part 6.
use crate::domain::Domain;
use crate::erosion::{commit_erosion_state, Erosion};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::land_water::{run_land_cycle, seed_land_state};
use crate::module::{DirtyRegion, Module};
use crate::rivers_lakes::RiversLakes;
use crate::scheduler::{FeedbackLoop, Scheduler};
use crate::sediment::{commit_sediment_state, seed_surface_state, set_surface_years, Sediment};
use crate::terrain::TerrainShaping;

/// Default tolerance and pass cap of the inner `landscape` loop.
pub const LANDSCAPE_TOL: f64 = 1.0e-4;
pub const LANDSCAPE_MAX_ITERS: usize = 40;

/// Module `landscape_mask`: writes `coast_band` (0/1) = ocean cells, land cells with an ocean neighbour, land cells within `band_m` of
/// `sea_level`, and (since snapshot 4) depression/lake cells with their neighbours. These cells flip between land and ocean (and between two receivers) from pass to pass; the loop relaxes them but does not
/// count them in its convergence test (`FeedbackLoop::exclude_where("coast_band")`).
pub struct LandscapeMask {
    params: crate::module::ParamSet,
}

impl LandscapeMask {
    pub fn new() -> Self {
        LandscapeMask { params: crate::module::ParamSet::new().with("band_m", 25.0) }
    }
}

impl Default for LandscapeMask {
    fn default() -> Self { Self::new() }
}

impl Module for LandscapeMask {
    fn spec(&self) -> crate::module::ModuleSpec {
        crate::module::ModuleSpec {
            name: "landscape_mask".into(),
            inputs: vec!["elevation".into(), "ocean_mask".into(), "sea_level".into(), "depression_depth".into()],
            outputs: vec!["coast_band".into()],
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &crate::module::ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "band_m" || !(value >= 0.0) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("landscape_mask: bad parameter '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let band = self.params.get_or("band_m", 25.0);
        let z = fields.data("elevation")?.to_vec();
        let ocean = fields.data("ocean_mask")?.to_vec();
        let sl = fields.data("sea_level")?.to_vec();
        let dep = fields.data("depression_depth")?.to_vec();
        let out: Vec<f64> = (0..n)
            .map(|c| {
                // shoreline cells, and lake (depression) cells with their neighbours: a shallow pit that appears and dissolves from pass to
                // pass (sediment trapping and the spill level change it discretely) and the cells that cut down to its surface oscillate
                // with it (2-cycles found by `examples/landscape_diag.rs`)
                let flagged = ocean[c] > 0.5
                    || (z[c] - sl[c]).abs() <= band
                    || dep[c] > 0.0
                    || domain.mesh.neighbors(c).iter().any(|&j| ocean[j as usize] > 0.5 || dep[j as usize] > 0.0);
                if flagged { 1.0 } else { 0.0 }
            })
            .collect();
        fields.put("coast_band", 1, out)
    }
}

/// Registers the geology members and declares the loop `landscape`. External inputs: `tectonic_elevation`, `rock_strength`, `runoff`,
/// `evaporation` (climate), seeded state (`seed_landscape`).
pub fn add_geology_stage(s: &mut Scheduler) -> Result<()> {
    crate::ocean::add_ocean_mask_stage(s)?;
    s.register(Box::new(LandscapeMask::new()))?;
    crate::flow_routing::add_flow_routing_module(s)?;
    s.register(Box::new(RiversLakes::with_land_runoff()))?;
    s.register(Box::new(Erosion::with_discharge()))?;
    s.register(Box::new(Sediment::with_discharge()))?;
    s.register(Box::new(TerrainShaping::with_net_erosion()))?;
    s.add_feedback_loop(
        // landscape_mask sits after flow_routing: it reads this pass's `depression_depth` (the scheduler reads the mask after the whole pass)
        FeedbackLoop::new("landscape", &["sea_level", "ocean_mask", "flow_routing", "landscape_mask", "rivers_lakes", "erosion", "sediment", "terrain"])
            .watch(&["elevation"])
            .exclude_where("coast_band")
            .relaxation(0.5)
            // the map has discrete switches (receivers, shallow lakes): constant relaxation chatters at a fixed amplitude, so the weight
            // decays 0.8x per pass after pass 8 (0.5 -> ~5e-4 at pass 40); smooth parts still converge normally
            .relax_decay(0.8, 8)
            .tolerance(LANDSCAPE_TOL)
            .max_iters(LANDSCAPE_MAX_ITERS),
    )
}

/// Seeds everything the two schedulers need that the user does not supply: zero erosion and sediment state, `terrain_delta` = 0, the
/// consistent first `elevation` (= `terrain` on `tectonic_elevation`), temperature 288 K and the land state (only when absent).
/// Needs `tectonic_elevation` and `rock_strength`. `TerrainShaping::with_net_erosion()` with default parameters is used for the first
/// elevation; if you change terrain parameters on the geology scheduler, call `seed_landscape_with` instead.
pub fn seed_landscape(domain: &Domain, fields: &mut FieldStore) -> Result<()> {
    seed_landscape_with(domain, fields, TerrainShaping::with_net_erosion())
}

pub fn seed_landscape_with(domain: &Domain, fields: &mut FieldStore, mut terrain: TerrainShaping) -> Result<()> {
    let n = domain.n_cells();
    seed_surface_state(fields)?;
    fields.put("terrain_delta", 1, vec![0.0; n])?;
    terrain.run(domain, fields, &DirtyRegion::All)?;
    if !fields.has("temperature") {
        crate::energy::seed_temperature(fields, 288.0)?;
    }
    if !fields.has("soil_moisture_prev") {
        seed_land_state(fields, 0.05)?;
    }
    Ok(())
}

/// Driver settings. `years_per_step * n_steps` is the geological time (default 2e4 * 50 = 1e6 yr).
#[derive(Debug, Clone)]
pub struct GeologyParams {
    pub years_per_step: f64,
    pub n_steps: usize,
    /// Climate re-runs every this many geological steps (the first run is before step 0).
    pub climate_every: usize,
    /// Fractions of the orbit averaged by one climate run.
    pub seasons: Vec<f64>,
    /// Weight of the new climate in the blend (the first run takes it whole).
    pub climate_relax: f64,
    /// Convergence of the climate coupling: relative change of the blended `runoff` between two climate runs and relative change of
    /// `elevation` (over the relief) between them.
    pub tol_runoff: f64,
    pub tol_elevation: f64,
    /// Stop at the first climate period that meets both tolerances (never before two climate runs).
    pub stop_when_converged: bool,
}

impl Default for GeologyParams {
    fn default() -> Self {
        GeologyParams {
            years_per_step: 2.0e4,
            n_steps: 50,
            climate_every: 10,
            seasons: vec![0.125, 0.375, 0.625, 0.875],
            climate_relax: 0.5,
            tol_runoff: 0.05,
            tol_elevation: 0.01,
            stop_when_converged: false,
        }
    }
}

/// One geological step.
#[derive(Debug, Clone, Copy)]
pub struct GeoStep {
    pub step: usize,
    /// Passes of the `landscape` loop and whether it met its tolerance.
    pub passes: usize,
    pub converged: bool,
    pub residual: f64,
    /// Sediment volumes of the last pass (m^3 per step).
    pub eroded_m3: f64,
    pub deposited_m3: f64,
    pub stored_lakes_m3: f64,
    pub exported_m3: f64,
    /// `|eroded - (deposited + stored + exported)|` (m^3).
    pub sediment_residual_m3: f64,
    /// `sum(area * (elevation_end - elevation_start))` and its difference from `-exported` (m^3).
    pub surface_change_m3: f64,
    pub drift_m3: f64,
    pub n_depression_cells: usize,
    /// `rivers_lakes.n_unresolved_lakes` of the last pass (lakes the fill-and-spill solve could not resolve; must be 0).
    pub n_unresolved_lakes: usize,
    /// `|rivers_lakes.balance_residual|`, already relative to the land runoff input.
    pub river_balance_rel: f64,
    pub max_elevation_m: f64,
    pub mean_land_elevation_m: f64,
}

/// One climate run.
#[derive(Debug, Clone, Copy)]
pub struct ClimateRun {
    /// Geological step it ran before.
    pub step: usize,
    /// Relative max change of the blended `runoff` against the previous blend (infinite on the first run).
    pub runoff_residual: f64,
    /// Max change of `elevation` since the previous climate run over `max(relief, 1 m)` (infinite on the first run).
    pub elevation_residual: f64,
    /// Worst water budget residual (relative) of its seasons.
    pub water_residual_rel: f64,
    pub mean_precip_mm_day: f64,
}

#[derive(Debug, Clone)]
pub struct GeologyReport {
    pub steps: Vec<GeoStep>,
    pub climate: Vec<ClimateRun>,
    /// Both climate residuals under tolerance at the last climate run (needs >= 2 runs).
    pub converged: bool,
    /// Every inner loop met its tolerance.
    pub all_loops_converged: bool,
    pub max_passes: usize,
    pub years: f64,
    pub eroded_total_m3: f64,
    pub deposited_total_m3: f64,
    pub exported_total_m3: f64,
    /// Worst per-step `sediment_residual / eroded` and worst climate water residual.
    pub max_sediment_residual_rel: f64,
    pub max_water_residual_rel: f64,
    /// Worst `|balance_residual|` (relative) of `rivers_lakes` over the run.
    pub max_river_balance_rel: f64,
}

fn rel_change(old: &[f64], new: &[f64]) -> f64 {
    let (mut d, mut m) = (0.0f64, 0.0f64);
    for (a, b) in old.iter().zip(new) {
        d = d.max((b - a).abs());
        m = m.max(b.abs());
    }
    d / m.max(1e-300)
}

/// Runs the climate scheduler over the seasons and blends `runoff`, `evaporation`, `precipitation` (the fields the geology reads).
fn climate_run(domain: &Domain, fields: &mut FieldStore, climate: &mut Scheduler, p: &GeologyParams, step: usize, first: bool, prev_elev: &[f64]) -> Result<ClimateRun> {
    let n = domain.n_cells();
    let names = ["runoff", "evaporation", "precipitation"];
    let old: Vec<Vec<f64>> = if first { Vec::new() } else { names.iter().map(|k| fields.data(k).map(|d| d.to_vec())).collect::<Result<_>>()? };
    let mut acc = vec![vec![0.0; n]; 3];
    let (mut water, mut mp) = (0.0f64, 0.0);
    for &season in &p.seasons {
        let r = run_land_cycle(domain, fields, climate, &[season])?;
        water = water.max(r[0].climate.budget.residual_rel);
        mp += r[0].climate.mean_precip_mm_day / p.seasons.len() as f64;
        for (a, k) in acc.iter_mut().zip(names.iter()) {
            for (x, y) in a.iter_mut().zip(fields.data(k)?) {
                *x += y / p.seasons.len() as f64;
            }
        }
    }
    let mut runoff_res = f64::INFINITY;
    for (i, k) in names.iter().enumerate() {
        let blended: Vec<f64> = if first { acc[i].clone() } else { old[i].iter().zip(&acc[i]).map(|(o, nw)| o + p.climate_relax * (nw - o)).collect() };
        if i == 0 && !first {
            runoff_res = rel_change(&old[0], &blended);
        }
        fields.put(k, 1, blended)?;
    }
    let z = fields.data("elevation")?;
    let relief = z.iter().cloned().fold(f64::MIN, f64::max) - z.iter().cloned().fold(f64::MAX, f64::min);
    let elev_res = if first { f64::INFINITY } else { z.iter().zip(prev_elev).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max) / relief.max(1.0) };
    Ok(ClimateRun { step, runoff_residual: runoff_res, elevation_residual: elev_res, water_residual_rel: water, mean_precip_mm_day: mp })
}

/// One geological step on the geology scheduler: runs the `landscape` loop, reads the volumes of the converged state, then COMMITS the
/// erosion and sediment state (one commit per step, so reruns inside the loop were idempotent).
fn geology_step(domain: &Domain, fields: &mut FieldStore, geo: &mut Scheduler, step: usize) -> Result<GeoStep> {
    let n = domain.n_cells();
    let area = &domain.mesh.area;
    let e0 = fields.data("elevation")?.to_vec();
    let rep = geo.run(domain, fields, &DirtyRegion::All)?;
    let (passes, converged, residual) = match rep.loops.iter().find(|l| l.name == "landscape") {
        Some(l) => (l.iterations, l.converged, l.residual),
        None => return Err(Error::InvalidParam("run_geology: the geology scheduler has no loop 'landscape'".into())),
    };
    let m = |k: &str| geo.module_param("sediment", k).unwrap_or(0.0);
    let (eroded, dep, lakes) = (m("eroded_volume_m3"), m("deposited_land_m3") + m("deposited_delta_m3"), m("stored_lakes_m3"));
    let exported = m("export_ocean_m3") + m("export_void_m3") + m("export_sink_m3");
    let e1 = fields.data("elevation")?.to_vec();
    let ocean = fields.data("ocean_mask")?.to_vec();
    let change: f64 = (0..n).map(|c| area[c] * (e1[c] - e0[c])).sum();
    let (mut ls, mut la) = (0.0, 0.0);
    for c in 0..n {
        if ocean[c] < 0.5 {
            ls += area[c] * e1[c];
            la += area[c];
        }
    }
    // `rivers_lakes.balance_residual` is ALREADY relative to the land runoff input (4b-2): do not normalise it again.
    let river_rel = geo.module_param("rivers_lakes", "balance_residual").unwrap_or(0.0).abs();
    let sres = (eroded - dep - lakes - exported).abs();
    let n_dep = fields.data("depression_depth")?.iter().filter(|&&d| d > 0.0).count();
    let out = GeoStep {
        step,
        passes,
        converged,
        residual,
        eroded_m3: eroded,
        deposited_m3: dep,
        stored_lakes_m3: lakes,
        exported_m3: exported,
        sediment_residual_m3: sres,
        surface_change_m3: change,
        drift_m3: change + exported,
        n_depression_cells: n_dep,
        n_unresolved_lakes: geo.module_param("rivers_lakes", "n_unresolved_lakes").unwrap_or(0.0) as usize,
        river_balance_rel: river_rel,
        max_elevation_m: e1.iter().cloned().fold(f64::MIN, f64::max),
        mean_land_elevation_m: if la > 0.0 { ls / la } else { 0.0 },
    };
    commit_erosion_state(fields)?;
    commit_sediment_state(fields)?;
    Ok(out)
}

fn build_report(steps: Vec<GeoStep>, runs: Vec<ClimateRun>, years: f64, converged: bool) -> GeologyReport {
    let sum = |f: &dyn Fn(&GeoStep) -> f64| steps.iter().map(|s| f(s)).sum::<f64>();
    let max_sed = steps.iter().map(|s| s.sediment_residual_m3 / s.eroded_m3.max(1.0)).fold(0.0, f64::max);
    GeologyReport {
        converged,
        all_loops_converged: steps.iter().all(|s| s.converged),
        max_passes: steps.iter().map(|s| s.passes).max().unwrap_or(0),
        years,
        eroded_total_m3: sum(&|s| s.eroded_m3),
        deposited_total_m3: sum(&|s| s.deposited_m3),
        exported_total_m3: sum(&|s| s.exported_m3),
        max_sediment_residual_rel: max_sed,
        max_water_residual_rel: runs.iter().map(|r| r.water_residual_rel).fold(0.0, f64::max),
        max_river_balance_rel: steps.iter().map(|s| s.river_balance_rel).fold(0.0, f64::max),
        steps,
        climate: runs,
    }
}

/// Geological time driver, analogous to `run_season`. `climate` holds `add_land_water_pipeline`, `geo` holds `add_geology_stage`;
/// `fields` was prepared by `seed_landscape` (plus `tectonic_elevation`, `rock_strength`). Sets `years_per_step` on `erosion` and
/// `sediment`. Elevation, erosion and sediment state advance by `n_steps * years_per_step` years.
pub fn run_geology(domain: &Domain, fields: &mut FieldStore, climate: &mut Scheduler, geo: &mut Scheduler, p: &GeologyParams) -> Result<GeologyReport> {
    if p.n_steps == 0 || p.climate_every == 0 || p.seasons.is_empty() || !(p.years_per_step > 0.0) || !(p.climate_relax > 0.0 && p.climate_relax <= 1.0) {
        return Err(Error::InvalidParam("run_geology: n_steps, climate_every, seasons, years_per_step and climate_relax must be valid".into()));
    }
    set_surface_years(geo, p.years_per_step)?;
    let mut steps = Vec::with_capacity(p.n_steps);
    let mut runs: Vec<ClimateRun> = Vec::new();
    let mut at_last_climate = fields.data("elevation")?.to_vec();
    for step in 0..p.n_steps {
        if step % p.climate_every == 0 {
            let run = climate_run(domain, fields, climate, p, step, runs.is_empty(), &at_last_climate)?;
            at_last_climate = fields.data("elevation")?.to_vec();
            runs.push(run);
            if p.stop_when_converged && runs.len() >= 2 && run.runoff_residual <= p.tol_runoff && run.elevation_residual <= p.tol_elevation {
                break;
            }
        }
        steps.push(geology_step(domain, fields, geo, step)?);
    }
    let converged = runs.len() >= 2 && runs.last().map(|r| r.runoff_residual <= p.tol_runoff && r.elevation_residual <= p.tol_elevation).unwrap_or(false);
    Ok(build_report(steps, runs, p.years_per_step * p.n_steps as f64, converged))
}

/// Geology with a PRESCRIBED climate: no climate scheduler runs, `runoff` and `evaporation` (kg/m^2/s) stay as the caller put them
/// (a painted rainfall map, a constant, or the blend left by an earlier `run_geology`). The `landscape` loop and all budgets are
/// identical to `run_geology`; the report has no climate runs and `converged` is false (no outer coupling was tested). This is what the
/// cube and wrapped-plane convergence tests use, and what an editor calls to re-erode under a user-painted climate.
pub fn run_geology_prescribed(domain: &Domain, fields: &mut FieldStore, geo: &mut Scheduler, years_per_step: f64, n_steps: usize) -> Result<GeologyReport> {
    if n_steps == 0 || !(years_per_step > 0.0) || !years_per_step.is_finite() {
        return Err(Error::InvalidParam("run_geology_prescribed: n_steps and years_per_step must be valid".into()));
    }
    set_surface_years(geo, years_per_step)?;
    let mut steps = Vec::with_capacity(n_steps);
    for step in 0..n_steps {
        steps.push(geology_step(domain, fields, geo, step)?);
    }
    Ok(build_report(steps, Vec::new(), years_per_step * n_steps as f64, false))
}

/// Part 4 close-out diagnostics of the current state (after `run_geology`, or after a climate run plus the geology members).
#[derive(Debug, Clone, Copy)]
pub struct LandscapeSummary {
    pub land_fraction: f64,
    pub mean_land_elevation_m: f64,
    pub max_elevation_m: f64,
    /// Share of LAND area covered by glacier ice (`glacier_mask`).
    pub glacier_fraction_of_land: f64,
    /// Total river water reaching the ocean (m^3/s) and the same in Sverdrup (1e6 m^3/s).
    pub ocean_discharge_m3s: f64,
    pub river_cells: usize,
    pub n_lakes: usize,
    pub n_depression_cells: usize,
    /// Land cells that do not reach the ocean, a lake or an outlet by following `flow_dir` (must be 0).
    pub land_cells_not_draining: usize,
}

pub fn landscape_summary(domain: &Domain, fields: &FieldStore, geo: &Scheduler) -> Result<LandscapeSummary> {
    let n = domain.n_cells();
    let area = &domain.mesh.area;
    let z = fields.data("elevation")?;
    let ocean = fields.data("ocean_mask")?;
    // worlds run with a prescribed climate (`run_geology_prescribed`) have no glaciers: the field is optional here
    let gl: Vec<f64> = match fields.get("glacier_mask") {
        Some(f) => f.data.clone(),
        None => vec![0.0; n],
    };
    let fd: Vec<i64> = fields.data("flow_dir")?.iter().map(|&v| v as i64).collect();
    let dep = fields.data("depression_depth")?;
    let (mut la, mut ls, mut ga) = (0.0, 0.0, 0.0);
    for c in 0..n {
        if ocean[c] < 0.5 {
            la += area[c];
            ls += area[c] * z[c];
            if gl[c] > 0.5 { ga += area[c]; }
        }
    }
    let mut bad = 0;
    for c in 0..n {
        if ocean[c] > 0.5 { continue; }
        let (mut cur, mut hops) = (c, 0usize);
        while fd[cur] >= 0 && ocean[cur] < 0.5 && hops <= n {
            cur = fd[cur] as usize;
            hops += 1;
        }
        if hops > n || !(ocean[cur] > 0.5 || fd[cur] < 0 || dep[cur] > 0.0) { bad += 1; }
    }
    let rl = |k: &str| geo.module_param("rivers_lakes", k).unwrap_or(0.0);
    Ok(LandscapeSummary {
        land_fraction: la / domain.mesh.total_area(),
        mean_land_elevation_m: if la > 0.0 { ls / la } else { 0.0 },
        max_elevation_m: z.iter().cloned().fold(f64::MIN, f64::max),
        glacier_fraction_of_land: if la > 0.0 { ga / la } else { 0.0 },
        ocean_discharge_m3s: rl("ocean_discharge_m3s"),
        river_cells: rl("river_cells") as usize,
        n_lakes: rl("n_lakes") as usize,
        n_depression_cells: dep.iter().filter(|&&d| d > 0.0).count(),
        land_cells_not_draining: bad,
    })
}
