//! Part 3b-3 acceptance tests: lift, condensation and precipitation (coupled to transport).
use enbuilder::generators::*;
use enbuilder::math::scale;
use enbuilder::precipitation::*;
use enbuilder::*;

const R: f64 = 6.371e6;

fn sphere() -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: 4, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane_wrap(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 1 }).unwrap()
}

fn fields(d: &Domain, wind: Vec<f64>, z: Vec<f64>, t: f64, evap: Vec<f64>, vapor: Vec<f64>) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    f.put("wind", 3, wind).unwrap();
    f.put("elevation", 1, z).unwrap();
    f.put("temperature", 1, vec![t; d.n_cells()]).unwrap();
    f.put("evaporation", 1, evap).unwrap();
    f.put("vapor", 1, vapor).unwrap();
    f
}

fn run(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) {
    let mut s = Scheduler::new();
    add_moisture_precip_stage(&mut s).unwrap();
    s.set_param("moisture_transport", "warm_start", 1.0).unwrap();
    for (k, v) in params {
        s.set_param("moisture_transport", k, *v).unwrap();
    }
    s.run(d, f, &DirtyRegion::All).unwrap();
}

fn mass(d: &Domain, v: &[f64]) -> f64 { v.iter().zip(&d.mesh.area).map(|(a, b)| a * b).sum() }

/// Returns (windward precip_oro sum, leeward sum, vapour upwind mean, vapour downwind mean) for the plane ridge.
fn plane_ridge(extra: &[(&str, f64)]) -> (f64, f64, f64, f64, FieldStore, Domain) {
    let d = plane_wrap(80, 16);
    let n = d.n_cells();
    let mut w = vec![0.0; 3 * n];
    for i in 0..n { w[3 * i] = 10.0; }
    let xs: Vec<f64> = (0..n).map(|i| d.mesh.centroid[i][0] / 1e4).collect();
    let z: Vec<f64> = xs.iter().map(|x| 4000.0 * (-x * x / (2.0 * 16.0)).exp()).collect();
    let mut f = fields(&d, w, z, 290.0, vec![0.0; n], vec![25.0; n]);
    run(&d, &mut f, &[("transport_time_s", 4.0e4)].iter().chain(extra.iter()).cloned().collect::<Vec<_>>());
    let po = f.data("precip_oro").unwrap().to_vec();
    let v = f.data("vapor").unwrap().to_vec();
    let sum = |pred: &dyn Fn(f64) -> bool, a: &[f64]| (0..n).filter(|&i| pred(xs[i])).map(|i| a[i]).sum::<f64>();
    let cnt = |pred: &dyn Fn(f64) -> bool| (0..n).filter(|&i| pred(xs[i])).count() as f64;
    let (wi, le) = (sum(&|x| x < 0.0, &po), sum(&|x| x > 0.0, &po));
    let up = sum(&|x| x < -20.0, &v) / cnt(&|x| x < -20.0);
    let dn = sum(&|x| x > 8.0 && x < 30.0, &v) / cnt(&|x| x > 8.0 && x < 30.0);
    (wi, le, up, dn, f, d)
}

#[test]
fn rain_shadow_on_plane() {
    let (wi, le, up, dn, _, _) = plane_ridge(&[]);
    assert!(wi > 0.0, "no windward rain");
    assert!(le <= 1e-6 * wi, "leeward {le} vs windward {wi}");
    assert!(dn < 0.97 * up, "lee vapour {dn} not drier than upwind {up}");
}

#[test]
fn rain_shadow_on_sphere() {
    let d = sphere();
    let n = d.n_cells();
    let lon = |i: usize| d.mesh.centroid[i][1].atan2(d.mesh.centroid[i][0]).to_degrees();
    let lat = |i: usize| d.mesh.up[i][2].asin().to_degrees();
    let z: Vec<f64> = (0..n).map(|i| 5000.0 * (-((lon(i) - 90.0).powi(2)) / (2.0 * 8.0f64.powi(2))).exp()).collect();
    let mut w = Vec::new();
    for i in 0..n { w.extend_from_slice(&scale(d.frames[i].east, 10.0)); }
    let mut f = fields(&d, w, z, 290.0, vec![0.0; n], vec![25.0; n]);
    run(&d, &mut f, &[("transport_time_s", 4.0e5)]);
    let po = f.data("precip_oro").unwrap();
    let band = |a: f64, b: f64| (0..n).filter(|&i| lat(i).abs() < 50.0 && lon(i) > a && lon(i) < b).map(|i| po[i]).sum::<f64>();
    let (wi, le) = (band(60.0, 90.0), band(90.0, 130.0));
    assert!(wi > 0.0 && le < 0.01 * wi, "windward {wi} leeward {le}");
    let v = f.data("vapor").unwrap();
    let mean = |a: f64, b: f64| { let c: Vec<f64> = (0..n).filter(|&i| lat(i).abs() < 40.0 && lon(i) > a && lon(i) < b).map(|i| v[i]).collect(); c.iter().sum::<f64>() / c.len() as f64 };
    assert!(mean(100.0, 125.0) < 0.97 * mean(20.0, 60.0));
}

#[test]
fn water_balance_with_all_three_terms() {
    // warm (conv on), divergent wind (frontal on), ridge (oro on), evaporation source
    let d = sphere();
    let n = d.n_cells();
    let mut w = Vec::new();
    for i in 0..n {
        let c = d.mesh.centroid[i];
        let up = d.mesh.up[i];
        let a = [3.0 + c[1] * 1e-6, -2.0 + c[2] * 1e-6, 4.0 - c[0] * 1e-6];
        let t = enbuilder::math::sub(a, scale(up, enbuilder::math::dot(a, up)));
        let l = enbuilder::math::length(t).max(1e-9);
        w.extend_from_slice(&scale(t, 10.0 / l));
    }
    let z: Vec<f64> = (0..n).map(|i| 6000.0 * (d.mesh.centroid[i][0] / R * 4.0).sin().abs()).collect();
    let evap: Vec<f64> = (0..n).map(|i| 2e-5 * (1.0 + (i % 4) as f64)).collect();
    let v0: Vec<f64> = (0..n).map(|i| 55.0 + (i % 7) as f64).collect();
    let mut f = fields(&d, w, z, 303.0, evap.clone(), v0.clone());
    let t = 1.0e5;
    run(&d, &mut f, &[("transport_time_s", t)]);
    let gain: f64 = evap.iter().zip(&d.mesh.area).map(|(e, a)| e * a).sum::<f64>() * t;
    let rain: f64 = f.data("precipitation").unwrap().iter().zip(&d.mesh.area).map(|(p, a)| p * a).sum::<f64>() * t;
    let m1 = mass(&d, f.data("vapor").unwrap());
    let m0 = mass(&d, &v0);
    assert!(((m1 + rain - m0 - gain) / m0).abs() < 1e-8, "balance error {}", (m1 + rain - m0 - gain) / m0);
    for name in ["precip_oro", "precip_conv", "precip_front"] {
        assert!(f.data(name).unwrap().iter().sum::<f64>() > 0.0, "{name} never rained");
    }
    let (p, a, b, c, e) = (f.data("precipitation").unwrap(), f.data("precip_oro").unwrap(), f.data("precip_conv").unwrap(), f.data("precip_front").unwrap(), f.data("precip_sat").unwrap());
    for i in 0..n {
        assert!((p[i] - (a[i] + b[i] + c[i] + e[i])).abs() <= 1e-12 * (1.0 + p[i]));
        assert!(p[i] >= 0.0);
    }
    assert!(f.data("vapor").unwrap().iter().all(|&x| x >= 0.0 && x.is_finite()));
}

#[test]
fn no_vapour_no_rain() {
    let d = plane_wrap(40, 12);
    let n = d.n_cells();
    let mut w = vec![0.0; 3 * n];
    for i in 0..n { w[3 * i] = 10.0; }
    let z: Vec<f64> = (0..n).map(|i| 3000.0 * (-(d.mesh.centroid[i][0] / 1e5).powi(2)).exp()).collect();
    let mut f = fields(&d, w, z, 305.0, vec![0.0; n], vec![0.0; n]);
    run(&d, &mut f, &[("transport_time_s", 3.0e4)]);
    for name in ["precipitation", "precip_oro", "precip_conv", "precip_front", "precip_sat"] {
        assert!(f.data(name).unwrap().iter().all(|&x| x == 0.0), "{name}");
    }
    let par = PrecipParams::from_params(&ParamSet::new());
    let st = PrecipState { lift: vec![1.0], rise: vec![500.0], tau: vec![1000.0], lapse: vec![0.0098], g: vec![9.81], temp: vec![305.0], conv: vec![1e-5], div: vec![-1e-5] };
    assert_eq!(terms(&par, &st, 0, 0.0, 100.0), (0.0, 0.0, 0.0, 0.0));
}

#[test]
fn terms_are_independent() {
    let st = PrecipState { lift: vec![1.0], rise: vec![600.0], tau: vec![1000.0], lapse: vec![0.0098], g: vec![9.81], temp: vec![300.0], conv: vec![1e-5], div: vec![-1e-5] };
    let base = PrecipParams::from_params(&ParamSet::new());
    let (o, c, fr, sa) = terms(&base, &st, 0, 40.0, 600.0);
    assert!(o > 0.0 && c > 0.0 && fr > 0.0 && sa == 0.0, "{o} {c} {fr}");
    assert!(o + c + fr < 40.0);
    for which in 0..3 {
        let mut p = base.clone();
        match which { 0 => p.oro_on = false, 1 => p.conv_on = false, _ => p.front_on = false }
        let (o2, c2, f2, _) = terms(&p, &st, 0, 40.0, 600.0);
        let (eo, ec, ef) = (if which == 0 { 0.0 } else { o }, if which == 1 { 0.0 } else { c }, if which == 2 { 0.0 } else { fr });
        assert_eq!((o2, c2, f2), (eo, ec, ef), "switch {which}");
    }
    let mut p = base.clone();
    p.removal_fraction = 0.5;
    let (o3, c3, f3, _) = terms(&p, &st, 0, 40.0, 600.0);
    assert!((o3 - 0.5 * o).abs() < 1e-12 && (c3 - 0.5 * c).abs() < 1e-12 && (f3 - 0.5 * fr).abs() < 1e-12);
}

#[test]
fn gravity_link() {
    // NOTE: the lapse rate g/cp grows with gravity (the PART file says "reduces"; physics says increases).
    let cp = 1005.0;
    assert!((dry_adiabatic_lapse(19.62, cp) / dry_adiabatic_lapse(9.81, cp) - 2.0).abs() < 1e-12);
    // a high-gravity column holds less vapour, so the same vapour is closer to saturation
    assert!(column_capacity(290.0, 19.62, 0.25) < column_capacity(290.0, 9.81, 0.25));
    // and the module actually reads local |g|: doubling gravity changes orographic rain
    let mk = |g: f64| {
        let d = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 80, ny: 16 }, cell_size: 10_000.0, boundary: BoundaryKind::Wrap, surface_gravity: g, seed: 1 }).unwrap();
        let n = d.n_cells();
        let mut w = vec![0.0; 3 * n];
        for i in 0..n { w[3 * i] = 10.0; }
        let z: Vec<f64> = (0..n).map(|i| 4000.0 * (-(d.mesh.centroid[i][0] / 1e4).powi(2) / 32.0).exp()).collect();
        let mut f = fields(&d, w, z, 290.0, vec![0.0; n], vec![25.0; n]);
        run(&d, &mut f, &[("transport_time_s", 4.0e4)]);
        f.data("precip_oro").unwrap().iter().sum::<f64>()
    };
    assert!((mk(9.81) - mk(19.62)).abs() > 1e-9);
}

#[test]
fn full_wiring_through_scheduler() {
    // moisture_source -> moisture_transport(precip) with the other fields supplied externally
    let d = sphere();
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("temperature", 1, d.mesh.up.iter().map(|u| 300.0 - 40.0 * u[2] * u[2]).collect()).unwrap();
    f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
    f.put("elevation", 1, (0..n).map(|i| if d.mesh.up[i][0] > 0.9 { 2500.0 } else { -3000.0 }).collect()).unwrap();
    f.put("wind_speed", 1, vec![8.0; n]).unwrap();
    let mut w = Vec::new();
    for i in 0..n { w.extend_from_slice(&scale(d.frames[i].east, 8.0)); }
    f.put("wind", 3, w).unwrap();
    let mut s = Scheduler::new();
    add_moisture_source_stage(&mut s).unwrap();
    add_moisture_precip_stage(&mut s).unwrap();
    assert_eq!(s.plan_order().unwrap(), vec!["moisture_source".to_string(), "moisture_transport".to_string()]);
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    for name in ["vapor", "precipitation", "precip_oro", "precip_conv", "precip_front", "lift", "evaporation"] {
        assert!(f.data(name).unwrap().iter().all(|x| x.is_finite() && *x >= 0.0), "{name}");
    }
    assert!(f.data("precipitation").unwrap().iter().sum::<f64>() > 0.0);
}

#[test]
fn params_and_plain_transport_unchanged() {
    let mut m = MoistureTransport::with_precipitation();
    assert!(m.set_param("rh_trigger", 1.0).is_err());
    assert!(m.set_param("removal_fraction", 1.5).is_err());
    assert!(m.set_param("conv_on", 0.5).is_err());
    assert!(m.set_param("lift_gain", 2.0).is_ok());
    // the plain 3b-2 module rejects precipitation parameters and needs no temperature/elevation
    let mut plain = MoistureTransport::new();
    assert!(plain.set_param("lift_gain", 1.0).is_err());
    assert!(!plain.spec().inputs.contains(&"temperature".to_string()));
    assert!(m.spec().outputs.contains(&"precip_oro".to_string()));
}
