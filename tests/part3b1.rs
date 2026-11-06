//! Part 3b-1 acceptance tests: evaporation sources.
use enbuilder::generators::*;
use enbuilder::*;

fn sphere() -> Domain {
    generate_sphere(&SphereParams { radius: 6.371e6, subdivision: 4, surface_gravity: 9.81, seed: 7 }).unwrap()
}

/// All-ocean Earth-like fields: T = 300 - 40 sin^2(lat), wind 7 m/s, no ice.
fn fields(d: &Domain) -> FieldStore {
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("temperature", 1, d.mesh.up.iter().map(|u| 300.0 - 40.0 * u[2] * u[2]).collect()).unwrap();
    f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
    f.put("elevation", 1, vec![-3000.0; n]).unwrap();
    f.put("wind_speed", 1, vec![7.0; n]).unwrap();
    f
}

fn run(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) {
    let mut s = Scheduler::new();
    add_moisture_source_stage(&mut s).unwrap();
    for (k, v) in params {
        s.set_param("moisture_source", k, *v).unwrap();
    }
    s.run(d, f, &DirtyRegion::All).unwrap();
}

fn total(d: &Domain, f: &FieldStore) -> f64 {
    f.data("evaporation").unwrap().iter().zip(&d.mesh.area).map(|(e, a)| e * a).sum()
}

#[test]
fn equator_exceeds_poles_and_nonnegative() {
    let d = sphere();
    let mut f = fields(&d);
    run(&d, &mut f, &[]);
    let e = f.data("evaporation").unwrap();
    assert!(e.iter().all(|&x| x >= 0.0 && x.is_finite()));
    let band = |lo: f64, hi: f64| {
        let v: Vec<f64> = (0..d.n_cells()).filter(|&i| (lo..hi).contains(&d.mesh.up[i][2].abs())).map(|i| e[i]).collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    let (eq, pole) = (band(0.0, 0.2), band(0.85, 1.01));
    assert!(eq > 3.0 * pole, "eq {eq} pole {pole}");
    // realistic tropical rate: 2-8 mm/day (1 kg/m^2 = 1 mm)
    let mm_day = eq * 86400.0;
    assert!((2.0..8.0).contains(&mm_day), "{mm_day} mm/day");
}

#[test]
fn ice_cells_zero_and_partial_ice_scales() {
    let d = sphere();
    let mut f = fields(&d);
    let n = d.n_cells();
    let ice: Vec<f64> = (0..n).map(|i| match i % 3 { 0 => 1.0, 1 => 0.5, _ => 0.0 }).collect();
    f.put("ice_fraction", 1, ice).unwrap();
    run(&d, &mut f, &[]);
    let e = f.data("evaporation").unwrap().to_vec();
    let mut g = fields(&d);
    run(&d, &mut g, &[]);
    let e0 = g.data("evaporation").unwrap();
    for i in 0..n {
        match i % 3 {
            0 => assert_eq!(e[i], 0.0),
            1 => assert!((e[i] - 0.5 * e0[i]).abs() < 1e-15),
            _ => assert_eq!(e[i], e0[i]),
        }
    }
}

#[test]
fn no_atmosphere_is_zero() {
    let mut d = sphere();
    d.media.atmosphere = None;
    let mut f = fields(&d);
    run(&d, &mut f, &[]);
    assert!(f.data("evaporation").unwrap().iter().all(|&x| x == 0.0));
}

#[test]
fn k_ocean_scales_total_exactly() {
    let d = sphere();
    let mut a = fields(&d);
    let mut b = fields(&d);
    run(&d, &mut a, &[("k_ocean", 1.0e-8), ("max_evap", 1.0)]);
    run(&d, &mut b, &[("k_ocean", 2.0e-8), ("max_evap", 1.0)]);
    let (ta, tb) = (total(&d, &a), total(&d, &b));
    assert!(ta > 0.0);
    assert!((tb / ta - 2.0).abs() < 1e-12, "{}", tb / ta);
}

#[test]
fn land_is_scaled_by_availability_and_cap_applies() {
    let d = sphere();
    let n = d.n_cells();
    let mut f = fields(&d);
    f.put("elevation", 1, vec![100.0; n]).unwrap();
    let mut g = fields(&d);
    run(&d, &mut f, &[("land_availability", 0.3)]);
    run(&d, &mut g, &[]);
    let r = total(&d, &f) / total(&d, &g);
    assert!((r - 0.3).abs() < 1e-12, "{r}");
    let mut h = fields(&d);
    run(&d, &mut h, &[("max_evap", 1e-6)]);
    assert!(h.data("evaporation").unwrap().iter().all(|&x| x <= 1e-6));
}

#[test]
fn bad_params_rejected_and_cold_ocean_safe() {
    let mut m = MoistureSource::new();
    assert!(m.set_param("relative_humidity_ref", 1.5).is_err());
    assert!(m.set_param("nope", 1.0).is_err());
    assert!(m.set_param("k_ocean", -1.0).is_err());
    let d = sphere();
    let mut f = fields(&d);
    f.put("temperature", 1, vec![0.0; d.n_cells()]).unwrap(); // absurd 0 K input must stay finite
    run(&d, &mut f, &[]);
    assert!(f.data("evaporation").unwrap().iter().all(|x| x.is_finite() && *x >= 0.0));
}

#[test]
fn deterministic() {
    let d = sphere();
    let (mut a, mut b) = (fields(&d), fields(&d));
    run(&d, &mut a, &[]);
    run(&d, &mut b, &[]);
    assert_eq!(a.data("evaporation").unwrap(), b.data("evaporation").unwrap());
}
