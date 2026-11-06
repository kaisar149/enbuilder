//! Part 3c-1 acceptance tests: stepped shallow-layer wind (CPU reference).
use enbuilder::generators::*;
use enbuilder::*;

const DAY: f64 = 86400.0;

fn sphere(sub: u32) -> Domain {
    generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: 9.81, seed: 7 }).unwrap()
}
fn lat_deg(up: [f64; 3]) -> f64 { up[2].clamp(-1.0, 1.0).asin().to_degrees() }
fn earth_t(u: [f64; 3]) -> f64 { 300.0 - 40.0 * u[2] * u[2] }

fn atmos_fields(d: &Domain, t: impl Fn([f64; 3]) -> f64, z: impl Fn([f64; 3]) -> f64) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    let tsl: Vec<f64> = d.mesh.up.iter().map(|&u| t(u)).collect();
    f.put("temperature_sl", 1, tsl.clone()).unwrap();
    f.put("temperature", 1, tsl).unwrap();
    f.put("elevation", 1, d.mesh.up.iter().map(|&u| z(u)).collect()).unwrap();
    f
}
fn ocean(_u: [f64; 3]) -> f64 { -3000.0 }

/// Runs the stepped (true) or diagnostic (false) atmosphere; returns the scheduler for `module_param`.
fn run(d: &Domain, f: &mut FieldStore, stepped: bool, params: &[(&str, &str, f64)]) -> Scheduler {
    let mut s = Scheduler::new();
    if stepped { add_atmosphere_stage_stepped(&mut s).unwrap() } else { add_atmosphere_stage(&mut s).unwrap() }
    for (m, k, v) in params {
        s.set_param(m, k, *v).unwrap();
    }
    s.run(d, f, &DirtyRegion::All).unwrap();
    s
}

/// Mean of `wind_east` in |lat| bands of `width` degrees (index 0 = equator band).
fn zonal_bands(d: &Domain, f: &FieldStore, width: f64) -> Vec<f64> {
    let nb = (90.0 / width) as usize;
    let we = f.data("wind_east").unwrap();
    let (mut s, mut c) = (vec![0.0; nb], vec![0usize; nb]);
    for i in 0..d.n_cells() {
        let b = ((lat_deg(d.mesh.up[i]).abs() / width) as usize).min(nb - 1);
        s[b] += we[i];
        c[b] += 1;
    }
    (0..nb).map(|b| if c[b] > 0 { s[b] / c[b] as f64 } else { 0.0 }).collect()
}
fn three_cell_means(d: &Domain, f: &FieldStore) -> [f64; 3] {
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
/// Number of sign changes of the zonal-mean east wind from equator to pole (dead band 0.3 m/s).
fn band_count(d: &Domain, f: &FieldStore) -> usize {
    let z = zonal_bands(d, f, 3.0);
    let mut last = 0.0;
    let mut n = 0;
    for v in z {
        if v.abs() < 0.3 { continue; }
        if last != 0.0 && v * last < 0.0 { n += 1; }
        last = v;
    }
    n
}
fn max_diff(a: &FieldStore, b: &FieldStore, name: &str) -> f64 {
    a.data(name).unwrap().iter().zip(b.data(name).unwrap()).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}

#[test]
fn converges_without_blocking_or_terrain_on_levels_3_and_4() {
    for lvl in [3, 4] {
        let d = sphere(lvl);
        let mut f = atmos_fields(&d, earth_t, ocean);
        let s = run(&d, &mut f, true, &[("wind", "blocking_gain", 0.0)]);
        let steps = s.module_param("wind", "last_steps").unwrap();
        let resid = s.module_param("wind", "last_residual").unwrap();
        println!("L{lvl}: converged in {steps} steps, last max |du| per step {resid:.2e} m/s");
        assert!(resid <= 1e-3, "residual {resid}");
        assert!(steps > 10.0 && steps < 20000.0, "steps {steps}");
    }
}

#[test]
fn stepped_steady_state_matches_the_diagnostic_wind_on_an_ocean_sphere() {
    for lvl in [3, 4] {
        let d = sphere(lvl);
        let (mut a, mut b) = (atmos_fields(&d, earth_t, ocean), atmos_fields(&d, earth_t, ocean));
        run(&d, &mut a, false, &[("wind", "blocking_gain", 0.0)]);
        run(&d, &mut b, true, &[("wind", "blocking_gain", 0.0)]);
        let (da, sb) = (three_cell_means(&d, &a), three_cell_means(&d, &b));
        println!("L{lvl} zonal means diag {da:?} stepped {sb:?} max |dspeed| {:.3}", max_diff(&a, &b, "wind_speed"));
        for k in 0..3 {
            assert_eq!(da[k].signum(), sb[k].signum(), "sign of cell {k}");
            assert!((sb[k] / da[k] - 1.0).abs() < 0.2, "cell {k}: diag {} stepped {}", da[k], sb[k]);
        }
        assert!(sb[0] < -5.0 && sb[1] > 0.8 && sb[2] < -1.0);
        // per-cell agreement is much tighter than the 20% zonal-mean requirement
        assert!(max_diff(&a, &b, "wind_speed") < 0.2, "max per-cell difference");
        assert!(max_diff(&a, &b, "wind_upper") < 1e-12, "upper level is the diagnostic one");
    }
}

#[test]
fn steady_state_does_not_depend_on_the_time_step() {
    let d = sphere(3);
    let mut res = Vec::new();
    for dt in [150.0, 600.0, 3000.0] {
        let mut f = atmos_fields(&d, earth_t, ocean);
        run(&d, &mut f, true, &[("wind", "blocking_gain", 0.0), ("wind", "dt_max_s", dt), ("wind", "cfl_wind", 50.0)]);
        res.push(f);
    }
    assert!(max_diff(&res[0], &res[1], "wind_speed") < 0.3);
    assert!(max_diff(&res[1], &res[2], "wind_speed") < 0.3, "{}", max_diff(&res[1], &res[2], "wind_speed"));
}

#[test]
fn tidally_locked_sphere_flows_toward_the_substellar_point_at_the_surface() {
    let d = sphere(4);
    let temp = |u: [f64; 3]| 240.0 + 60.0 * u[0].max(0.0);
    for period in [0.0, 30.0 * DAY] {
        let mut comp = [0.0; 2];
        for (k, stepped) in [false, true].into_iter().enumerate() {
            let mut f = atmos_fields(&d, temp, ocean);
            run(&d, &mut f, stepped, &[("pressure", "rotation_period_s", period)]);
            let w = f.data("wind").unwrap();
            let (mut s, mut c) = (0.0, 0usize);
            for i in 0..d.n_cells() {
                let up = d.mesh.up[i];
                let cs = up[0];
                if cs.abs() > 0.99 { continue; }
                let dir = [1.0 - up[0] * cs, -up[1] * cs, -up[2] * cs];
                let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                s += (w[3 * i] * dir[0] + w[3 * i + 1] * dir[1] + w[3 * i + 2] * dir[2]) / l;
                c += 1;
            }
            comp[k] = s / c as f64;
        }
        println!("period {period}: mean flow toward substellar point diag {:.2} stepped {:.2} m/s", comp[0], comp[1]);
        assert!(comp[1] > 1.0, "stepped surface flow must converge on the day side, got {}", comp[1]);
        assert!((comp[1] / comp[0] - 1.0).abs() < 0.2, "diag {} stepped {}", comp[0], comp[1]);
    }
}

#[test]
fn number_of_wind_bands_is_monotone_in_rotation_rate() {
    let d = sphere(4);
    let mut counts = Vec::new();
    let mut diag = Vec::new();
    for period in [86164.0 * 4.0, 86164.0, 86164.0 / 3.0] {
        let (mut a, mut b) = (atmos_fields(&d, earth_t, ocean), atmos_fields(&d, earth_t, ocean));
        run(&d, &mut a, false, &[("pressure", "rotation_period_s", period), ("wind", "blocking_gain", 0.0)]);
        run(&d, &mut b, true, &[("pressure", "rotation_period_s", period), ("wind", "blocking_gain", 0.0)]);
        diag.push(band_count(&d, &a));
        counts.push(band_count(&d, &b));
    }
    println!("band counts (sign changes of zonal-mean wind): diagnostic {diag:?}, stepped {counts:?}");
    assert_eq!(counts, diag, "stepped bands match the diagnostic ones");
    assert!(counts[0] <= counts[1] && counts[1] <= counts[2], "monotone: {counts:?}");
    assert!(counts[2] > counts[0] + 1, "fast rotation has clearly more bands: {counts:?}");
}

#[test]
fn quadratic_drag_slows_the_wind_and_still_converges() {
    let d = sphere(3);
    let (mut a, mut b) = (atmos_fields(&d, earth_t, ocean), atmos_fields(&d, earth_t, ocean));
    run(&d, &mut a, true, &[("wind", "blocking_gain", 0.0)]);
    let s = run(&d, &mut b, true, &[("wind", "blocking_gain", 0.0), ("wind", "drag_quad", 2e-6)]);
    let mean = |f: &FieldStore| f.data("wind_speed").unwrap().iter().sum::<f64>() / d.n_cells() as f64;
    println!("mean speed linear {:.2}, with quadratic drag {:.2}, steps {:?}", mean(&a), mean(&b), s.module_param("wind", "last_steps"));
    assert!(mean(&b) < 0.9 * mean(&a));
}

#[test]
fn blocking_steers_flow_around_a_ridge_like_the_diagnostic_wind() {
    let d = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 60, ny: 20 }, cell_size: 5.0e4, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1,
    })
    .unwrap();
    let n = d.n_cells();
    let x: Vec<f64> = d.mesh.centroid.iter().map(|c| c[0]).collect();
    let y: Vec<f64> = d.mesh.centroid.iter().map(|c| c[1]).collect();
    let z: Vec<f64> = x.iter().map(|&xx| if xx.abs() < 2.5e5 { 3000.0 * (1.0 - xx.abs() / 2.5e5) } else { 10.0 }).collect();
    let go = |stepped: bool, gain: f64| -> Vec<f64> {
        let mut f = FieldStore::new(n);
        f.put("elevation", 1, z.clone()).unwrap();
        f.put("pressure_sl", 1, x.iter().map(|xx| 101325.0 - 1.0e-3 * xx).collect()).unwrap();
        f.put("pressure_upper", 1, vec![5.0e4; n]).unwrap();
        f.put("air_density", 1, vec![1.2; n]).unwrap();
        f.put("coriolis_f", 1, vec![0.0; n]).unwrap();
        let mut s = Scheduler::new();
        if stepped { s.register(Box::new(SteppedWind::new())).unwrap() } else { s.register(Box::new(DiagnosticWind::new())).unwrap() }
        s.set_param("wind", "blocking_gain", gain).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (0..n).map(|i| f.data("wind").unwrap()[3 * i]).collect()
    };
    let mean = |u: &[f64], lo: f64, hi: f64| {
        let ids: Vec<usize> = (0..n).filter(|&i| x[i] > lo && x[i] < hi && y[i].abs() < 1.0e5).collect();
        ids.iter().map(|&i| u[i]).sum::<f64>() / ids.len() as f64
    };
    let (sf, sb, db) = (go(true, 0.0), go(true, 1.0), go(false, 1.0));
    let far = (mean(&sf, -1.0e6, -6.0e5), mean(&sb, -1.0e6, -6.0e5));
    let wind_slope = (mean(&sf, -2.3e5, -0.3e5), mean(&sb, -2.3e5, -0.3e5), mean(&db, -2.3e5, -0.3e5));
    let lee = (mean(&sf, 0.3e5, 2.3e5), mean(&sb, 0.3e5, 2.3e5));
    println!("far upwind free {:.2} blocked {:.2}; windward slope free {:.2} blocked {:.2} (diag blocked {:.2}); lee free {:.2} blocked {:.2}",
        far.0, far.1, wind_slope.0, wind_slope.1, wind_slope.2, lee.0, lee.1);
    assert!((far.0 - 26.7).abs() < 0.5, "drag-limited far-field wind {}", far.0);
    assert!((far.0 - far.1).abs() < 0.05, "no blocking far upwind");
    assert!(wind_slope.1 < 0.9 * wind_slope.0, "windward slope slowed: free {} blocked {}", wind_slope.0, wind_slope.1);
    assert!(wind_slope.1 > 0.0, "steered, not reversed");
    assert!(wind_slope.1 < 2.0 * wind_slope.2 + 1.0 && wind_slope.1 > 0.3 * wind_slope.2, "same order as diagnostic");
    assert!((lee.0 - lee.1).abs() < 0.5, "lee slope not blocked");
}

#[test]
fn guard_warm_start_and_spin_up() {
    let d = sphere(3);
    // max_steps guard: error, not an unconverged field
    let mut f = atmos_fields(&d, earth_t, ocean);
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s).unwrap();
    s.set_param("wind", "max_steps", 5.0).unwrap();
    assert!(matches!(s.run(&d, &mut f, &DirtyRegion::All), Err(Error::InvalidParam(_))));
    // bad parameters
    assert!(s.set_param("wind", "dt_max_s", 0.0).is_err());
    assert!(s.set_param("wind", "last_steps", 3.0).is_err());
    assert!(s.set_param("wind", "nope", 1.0).is_err());
    // spin-up from the diagnostic balance needs far fewer steps than a cold start
    let steps = |params: &[(&str, &str, f64)]| {
        let mut f = atmos_fields(&d, earth_t, ocean);
        let s = run(&d, &mut f, true, params);
        s.module_param("wind", "last_steps").unwrap()
    };
    let cold = steps(&[("wind", "blocking_gain", 0.0)]);
    let spun = steps(&[("wind", "blocking_gain", 0.0), ("wind", "spin_up_steps", 1.0)]);
    println!("cold start {cold} steps, spin-up start {spun} steps");
    assert!(spun < 0.2 * cold);
}

#[test]
fn robust_deterministic_and_bounded_on_all_topologies() {
    let cube = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 12, surface_gravity: 9.81, seed: 1 }).unwrap();
    let torus = generate_plane(&PlaneParams {
        shape: PlaneShape::Rect { nx: 40, ny: 20 }, cell_size: 1.0e5, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 2,
    })
    .unwrap();
    let sph = sphere(4);
    let mk = |d: &Domain, rugged: bool| {
        let mut f = FieldStore::new(d.n_cells());
        let t: Vec<f64> = d.mesh.centroid.iter().map(|c| 290.0 + 8.0 * (c[0] / 1.0e6).sin() + 5.0 * (c[1] / 1.0e6).cos()).collect();
        f.put("temperature_sl", 1, t.clone()).unwrap();
        f.put("temperature", 1, t).unwrap();
        let z: Vec<f64> = d.mesh.centroid.iter().map(|c| if rugged { 2500.0 * (c[0] / 7.0e5).sin() * (c[1] / 9.0e5).cos() } else { -3000.0 }).collect();
        f.put("elevation", 1, z).unwrap();
        f
    };
    let no_p: Vec<(&str, &str, f64)> = vec![];
    let tp: Vec<(&str, &str, f64)> = vec![("pressure", "coriolis_const", 5.0e-5)];
    for (d, params, rugged) in [(&cube, &no_p, true), (&torus, &tp, true), (&sph, &no_p, true), (&sph, &no_p, false)] {
        let (mut a, mut b) = (mk(d, rugged), mk(d, rugged));
        run(d, &mut a, true, params);
        run(d, &mut b, true, params);
        let sp = a.data("wind_speed").unwrap();
        assert!(sp.iter().all(|&s| s.is_finite() && s <= 80.0));
        assert!(a.data("wind").unwrap().iter().all(|v| v.is_finite()));
        assert!(sp.iter().sum::<f64>() > 0.0);
        assert_eq!(a.data("wind").unwrap(), b.data("wind").unwrap(), "bit-identical");
    }
}

#[test]
fn no_atmosphere_gives_zero_wind_and_full_pipeline_runs_with_closed_water_budget() {
    let mut dn = sphere(3);
    dn.media.atmosphere = None;
    let mut f = atmos_fields(&dn, earth_t, ocean);
    run(&dn, &mut f, true, &[]);
    assert!(f.data("wind").unwrap().iter().all(|&v| v == 0.0));

    // stepped wind feeding moisture transport and rain through the scheduler
    let d = sphere(3);
    let mut f = atmos_fields(&d, earth_t, |u| if u[2] > 0.5 { 500.0 } else { -3000.0 });
    f.put("ice_fraction", 1, vec![0.0; d.n_cells()]).unwrap();
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s).unwrap();
    add_moisture_source_stage(&mut s).unwrap();
    add_moisture_precip_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let b = WaterBudget::from_fields(&d, &f, 2592000.0).unwrap();
    println!("water budget residual_rel {:.2e}, mean P {:.2} mm/day", b.residual_rel, mean_precip_mm_day(&d, &f).unwrap());
    assert!(b.residual_rel.abs() < 1e-8);
    assert!(mean_precip_mm_day(&d, &f).unwrap() > 0.5);
}
