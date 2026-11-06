//! Part 3b-4: water budget closure, seasonal runs, Earth-like validation of the whole moisture stage.
//! (The PART file calls this tests/part3b.rs; it is named part3b4.rs to match part3.rs / part3b1..3.rs.)
use enbuilder::generators::*;
use enbuilder::*;

const YEAR: f64 = 3.15576e7;
const DAY: f64 = 86400.0;
// Season 0 and 0.5 are both equinoxes (declination = asin(sin(tilt) sin(2 pi season))), so the two "seasons"
// are the solstices.
const SUMMER: f64 = 0.25;
const WINTER: f64 = 0.75;

fn ridge_axis() -> [f64; 3] {
    let a = [0.3_f64, 0.5, 0.8];
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

fn cap_cos(up: [f64; 3]) -> f64 {
    let a = ridge_axis();
    up[0] * a[0] + up[1] * a[1] + up[2] * a[2]
}

/// ~30% land cap with a 3000 m ridge ring through it, rest ocean.
fn earth_elevation(d: &Domain) -> Vec<f64> {
    d.mesh.up.iter().map(|&u| {
        let c = cap_cos(u);
        if c > 0.4 { 100.0 + 3000.0 * (-((c - 0.7).powi(2)) / (2.0 * 0.05f64.powi(2))).exp() } else { -3000.0 }
    }).collect()
}

fn earth_sphere(sub: u32) -> Domain {
    let mut d = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: 9.81, seed: 7 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 23.4f64.to_radians()));
    d
}

fn setup(d: &Domain, z: Vec<f64>) -> (FieldStore, Scheduler) {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z).unwrap();
    seed_temperature(&mut f, 288.0).unwrap();
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    (f, s)
}

fn zonal_mean(d: &Domain, x: &[f64], lo: f64, hi: f64) -> f64 {
    let (mut s, mut a) = (0.0, 0.0);
    for i in 0..d.n_cells() {
        let lat = d.mesh.up[i][2].asin().to_degrees().abs();
        if lat >= lo && lat < hi {
            s += x[i] * d.mesh.area[i];
            a += d.mesh.area[i];
        }
    }
    s / a
}

fn all_finite_and_sane(f: &FieldStore) {
    for name in f.names() {
        let v = f.data(&name).unwrap();
        assert!(v.iter().all(|x| x.is_finite()), "field {name} has non-finite values");
    }
    for name in ["vapor", "vapor_start", "evaporation", "precipitation", "precip_oro", "precip_conv", "precip_front", "precip_sat", "lift"] {
        assert!(f.data(name).unwrap().iter().all(|&x| x >= 0.0), "{name} negative");
    }
}

fn two_season_budget(d: &Domain, z: Vec<f64>, label: &str) {
    let (mut f, mut s) = setup(d, z);
    let mut totals = Vec::new();
    for season in [SUMMER, WINTER] {
        let r = run_season(d, &mut f, &mut s, season).unwrap();
        assert!(r.budget.residual_rel < 1e-8, "{label} season {season}: residual {}", r.budget.residual_rel);
        assert!(r.budget.evaporation_kg > 0.0 && r.budget.precipitation_kg > 0.0, "{label}: no water cycle");
        all_finite_and_sane(&f);
        totals.push(r.budget);
    }
    // two-season totals
    let resid: f64 = totals.iter().map(|b| b.residual_kg).sum();
    let scale: f64 = totals.iter().map(|b| b.vapor_start_kg + b.evaporation_kg).sum();
    assert!(resid.abs() / scale < 1e-8, "{label}: two-season residual {}", resid / scale);
}

#[test]
fn budget_closes_on_sphere() {
    let d = earth_sphere(3);
    two_season_budget(&d, earth_elevation(&d), "sphere");
}

#[test]
fn budget_closes_on_cube() {
    let mut d = generate_cube(&CubeParams { half_side: 3.2e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 3 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 23.4f64.to_radians()));
    let z: Vec<f64> = d.mesh.up.iter().map(|u| if u[2] > 0.3 { 200.0 + 2000.0 * u[0].abs() } else { -3000.0 }).collect();
    two_season_budget(&d, z, "cube");
}

#[test]
fn budget_closes_on_plane() {
    let mut d = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 48, ny: 24 },
        cell_size: 150_000.0,
        boundary: BoundaryKind::Wrap,
        surface_gravity: 9.81,
        seed: 5,
    })
    .unwrap();
    let h = 4.0e6;
    d.lights.push(Light::point([0.0, 0.0, h], 1361.0, h));
    let z: Vec<f64> = d.mesh.centroid.iter().map(|c| if c[0] > 0.0 { 150.0 + 1500.0 * (-(c[0] / 1.5e6).powi(2)).exp() } else { -3000.0 }).collect();
    two_season_budget(&d, z, "plane");
}

#[test]
fn earth_like_sphere_water_cycle() {
    let d = earth_sphere(4);
    let n = d.n_cells();
    let (mut f, mut s) = setup(&d, earth_elevation(&d));
    let mut means = Vec::new();
    let (mut tropics, mut subtropics) = (0.0, 0.0);
    let mut fields_by_season: Vec<Vec<f64>> = Vec::new();
    for season in [0.0, SUMMER, WINTER] {
        let r = run_season(&d, &mut f, &mut s, season).unwrap();
        let b = r.budget;
        assert!(b.residual_rel < 1e-8);
        // one pass per season is a converged water cycle: vapour storage is a small part of the flux
        assert!(b.storage_fraction.abs() < 0.15, "season {season}: storage fraction {}", b.storage_fraction);
        assert!(r.mean_precip_mm_day.is_finite() && r.mean_precip_mm_day > 0.5 && r.mean_precip_mm_day < 8.0, "season {season}: P = {}", r.mean_precip_mm_day);
        all_finite_and_sane(&f);
        let p = f.data("precipitation").unwrap().to_vec();
        // equinox only: with no thermal inertia (Part 2) the solstice summer hemisphere is extreme and its monsoon
        // reaches 30 degrees, so the tropics/subtropics contrast is only meaningful at season 0.
        if season == 0.0 {
            tropics = zonal_mean(&d, &p, 0.0, 15.0) * DAY;
            subtropics = zonal_mean(&d, &p, 20.0, 35.0) * DAY;
        }
        means.push(r.mean_precip_mm_day);
        fields_by_season.push(p);
    }
    let annual = means.iter().sum::<f64>() / means.len() as f64;
    assert!((2.0..=4.0).contains(&annual), "annual-ish global mean precipitation {annual} mm/day (seasons {means:?})");
    assert!(tropics > 1.5 * subtropics, "tropics {tropics} vs subtropics {subtropics}");

    // seasons differ in distribution
    let (a, b) = (&fields_by_season[1], &fields_by_season[2]);
    let l1: f64 = (0..n).map(|i| (a[i] - b[i]).abs() * d.mesh.area[i]).sum();
    let tot: f64 = (0..n).map(|i| (a[i] + b[i]) * d.mesh.area[i]).sum();
    assert!(l1 / tot > 0.2, "seasons barely differ: {}", l1 / tot);
}

#[test]
fn earth_like_ridge_shows_rain_shadow() {
    let d = earth_sphere(4);
    let n = d.n_cells();
    let (mut f, mut s) = setup(&d, earth_elevation(&d));
    run_season(&d, &mut f, &mut s, SUMMER).unwrap();
    let (lift, po, p, z) = (f.data("lift").unwrap(), f.data("precip_oro").unwrap(), f.data("precipitation").unwrap(), f.data("elevation").unwrap());
    let band: Vec<usize> = (0..n).filter(|&i| cap_cos(d.mesh.up[i]) > 0.55 && cap_cos(d.mesh.up[i]) < 0.85 && z[i] > 0.0).collect();
    let (wi, le): (Vec<usize>, Vec<usize>) = band.iter().partition(|&&i| lift[i] > 0.0);
    assert!(wi.len() > 20 && le.len() > 20);
    let mean = |idx: &[usize], x: &[f64]| idx.iter().map(|&i| x[i]).sum::<f64>() / idx.len() as f64;
    assert!(mean(&wi, po) > 0.0 && le.iter().all(|&i| po[i] == 0.0));
    assert!(mean(&wi, p) > 1.2 * mean(&le, p), "windward {} leeward {}", mean(&wi, p) * DAY, mean(&le, p) * DAY);
}

#[test]
fn smoke_all_fields_finite_on_all_generators() {
    // cold start and warm start through the same pipeline, three topologies
    let sph = earth_sphere(2);
    let cube = {
        let mut d = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 8, surface_gravity: 9.81, seed: 1 }).unwrap();
        d.lights.push(Light::directional([0.0, 1.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 0.3));
        d
    };
    let plane = {
        let mut d = generate_plane(&PlaneParams { shape: PlaneShape::Disc { diameter_cells: 30 }, cell_size: 100_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap();
        d.lights.push(Light::point([0.0, 0.0, 3.0e6], 1361.0, 3.0e6));
        d
    };
    for (name, d) in [("sphere", sph), ("cube", cube), ("plane", plane)] {
        let z: Vec<f64> = (0..d.n_cells()).map(|i| if i % 3 == 0 { 800.0 } else { -2000.0 }).collect();
        let (mut f, mut s) = setup(&d, z);
        for season in [0.0, 0.3] {
            run_season(&d, &mut f, &mut s, season).unwrap();
            all_finite_and_sane(&f);
        }
        s.set_param("moisture_transport", "warm_start", 1.0).unwrap();
        let r = run_season(&d, &mut f, &mut s, 0.6).unwrap();
        assert!(r.budget.residual_rel < 1e-8, "{name} warm start residual");
        all_finite_and_sane(&f);
    }
}

#[test]
fn deterministic_two_season_run() {
    let d = earth_sphere(3);
    let go = || {
        let (mut f, mut s) = setup(&d, earth_elevation(&d));
        run_season(&d, &mut f, &mut s, SUMMER).unwrap();
        run_season(&d, &mut f, &mut s, WINTER).unwrap();
        f.data("precipitation").unwrap().to_vec()
    };
    assert_eq!(go(), go());
}
