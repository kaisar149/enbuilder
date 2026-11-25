//! Building a world from parameters, and the solve pipeline. Pure functions: the same parameters give the same world.
use crate::png::fnv1a;
use enbuilder::biomes::{classify_biomes, BiomeTable};
use enbuilder::generators::{generate_cube, generate_plane, generate_sphere, CubeParams, PlaneParams, PlaneShape, SphereParams};
use enbuilder::noise::fbm3;
use enbuilder::{BoundaryKind, Domain, FieldStore, Light, Scheduler};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SEASONS: usize = 12;
const DAY_S: f64 = 86400.0;

pub type Params = BTreeMap<String, f64>;

fn g(p: &Params, k: &str) -> f64 { *p.get(k).unwrap_or_else(|| panic!("missing parameter {k}")) }

pub fn shape_name(p: &Params) -> &'static str {
    match g(p, "shape") as i64 { 1 => "cube", 2 => "plane", _ => "sphere" }
}

/// Identifies the mesh: only shape, resolution and size change it.
pub fn mesh_id(p: &Params) -> String {
    let s = format!("{}|{}|{}", g(p, "shape"), g(p, "resolution"), g(p, "radius_km"));
    format!("m-{:016x}", fnv1a(s.as_bytes()))
}

/// Domain (mesh, gravity, light) for the parameters. No elevation yet.
pub fn build_domain(p: &Params) -> enbuilder::Result<Domain> {
    let r = g(p, "radius_km") * 1000.0;
    let res = g(p, "resolution") as usize;
    let seed = g(p, "seed") as u64;
    let grav = g(p, "surface_gravity");
    let mut d = match g(p, "shape") as i64 {
        1 => generate_cube(&CubeParams { half_side: r, cells_per_edge: 4 * res, surface_gravity: grav, seed })?,
        2 => {
            let nx = 16 * res;
            generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny: 8 * res }, cell_size: 2.0 * std::f64::consts::PI * r / nx as f64, boundary: BoundaryKind::Wrap, surface_gravity: grav, seed })?
        }
        _ => generate_sphere(&SphereParams { radius: r, subdivision: res as u32, surface_gravity: grav, seed })?,
    };
    let year_s = g(p, "year_length_days") * DAY_S;
    let day_s = g(p, "day_length_h") * 3600.0;
    d.lights.push(Light::directional([1.0, 0.0, 0.0], g(p, "solar_constant")).with_orbit(year_s, day_s, g(p, "axial_tilt_deg").to_radians()));
    Ok(d)
}

/// Terrain from fractal noise on the cell positions; the sea level is chosen so that `ocean_fraction` of the AREA is ocean.
pub fn build_elevation(d: &Domain, p: &Params) -> Vec<f64> {
    let r = g(p, "radius_km") * 1000.0;
    let seed = g(p, "seed") as u64;
    let q: Vec<f64> = d.mesh.centroid.iter().map(|c| fbm3([c[0] / r * 2.0, c[1] / r * 2.0, c[2] / r * 2.0], 5, seed)).collect();
    let mut order: Vec<usize> = (0..q.len()).collect();
    order.sort_by(|&a, &b| q[a].partial_cmp(&q[b]).unwrap().then(a.cmp(&b)));
    let total: f64 = d.mesh.area.iter().sum();
    let target = g(p, "ocean_fraction") * total;
    let (mut acc, mut thr_i) = (0.0, 0usize);
    for (k, &i) in order.iter().enumerate() {
        if acc >= target {
            break;
        }
        acc += d.mesh.area[i];
        thr_i = k + 1;
    }
    let (lo, hi) = (q[order[0]], q[*order.last().unwrap()]);
    let thr = if thr_i == 0 { lo - 1e-9 } else if thr_i >= order.len() { hi + 1e-9 } else { 0.5 * (q[order[thr_i - 1]] + q[order[thr_i]]) };
    q.iter()
        .map(|&v| {
            if v > thr {
                let t = ((v - thr) / (hi - thr).max(1e-9)).clamp(0.0, 1.0);
                50.0 + 3500.0 * t.powf(1.3)
            } else {
                let t = ((thr - v) / (thr - lo).max(1e-9)).clamp(0.0, 1.0);
                -50.0 - 4500.0 * t.powf(0.8)
            }
        })
        .collect()
}

pub fn build_world(p: &Params) -> enbuilder::Result<(Domain, FieldStore)> {
    let d = build_domain(p)?;
    let z = build_elevation(&d, p);
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z)?;
    enbuilder::seed_temperature(&mut f, 288.0)?;
    Ok((d, f))
}

pub fn biome_table() -> BiomeTable { BiomeTable::earth_like() }

/// A scheduler with the whole solve pipeline registered (also used, never run, by the schema export).
pub fn pipeline() -> enbuilder::Result<Scheduler> {
    let mut s = Scheduler::new();
    enbuilder::add_land_water_pipeline(&mut s)?;
    Ok(s)
}

pub struct Solved {
    pub domain: Domain,
    pub fields: FieldStore,
    pub table: BiomeTable,
}

pub enum SolveError {
    Cancelled,
    Failed(String),
}

impl From<enbuilder::Error> for SolveError {
    fn from(e: enbuilder::Error) -> Self { SolveError::Failed(e.to_string()) }
}

/// Full solve: spin up `spinup_years` years of 12 seasons, take the last year as the climate normals, classify biomes, and publish the
/// annual means as `temperature` (K) and `precipitation` (kg/m^2/s). `progress(fraction, stage)` is called between steps; `cancel` is
/// checked between steps (one step = one season of the full climate loop).
pub fn solve(p: &Params, cancel: &AtomicBool, progress: &mut dyn FnMut(f64, &str)) -> Result<Solved, SolveError> {
    let (domain, mut f) = build_world(p)?;
    let mut s = pipeline()?;
    enbuilder::seed_land_state(&mut f, 0.075)?;
    let years = g(p, "spinup_years") as usize;
    let total = (years * SEASONS + 1) as f64;
    let mut done = 0.0;
    for year in 0..years {
        for k in 0..SEASONS {
            if cancel.load(Ordering::SeqCst) {
                return Err(SolveError::Cancelled);
            }
            let season = (k as f64 + 0.5) / SEASONS as f64;
            enbuilder::run_land_cycle(&domain, &mut f, &mut s, &[season])?;
            // normals come from the last year only
            if year + 1 == years {
                enbuilder::capture_season(&mut f, k, SEASONS)?;
            }
            done += 1.0;
            progress(done / total, &format!("year {} season {}", year + 1, k + 1));
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(SolveError::Cancelled);
    }
    let table = biome_table();
    classify_biomes(&domain, &mut f, &table)?;
    let n = domain.n_cells();
    let mean = |name: &str| -> Result<Vec<f64>, SolveError> {
        let l = f.layers(name)?;
        let d = f.data(name)?;
        Ok((0..n).map(|i| (0..l).map(|k| d[i * l + k]).sum::<f64>() / l as f64).collect())
    };
    let (t, pr) = (mean("season_temperature")?, mean("season_precipitation")?);
    f.put("temperature", 1, t)?;
    f.put("precipitation", 1, pr)?;
    progress(1.0, "done");
    Ok(Solved { domain, fields: f, table })
}
