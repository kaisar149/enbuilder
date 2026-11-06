//! Part 3a acceptance tests: diagnostic pressure and wind (stage C, air side).
use enbuilder::generators::*;
use enbuilder::*;

const YEAR: f64 = 3.15576e7;
const DAY: f64 = 86400.0;
const OMEGA: f64 = 2.0 * std::f64::consts::PI / 86164.0;

// ------------------------------------------------------------ helpers

fn sphere(sub: u32, g: f64) -> Domain {
    generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: g, seed: 7 }).unwrap()
}

fn lat_deg(up: [f64; 3]) -> f64 { up[2].clamp(-1.0, 1.0).asin().to_degrees() }

/// Earth-like zonal temperature: equator 300 K, poles 260 K.
fn earth_t(u: [f64; 3]) -> f64 { 300.0 - 40.0 * u[2] * u[2] }

fn atmos_fields(d: &Domain, t: impl Fn([f64; 3]) -> f64, z: impl Fn([f64; 3]) -> f64) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    let tsl: Vec<f64> = d.mesh.up.iter().map(|&u| t(u)).collect();
    f.put("temperature_sl", 1, tsl.clone()).unwrap();
    f.put("temperature", 1, tsl).unwrap();
    f.put("elevation", 1, d.mesh.up.iter().map(|&u| z(u)).collect()).unwrap();
    f
}

fn run_atmosphere(d: &Domain, f: &mut FieldStore, params: &[(&str, &str, f64)]) {
    let mut s = Scheduler::new();
    add_atmosphere_stage(&mut s).unwrap();
    for (m, k, v) in params {
        s.set_param(m, k, *v).unwrap();
    }
    s.run(d, f, &DirtyRegion::All).unwrap();
}

fn ocean(_u: [f64; 3]) -> f64 { -3000.0 }

fn count_extrema(d: &Domain, p: &[f64]) -> usize {
    let nb = 30;
    let (mut sum, mut cnt) = (vec![0.0; nb], vec![0usize; nb]);
    for i in 0..d.n_cells() {
        let b = ((lat_deg(d.mesh.up[i]).abs() / 3.0) as usize).min(nb - 1);
        sum[b] += p[i];
        cnt[b] += 1;
    }
    let zp: Vec<f64> = (0..nb).filter(|&b| cnt[b] > 0).map(|b| sum[b] / cnt[b] as f64).collect();
    let dz: Vec<f64> = zp.windows(2).map(|w| w[1] - w[0]).filter(|x| x.abs() > 1.0).collect();
    dz.windows(2).filter(|w| w[0] * w[1] < 0.0).count()
}

fn zonal_east_means(d: &Domain, f: &FieldStore) -> [f64; 3] {
    let we = f.data("wind_east").unwrap();
    let (mut s, mut c) = ([0.0; 3], [0usize; 3]);
    for i in 0..d.n_cells() {
        let l = lat_deg(d.mesh.up[i]).abs();
        let b = if l < 30.0 { 0 } else if l < 60.0 { 1 } else { 2 };
        s[b] += we[i];
        c[b] += 1;
    }
    [s[0] / c[0] as f64, s[1] / c[1] as f64, s[2] / c[2] as f64]
}

fn area_mean(d: &Domain, x: &[f64]) -> f64 {
    x.iter().zip(d.mesh.area.iter()).map(|(a, b)| a * b).sum::<f64>() / d.mesh.total_area()
}

// ------------------------------------------------------------ pressure

#[test]
fn sea_level_pressure_keeps_its_mean_and_colder_air_is_higher() {
    let d = sphere(3, 9.81);
    let mut f = atmos_fields(&d, earth_t, ocean);
    run_atmosphere(&d, &mut f, &[("pressure", "rotation_period_s", 0.0)]); // no bands: thermal part only
    let psl = f.data("pressure_sl").unwrap();
    assert!((area_mean(&d, psl) / 101325.0 - 1.0).abs() < 1e-9, "mean {}", area_mean(&d, psl));
    let (mut eq, mut pole) = (0.0, 0.0);
    let (mut ce, mut cp) = (0, 0);
    for i in 0..d.n_cells() {
        let z = d.mesh.up[i][2].abs();
        if z < 0.2 { eq += psl[i]; ce += 1; }
        if z > 0.9 { pole += psl[i]; cp += 1; }
    }
    assert!(pole / cp as f64 > eq / ce as f64 + 300.0, "thermal low at the warm equator");
}

#[test]
fn surface_pressure_is_hydrostatic_and_depends_on_gravity() {
    let hi = |u: [f64; 3]| if u[0] > 0.5 { 3000.0 } else { -3000.0 };
    let (d1, d2) = (sphere(3, 9.81), sphere(3, 25.0));
    let (mut f1, mut f2) = (atmos_fields(&d1, earth_t, hi), atmos_fields(&d2, earth_t, hi));
    run_atmosphere(&d1, &mut f1, &[]);
    run_atmosphere(&d2, &mut f2, &[]);
    for (d, f, g) in [(&d1, &f1, 9.81), (&d2, &f2, 25.0)] {
        let (psl, ps) = (f.data("pressure_sl").unwrap(), f.data("pressure").unwrap());
        let t = f.data("temperature").unwrap();
        for i in 0..d.n_cells() {
            if d.mesh.up[i][0] > 0.5 {
                let expect = psl[i] * (-g * 3000.0 / (287.05 * t[i])).exp();
                assert!((ps[i] - expect).abs() < 1e-6 * expect, "cell {i}");
                assert!(ps[i] < psl[i]);
            } else {
                assert!((ps[i] - psl[i]).abs() < 1e-9, "ocean surface sits at sea level");
            }
        }
    }
    let i = (0..d1.n_cells()).find(|&i| d1.mesh.up[i][0] > 0.6).unwrap();
    assert!(f2.data("pressure").unwrap()[i] < f1.data("pressure").unwrap()[i], "stronger gravity, thinner air aloft");
    let rho = f1.data("air_density").unwrap();
    assert!(rho.iter().all(|&r| r > 0.2 && r < 2.0));
}

#[test]
fn coriolis_comes_from_spin_axis_not_latitude() {
    let d = sphere(3, 9.81);
    let mut f = atmos_fields(&d, earth_t, ocean);
    run_atmosphere(&d, &mut f, &[]);
    let cf = f.data("coriolis_f").unwrap();
    for i in 0..d.n_cells() {
        assert!((cf[i] - 2.0 * OMEGA * d.mesh.up[i][2]).abs() < 1e-12);
    }
    // flat world: spin axis lies in the plane, so f = 0 unless an f-plane value is given
    let p = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 20, ny: 10 }, cell_size: 5.0e4, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1,
    })
    .unwrap();
    let mut fp = atmos_fields(&p, |_| 285.0, ocean);
    run_atmosphere(&p, &mut fp, &[]);
    assert!(fp.data("coriolis_f").unwrap().iter().all(|&v| v == 0.0));
    let mut fq = atmos_fields(&p, |_| 285.0, ocean);
    run_atmosphere(&p, &mut fq, &[("pressure", "coriolis_const", 1.0e-4)]);
    assert!(fq.data("coriolis_f").unwrap().iter().all(|&v| (v - 1.0e-4).abs() < 1e-18));
    let mut fz = atmos_fields(&d, earth_t, ocean);
    run_atmosphere(&d, &mut fz, &[("pressure", "coriolis_scale", 0.0)]);
    assert!(fz.data("coriolis_f").unwrap().iter().all(|&v| v == 0.0));
}

// ------------------------------------------------------------ wind (acceptance)

#[test]
fn circulation_band_count_grows_with_rotation_rate() {
    let d = sphere(4, 9.81);
    let mut counts = Vec::new();
    for period in [86164.0 * 4.0, 86164.0, 86164.0 / 3.0] {
        let mut f = atmos_fields(&d, earth_t, ocean);
        run_atmosphere(&d, &mut f, &[("pressure", "rotation_period_s", period)]);
        counts.push(count_extrema(&d, f.data("pressure_sl").unwrap()));
    }
    assert!(counts[0] <= 1, "slow rotation: one big cell, got {counts:?}");
    assert!((2..=4).contains(&counts[1]), "Earth-like: a few bands, got {counts:?}");
    assert!(counts[2] >= 7, "fast rotation: many narrow bands, got {counts:?}");
}

#[test]
fn earth_like_wind_has_trades_and_westerlies() {
    let d = sphere(4, 9.81);
    let mut f = atmos_fields(&d, earth_t, ocean);
    run_atmosphere(&d, &mut f, &[]);
    let [trades, mid, polar] = zonal_east_means(&d, &f);
    assert!(trades < -5.0, "trade-wind belt should be easterly, got {trades}");
    assert!(mid > 0.8, "mid-latitude belt should be westerly, got {mid}");
    assert!(polar < -1.0, "polar belt should be easterly, got {polar}");
    let sp = f.data("wind_speed").unwrap();
    let mean = sp.iter().sum::<f64>() / sp.len() as f64;
    assert!((3.0..15.0).contains(&mean), "mean surface wind {mean} m/s");
    assert!(sp.iter().all(|&s| s.is_finite() && s <= 80.0));
    // wind is tangent to the surface
    let w = f.data("wind").unwrap();
    for i in 0..d.n_cells() {
        let v = [w[3 * i], w[3 * i + 1], w[3 * i + 2]];
        let radial = v[0] * d.mesh.up[i][0] + v[1] * d.mesh.up[i][1] + v[2] * d.mesh.up[i][2];
        assert!(radial.abs() < 1e-9 * (1.0 + sp[i]));
    }
}

#[test]
fn tidally_locked_world_flows_night_to_day_at_the_surface_and_back_aloft() {
    let d = sphere(4, 9.81);
    let temp = |u: [f64; 3]| 240.0 + 60.0 * u[0].max(0.0); // substellar point at +x
    for period in [0.0, 30.0 * DAY] {
        let mut f = atmos_fields(&d, temp, ocean);
        run_atmosphere(&d, &mut f, &[("pressure", "rotation_period_s", period)]);
        let (w, wu) = (f.data("wind").unwrap(), f.data("wind_upper").unwrap());
        let (mut s, mut u, mut c) = (0.0, 0.0, 0usize);
        for i in 0..d.n_cells() {
            let up = d.mesh.up[i];
            let cs = up[0];
            if cs.abs() > 0.99 { continue; }
            // unit tangent direction toward the substellar point
            let dir = [1.0 - up[0] * cs, -up[1] * cs, -up[2] * cs];
            let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let comp = |v: &[f64]| (v[3 * i] * dir[0] + v[3 * i + 1] * dir[1] + v[3 * i + 2] * dir[2]) / l;
            s += comp(w);
            u += comp(wu);
            c += 1;
        }
        let (s, u) = (s / c as f64, u / c as f64);
        assert!(s > 1.0, "period {period}: surface wind should converge on the day side, got {s}");
        assert!(u < -2.0, "period {period}: upper wind should flow day to night, got {u}");
    }
}

#[test]
fn mountain_range_blocks_and_slows_wind_that_climbs_it() {
    let d = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 60, ny: 20 }, cell_size: 5.0e4, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1,
    })
    .unwrap();
    let n = d.n_cells();
    let x: Vec<f64> = d.mesh.centroid.iter().map(|c| c[0]).collect();
    let y: Vec<f64> = d.mesh.centroid.iter().map(|c| c[1]).collect();
    let z: Vec<f64> = x.iter().map(|&xx| if xx.abs() < 2.5e5 { 3000.0 * (1.0 - xx.abs() / 2.5e5) } else { 10.0 }).collect();
    let run = |gain: f64| -> Vec<f64> {
        let mut f = FieldStore::new(n);
        f.put("elevation", 1, z.clone()).unwrap();
        f.put("pressure_sl", 1, x.iter().map(|xx| 101325.0 - 1.0e-3 * xx).collect()).unwrap();
        f.put("pressure_upper", 1, vec![5.0e4; n]).unwrap();
        f.put("air_density", 1, vec![1.2; n]).unwrap();
        f.put("coriolis_f", 1, vec![0.0; n]).unwrap();
        let mut s = Scheduler::new();
        s.register(Box::new(DiagnosticWind::new())).unwrap();
        s.set_param("wind", "blocking_gain", gain).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (0..n).map(|i| f.data("wind").unwrap()[3 * i]).collect()
    };
    let (free, blocked) = (run(0.0), run(1.0));
    let mean = |u: &[f64], lo: f64, hi: f64| {
        let ids: Vec<usize> = (0..n).filter(|&i| x[i] > lo && x[i] < hi && y[i].abs() < 1.0e5).collect();
        ids.iter().map(|&i| u[i]).sum::<f64>() / ids.len() as f64
    };
    // far upwind: identical, and equal to the drag-limited value
    assert!((mean(free.as_slice(), -1.0e6, -6.0e5) - mean(blocked.as_slice(), -1.0e6, -6.0e5)).abs() < 1e-6);
    assert!((mean(free.as_slice(), -1.0e6, -6.0e5) - 26.7).abs() < 0.5);
    // windward slope: blocking removes a good part of the up-slope flow
    let (wf, wb) = (mean(free.as_slice(), -2.3e5, -0.3e5), mean(blocked.as_slice(), -2.3e5, -0.3e5));
    assert!(wb < 0.9 * wf, "windward slope: free {wf}, blocked {wb}");
    assert!(wb > 0.0, "flow is steered, not reversed");
    // downwind slope is not blocked (flow descends)
    assert!((mean(free.as_slice(), 0.3e5, 2.3e5) - mean(blocked.as_slice(), 0.3e5, 2.3e5)).abs() < 1e-6);
}

// ------------------------------------------------------------ robustness

#[test]
fn no_atmosphere_means_no_pressure_and_no_wind() {
    let mut d = sphere(3, 9.81);
    d.media.atmosphere = None;
    let mut f = atmos_fields(&d, earth_t, ocean);
    run_atmosphere(&d, &mut f, &[]);
    for name in ["pressure_sl", "pressure", "air_density", "wind_speed"] {
        assert!(f.data(name).unwrap().iter().all(|&v| v == 0.0), "{name}");
    }
    assert!(f.data("wind").unwrap().iter().all(|&v| v == 0.0));
}

#[test]
fn runs_on_cube_and_wrapped_plane_and_is_deterministic() {
    let cube = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 12, surface_gravity: 9.81, seed: 1 }).unwrap();
    let torus = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 40, ny: 20 }, cell_size: 1.0e5, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 2,
    })
    .unwrap();
    let mk = |d: &Domain| {
        let mut f = FieldStore::new(d.n_cells());
        let t: Vec<f64> = d.mesh.centroid.iter().map(|c| 290.0 + 8.0 * (c[0] / 1.0e6).sin() + 5.0 * (c[1] / 1.0e6).cos()).collect();
        f.put("temperature_sl", 1, t.clone()).unwrap();
        f.put("temperature", 1, t).unwrap();
        f.put("elevation", 1, vec![-3000.0; d.n_cells()]).unwrap();
        f
    };
    let cube_params: Vec<(&str, &str, f64)> = vec![];
    let torus_params: Vec<(&str, &str, f64)> = vec![("pressure", "coriolis_const", 5.0e-5)];
    for (d, params) in [(&cube, &cube_params), (&torus, &torus_params)] {
        let (mut a, mut b) = (mk(d), mk(d));
        run_atmosphere(d, &mut a, params);
        run_atmosphere(d, &mut b, params);
        let sp = a.data("wind_speed").unwrap();
        assert!(sp.iter().all(|&s| s.is_finite() && s <= 80.0));
        assert!(sp.iter().sum::<f64>() > 0.0, "some wind");
        assert_eq!(a.data("wind").unwrap(), b.data("wind").unwrap());
    }
}

#[test]
fn full_pipeline_energy_then_atmosphere() {
    let mut d = sphere(3, 9.81);
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 0.0));
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, d.mesh.up.iter().map(|u| if u[2] > 0.7 { 500.0 } else { -3000.0 }).collect()).unwrap();
    seed_temperature(&mut f, 288.0).unwrap();
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    add_atmosphere_stage(&mut s).unwrap();
    let r = s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(r.loops[0].converged);
    let order = s.plan_order().unwrap();
    let pos = |n: &str| order.iter().position(|m| m == n).unwrap();
    assert!(pos("pressure") < pos("wind"));
    let sp = f.data("wind_speed").unwrap();
    let mean = sp.iter().sum::<f64>() / sp.len() as f64;
    assert!((1.0..20.0).contains(&mean), "mean wind {mean}");
    assert!(f.has("band_coord") && f.has("pressure_upper") && f.has("wind_upper"));
}
