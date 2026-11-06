//! Part 3c-5: the Earth-like test world and climate summaries shared by `tests/part3c5.rs`,
//! `examples/bench_wind.rs` and `examples/compare_backends.rs` (the same world as `examples/tune.rs`).
use crate::budget::SeasonReport;
use crate::domain::{Domain, Light};
use crate::error::Result;
use crate::fields::FieldStore;
use crate::generators::{generate_sphere, SphereParams};
use crate::scheduler::Scheduler;

pub const YEAR_S: f64 = 3.15576e7;
pub const DAY_S: f64 = 86400.0;
/// Axis of the single 3000 m ridge continent (unnormalised).
const AXIS: [f64; 3] = [0.3, 0.5, 0.8];

fn ridge_cos(u: [f64; 3]) -> f64 {
    let al = (AXIS[0] * AXIS[0] + AXIS[1] * AXIS[1] + AXIS[2] * AXIS[2]).sqrt();
    (u[0] * AXIS[0] + u[1] * AXIS[1] + u[2] * AXIS[2]) / al
}

/// Earth-sized sphere (tilt 23.4 deg, 1361 W/m^2), a 30% land cap with a 3000 m ridge, ocean elsewhere.
/// Temperature is seeded (288 K) so the energy loop can start. Returns the domain and its fields.
pub fn earth_like_world(subdivision: u32) -> Result<(Domain, FieldStore)> {
    let mut d = generate_sphere(&SphereParams { radius: 6.371e6, subdivision, surface_gravity: 9.81, seed: 7 })?;
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR_S, DAY_S, 23.4f64.to_radians()));
    let z: Vec<f64> = d
        .mesh
        .up
        .iter()
        .map(|&u| {
            let c = ridge_cos(u);
            if c > 0.4 { 100.0 + 3000.0 * (-((c - 0.7).powi(2)) / (2.0 * 0.05f64.powi(2))).exp() } else { -3000.0 }
        })
        .collect();
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z)?;
    crate::energy::seed_temperature(&mut f, 288.0)?;
    Ok((d, f))
}

/// What the backend comparison looks at, all in mm/day except `residual_rel`.
#[derive(Debug, Clone)]
pub struct ClimateSummary {
    pub season: f64,
    pub mean_precip: f64,
    /// Area-mean precipitation per 15-degree latitude band, from -90 to +90 (12 values).
    pub zonal: Vec<f64>,
    /// Mean precipitation over |lat| < 10 and over 20 <= |lat| < 35.
    pub tropics: f64,
    pub subtropics: f64,
    /// Ridge band (0.55 < cos < 0.85, land): windward (lift > 0) and leeward cells.
    pub windward: f64,
    pub leeward: f64,
    /// Water budget residual of the transport run (relative; the CPU f64 path, must be < 1e-8).
    pub residual_rel: f64,
}

pub fn summarize(d: &Domain, f: &FieldStore, rep: &SeasonReport) -> Result<ClimateSummary> {
    let n = d.n_cells();
    let (p, lift, z) = (f.data("precipitation")?, f.data("lift")?, f.data("elevation")?);
    let lat = |i: usize| d.mesh.up[i][2].asin().to_degrees();
    let area = &d.mesh.area;
    let band = |keep: &dyn Fn(f64) -> bool| -> f64 {
        let (mut s, mut a) = (0.0, 0.0);
        for i in 0..n {
            if keep(lat(i)) { s += p[i] * area[i]; a += area[i]; }
        }
        if a > 0.0 { s / a * DAY_S } else { f64::NAN }
    };
    let zonal: Vec<f64> = (0..12).map(|k| { let lo = -90.0 + 15.0 * k as f64; band(&|l| l >= lo && l < lo + 15.0) }).collect();
    let near: Vec<usize> = (0..n).filter(|&i| { let c = ridge_cos(d.mesh.up[i]); c > 0.55 && c < 0.85 && z[i] > 0.0 }).collect();
    let mean = |idx: Vec<usize>| if idx.is_empty() { f64::NAN } else { idx.iter().map(|&i| p[i]).sum::<f64>() / idx.len() as f64 * DAY_S };
    let (wi, le): (Vec<usize>, Vec<usize>) = near.iter().partition(|&&i| lift[i] > 0.0);
    Ok(ClimateSummary {
        season: rep.season,
        mean_precip: rep.mean_precip_mm_day,
        zonal,
        tropics: band(&|l| l.abs() < 10.0),
        subtropics: band(&|l| l.abs() >= 20.0 && l.abs() < 35.0),
        windward: mean(wi),
        leeward: mean(le),
        residual_rel: rep.budget.residual_rel,
    })
}

/// One fresh pipeline run (energy loop, stepped wind on `backend`, moisture, precipitation) over `seasons`.
/// Returns one summary per season and the scheduler (to read `module_param("wind", "last_*")`).
pub fn run_pipeline(
    d: &Domain,
    f: &mut FieldStore,
    backend: crate::wind_stepped::Backend,
    seasons: &[f64],
) -> Result<(Vec<ClimateSummary>, Scheduler)> {
    let mut s = Scheduler::new();
    crate::budget::add_climate_pipeline_with(&mut s, backend)?;
    let mut out = Vec::new();
    for &season in seasons {
        let rep = crate::budget::run_season(d, f, &mut s, season)?;
        out.push(summarize(d, f, &rep)?);
    }
    Ok((out, s))
}
