//! Part 3b-4: water-budget closure, seasonal driver and the assembled moisture pipeline.
//!
//! `WaterBudget::from_fields` closes `vapor_end + rain = vapor_start + evaporation` over one transport run
//! (every term is an area-integrated mass in kg). `run_season` sets the insolation season, runs the scheduler
//! once and returns the budget plus a precipitation summary, so the stage can be verified per season.
use crate::domain::Domain;
use crate::error::Result;
use crate::fields::FieldStore;
use crate::module::DirtyRegion;
use crate::scheduler::Scheduler;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterBudget {
    pub vapor_start_kg: f64,
    pub vapor_end_kg: f64,
    pub evaporation_kg: f64,
    pub precipitation_kg: f64,
    /// vapor_end + precipitation - vapor_start - evaporation (kg); 0 when water is conserved.
    pub residual_kg: f64,
    /// |residual| / max(vapor_start + evaporation, tiny).
    pub residual_rel: f64,
    /// (vapor_end - vapor_start) / evaporation: 0 = steady state, 1 = nothing rained out. Spin-up measure.
    pub storage_fraction: f64,
    pub transport_time_s: f64,
}

impl WaterBudget {
    /// Needs `vapor`, `vapor_start`, `evaporation` and (optionally) `precipitation` in `fields`.
    pub fn from_fields(domain: &Domain, fields: &FieldStore, transport_time_s: f64) -> Result<Self> {
        let area = &domain.mesh.area;
        let integ = |name: &str| -> Result<f64> { Ok(fields.data(name)?.iter().zip(area).map(|(a, b)| a * b).sum()) };
        let v0 = integ("vapor_start")?;
        let v1 = integ("vapor")?;
        let e = integ("evaporation")? * transport_time_s;
        let p = if fields.has("precipitation") { integ("precipitation")? * transport_time_s } else { 0.0 };
        let residual = v1 + p - v0 - e;
        Ok(WaterBudget {
            vapor_start_kg: v0,
            vapor_end_kg: v1,
            evaporation_kg: e,
            precipitation_kg: p,
            residual_kg: residual,
            residual_rel: residual.abs() / (v0 + e).max(1e-300),
            storage_fraction: (v1 - v0) / e.max(1e-300),
            transport_time_s,
        })
    }
}

/// Area-mean precipitation in mm/day (1 kg/m^2 = 1 mm).
pub fn mean_precip_mm_day(domain: &Domain, fields: &FieldStore) -> Result<f64> {
    let p = fields.data("precipitation")?;
    let s: f64 = p.iter().zip(&domain.mesh.area).map(|(a, b)| a * b).sum();
    Ok(s / domain.mesh.total_area() * 86400.0)
}

/// Area-mean evaporation in mm/day.
pub fn mean_evap_mm_day(domain: &Domain, fields: &FieldStore) -> Result<f64> {
    let p = fields.data("evaporation")?;
    let s: f64 = p.iter().zip(&domain.mesh.area).map(|(a, b)| a * b).sum();
    Ok(s / domain.mesh.total_area() * 86400.0)
}

#[derive(Debug, Clone, Copy)]
pub struct SeasonReport {
    pub season: f64,
    pub budget: WaterBudget,
    pub mean_precip_mm_day: f64,
    pub mean_evap_mm_day: f64,
    pub passes_energy: usize,
}

/// Registers the whole climate chain: insolation + ice-albedo energy loop, pressure + wind,
/// moisture sources and coupled transport/precipitation. `elevation` must exist (terrain or supplied).
pub fn add_climate_pipeline(s: &mut Scheduler) -> Result<()> {
    crate::energy::add_energy_stage(s)?;
    crate::pressure::add_atmosphere_stage(s)?;
    crate::moisture::add_moisture_source_stage(s)?;
    crate::transport::add_moisture_precip_stage(s)
}

/// Part 3c-5: the climate chain with the stepped wind on the chosen backend. `Backend::Cpu` uses the f64 stepped
/// wind (not the diagnostic one: `add_climate_pipeline` keeps the diagnostic wind as the default). Moisture
/// transport, projection and precipitation stay on the CPU in f64, so the water budget closes as before.
pub fn add_climate_pipeline_with(s: &mut Scheduler, backend: crate::wind_stepped::Backend) -> Result<()> {
    crate::energy::add_energy_stage(s)?;
    crate::wind_stepped::add_atmosphere_stage_stepped_with(s, backend)?;
    crate::moisture::add_moisture_source_stage(s)?;
    crate::transport::add_moisture_precip_stage(s)
}

/// Run the registered pipeline once for `season` (fraction of the orbit). `temperature_sl` from a previous season
/// is used as a warm start by the energy balance. Call `seed_temperature` before the first run.
pub fn run_season(domain: &Domain, fields: &mut FieldStore, scheduler: &mut Scheduler, season: f64) -> Result<SeasonReport> {
    scheduler.set_param("insolation", "season", season)?;
    let report = scheduler.run(domain, fields, &DirtyRegion::All)?;
    let t = scheduler.module_param("moisture_transport", "transport_time_s").unwrap_or(0.0);
    let passes = report.loops.iter().map(|l| l.iterations).max().unwrap_or(0);
    Ok(SeasonReport {
        season,
        budget: WaterBudget::from_fields(domain, fields, t)?,
        mean_precip_mm_day: mean_precip_mm_day(domain, fields)?,
        mean_evap_mm_day: mean_evap_mm_day(domain, fields)?,
        passes_energy: passes,
    })
}
