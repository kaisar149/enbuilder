//! Part 4a-4: upwelling, thermohaline sinking, sea ice, nutrients.
//! Worlds: (1) walled plane with land strips (analytic coastal upwelling, both signs of f), (2) wrapped plane (analytic Ekman pumping),
//! (3) Earth-like sphere (hemispheres, salinity, deep water), (4) the real pipeline (wiring, warming, switches).
//! Thresholds are analytic or structural; the two coupled-climate checks are loose (see comments).
use enbuilder::generators::*;
use enbuilder::math::{dot, normalize, scale, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_heat::*;
use enbuilder::ocean_secondary::*;
use enbuilder::testworld::{DAY_S, YEAR_S};
use enbuilder::*;

const R: f64 = 6.371e6;
const OMEGA: f64 = 7.292e-5;

fn lat(d: &Domain, i: usize) -> f64 { d.mesh.up[i][2].asin() }
fn area_sum(d: &Domain, a: &[f64]) -> f64 { (0..d.n_cells()).map(|i| d.mesh.area[i] * a[i]).sum() }
fn flat(v: &[Vec3]) -> Vec<f64> { v.iter().flat_map(|x| x.iter().copied()).collect() }

// ------------------------------------------------------------------------------------------------ worlds
/// Walled plane nx x ny, land (z = +100) where `land(i, j)`, ocean (z = -1000) elsewhere. Fields: ocean_mask, coast_distance, coriolis_f (constant `f`),
/// wind_stress = `tau0` along the local north everywhere.
fn plane_world(nx: usize, ny: usize, boundary: BoundaryKind, land: &dyn Fn(usize, usize) -> bool, f: f64, tau0: f64) -> (Domain, FieldStore, Vec<bool>) {
    let d = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary, surface_gravity: 9.81, seed: 1 }).unwrap();
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|k| if land(k % nx, k / nx) { 100.0 } else { -1000.0 }).collect();
    let (mask, comp) = ocean_cells(&d, &z, &vec![0.0; n], 1.0);
    let mut fs = FieldStore::new(n);
    fs.put("elevation", 1, z).unwrap();
    fs.put("ocean_mask", 1, mask.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    fs.put("coast_distance", 1, coast_distance(&d, &mask, &comp)).unwrap();
    fs.put("coriolis_f", 1, vec![f; n]).unwrap();
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].north, tau0)).collect();
    fs.put("wind_stress", 3, flat(&tau)).unwrap();
    (d, fs, mask)
}

fn run_up(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) {
    let mut m = Upwelling::new();
    for &(k, v) in params {
        m.set_param(k, v).unwrap();
    }
    m.run(d, f, &DirtyRegion::All).unwrap();
}

/// Earth-like caps sphere with synthetic climate fields: zonal temperature, E - P pattern, zonal stress, f = 2 Omega sin(lat).
fn sphere_world(level: u32, ep_amp: f64) -> (Domain, FieldStore, Vec<bool>) {
    let mut d = generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR_S, DAY_S, 23.4f64.to_radians()));
    let n = d.n_cells();
    let caps: [([f64; 3], f64); 3] = [([1.0, 0.2, 0.3], 42.0), ([-0.8, 0.5, -0.1], 36.0), ([0.1, -0.9, -0.4], 30.0)];
    let land: Vec<bool> = (0..n).map(|i| caps.iter().any(|(ax, th)| dot(d.mesh.up[i], normalize(*ax)) > th.to_radians().cos())).collect();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -3000.0 }).collect();
    let (mask, comp) = ocean_cells(&d, &z, &vec![0.0; n], 1.0);
    let cd = coast_distance(&d, &mask, &comp);
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, z).unwrap();
    f.put("ocean_mask", 1, mask.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("coast_distance", 1, cd).unwrap();
    f.put("coriolis_f", 1, (0..n).map(|i| 2.0 * OMEGA * d.mesh.up[i][2]).collect()).unwrap();
    f.put("temperature_sl", 1, (0..n).map(|i| 300.0 - 40.0 * lat(&d, i).sin().powi(2)).collect()).unwrap();
    // E - P: evaporation-dominated subtropics (+), rainy equator and high latitudes (-). kg/m^2/s.
    // E - P pattern: +/- by lat (kg/m^2/s); only the sign pattern and the amplitude matter here.
    let (e, p): (Vec<f64>, Vec<f64>) = (0..n)
        .map(|i| {
            let v = ep_amp * (3.0 * lat(&d, i)).cos(); // + at the equator and +-60..., sign pattern is all that matters here
            if v > 0.0 { (v, 0.0) } else { (0.0, -v) }
        })
        .unzip();
    f.put("evaporation", 1, e).unwrap();
    f.put("precipitation", 1, p).unwrap();
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat(&d, i)).cos())).collect();
    f.put("wind_stress", 3, flat(&tau)).unwrap();
    (d, f, mask)
}

fn copy_fields(f: &FieldStore) -> FieldStore {
    let mut g = FieldStore::new(f.n_cells());
    for name in f.names() {
        g.put(&name, f.layers(&name).unwrap(), f.data(&name).unwrap().to_vec()).unwrap();
    }
    g
}

fn run_th(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) {
    let mut m = Thermohaline::new();
    for &(k, v) in params {
        m.set_param(k, v).unwrap();
    }
    m.run(d, f, &DirtyRegion::All).unwrap();
}

// ------------------------------------------------------------------------------------------------ upwelling
#[test]
fn coastal_upwelling_has_the_right_sign_on_each_coast_of_a_flat_plane_and_flips_with_f() {
    // Land strips on the west (x < 6) and east (x >= 18); stress points north everywhere. With f > 0 the Ekman transport is to the right
    // (east): it leaves the west coast (upwelling) and presses on the east coast (downwelling). f < 0 reverses both.
    let (nx, ny) = (24usize, 16usize);
    let land = |i: usize, _j: usize| i < 6 || i >= 18;
    for (f0, sgn) in [(1.0e-4, 1.0), (-1.0e-4, -1.0)] {
        let (d, mut fs, _) = plane_world(nx, ny, BoundaryKind::Wall, &land, f0, 0.1);
        assert!(d.frames[0].east[0] > 0.9, "plane east is +x");
        run_up(&d, &mut fs, &[("ekman_on", 0.0)]);
        let w = fs.data("upwelling").unwrap().to_vec();
        let expect = 0.1 / (1025.0 * 1.0e-4) * 10_000.0 / 1.0e8; // |tau/(rho f)| * L / A
        for j in 0..ny {
            let (west, east, inner) = (j * nx + 6, j * nx + 17, j * nx + 11);
            assert!((w[west] - sgn * expect).abs() < 1e-3 * expect, "west coast f={f0}: {} vs {}", w[west], sgn * expect);
            assert!((w[east] + sgn * expect).abs() < 1e-3 * expect, "east coast f={f0}: {} vs {}", w[east], -sgn * expect);
            assert_eq!(w[inner], 0.0, "open water has no coastal term");
        }
        // Land cells have nothing.
        assert!(w.iter().enumerate().all(|(k, &v)| !(k % nx < 6 || k % nx >= 18) || v == 0.0));
    }
}

#[test]
fn coastal_upwelling_on_a_sphere_follows_the_sign_of_coriolis_f_not_latitude() {
    let (d, mut f, mask) = sphere_world(4, 0.0);
    let n = d.n_cells();
    // Stress along local north: Ekman transport is along east * sign(f), offshore when the land is to the west for f > 0.
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].north, 0.1)).collect();
    f.put("wind_stress", 3, flat(&tau)).unwrap();
    let mut g = copy_fields(&f);
    run_up(&d, &mut f, &[("ekman_on", 0.0), ("max_upwelling", 1.0)]);
    let cv = enbuilder::ocean_currents::coast_vectors(&d, &mask);
    let (w, fc) = (f.data("upwelling").unwrap().to_vec(), f.data("coriolis_f").unwrap().to_vec());
    let (mut pos_nh, mut neg_nh, mut pos_sh, mut neg_sh) = (0, 0, 0, 0);
    for i in 0..n {
        if !mask[i] || cv[i].1 == 0.0 || fc[i].abs() < 3.0e-5 {
            continue;
        }
        let se = dot(cv[i].0, d.frames[i].east);
        if se.abs() < 0.3 * cv[i].1 {
            continue; // coast mostly north/south of the cell: no clean along-stress geometry
        }
        let up_expected = (fc[i] > 0.0) == (se < 0.0); // f > 0: land to the west gives upwelling; f < 0: land to the east
        assert_eq!(w[i] > 0.0, up_expected, "cell {i} lat {:.1} f {:.2e} S.east {:.2e} w {:.2e}", lat(&d, i).to_degrees(), fc[i], se, w[i]);
        match (fc[i] > 0.0, w[i] > 0.0) {
            (true, true) => pos_nh += 1,
            (true, false) => neg_nh += 1,
            (false, true) => pos_sh += 1,
            (false, false) => neg_sh += 1,
        }
    }
    println!("NH up/down {pos_nh}/{neg_nh}, SH up/down {pos_sh}/{neg_sh}");
    assert!(pos_nh > 0 && neg_nh > 0 && pos_sh > 0 && neg_sh > 0, "both signs on both hemispheres");
    // Same latitude, opposite f: the field flips exactly.
    let w0 = f.data("upwelling").unwrap().to_vec();
    let nf: Vec<f64> = fc.iter().map(|v| -v).collect();
    g.put("coriolis_f", 1, nf).unwrap();
    run_up(&d, &mut g, &[("ekman_on", 0.0), ("max_upwelling", 1.0)]);
    let w1 = g.data("upwelling").unwrap();
    for i in 0..n {
        assert!((w0[i] + w1[i]).abs() <= 1e-12 * (1.0 + w0[i].abs()), "cell {i}");
    }
}

#[test]
fn ekman_pumping_matches_the_analytic_curl_on_a_wrapped_plane() {
    let (nx, ny) = (64usize, 8usize);
    let (d, mut f, _) = plane_world(nx, ny, BoundaryKind::Wrap, &|_, _| false, 1.0e-4, 0.0);
    let n = d.n_cells();
    let lx = nx as f64 * 10_000.0;
    let k = 2.0 * std::f64::consts::PI / lx;
    let tau0 = 0.1;
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].north, tau0 * (k * d.mesh.centroid[i][0]).sin())).collect();
    f.put("wind_stress", 3, flat(&tau)).unwrap();
    run_up(&d, &mut f, &[("coastal_on", 0.0)]);
    let ek = f.data("ekman_pumping").unwrap();
    // curl_z = d(tau_y)/dx = tau0 k cos(kx); w = curl / (rho f).
    let amp = tau0 * k / (1025.0 * 1.0e-4);
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..n {
        let want = amp * (k * d.mesh.centroid[i][0]).cos();
        num += ek[i] * want;
        den += want * want;
    }
    let gain = num / den;
    println!("Ekman pumping gain vs analytic: {gain:.4}");
    assert!((gain - 1.0).abs() < 0.05, "gain {gain}");
    assert!(ek.iter().all(|v| v.is_finite()) && f.data("coastal_upwelling").unwrap().iter().all(|&v| v == 0.0));
}

#[test]
fn near_the_equator_the_ekman_terms_are_off() {
    let (d, mut f, mask) = sphere_world(3, 0.0);
    run_up(&d, &mut f, &[]);
    let (w, fc) = (f.data("upwelling").unwrap(), f.data("coriolis_f").unwrap());
    for i in 0..d.n_cells() {
        if !mask[i] || fc[i].abs() < 2.0e-5 {
            assert_eq!(w[i], 0.0, "cell {i}");
        }
        assert!(w[i].abs() <= 1.0e-3 + 1e-15);
    }
}

// ------------------------------------------------------------------------------------------------ thermohaline
#[test]
fn salinity_is_bounded_and_mean_conserved_and_the_clamp_holds_for_extreme_forcing() {
    for amp in [3.0e-5, 5.0e-3] {
        let (d, mut f, mask) = sphere_world(4, amp);
        run_th(&d, &mut f, &[]);
        let s = f.data("salinity").unwrap();
        let n = d.n_cells();
        let (mut a, mut m) = (0.0, 0.0);
        for i in 0..n {
            if mask[i] {
                a += d.mesh.area[i];
                m += d.mesh.area[i] * s[i];
            } else {
                assert_eq!(s[i], 0.0);
            }
        }
        let mean = m / a;
        println!("amp {amp}: mean salinity {mean:.12}");
        assert!((mean - 35.0).abs() < 1e-9, "mean {mean}");
        let (lo, hi) = (35.0 - 2.0 * 5.0 - 1e-9, 35.0 + 2.0 * 5.0 + 1e-9);
        assert!(s.iter().zip(&mask).all(|(&v, &o)| !o || (v >= lo && v <= hi)));
        assert!(s.iter().zip(&mask).any(|(&v, &o)| o && (v - 35.0).abs() > 0.05), "a real pattern, not flat 35");
    }
}

#[test]
fn deep_water_forms_at_high_latitudes_not_at_the_equator_and_the_flux_integrates_to_zero() {
    let (d, mut f, mask) = sphere_world(4, 3.0e-5);
    run_th(&d, &mut f, &[]);
    let (flag, flux, rho) = (f.data("deep_water_formation").unwrap(), f.data("thermohaline_flux").unwrap(), f.data("ocean_density").unwrap());
    let n = d.n_cells();
    let (mut nf, mut min_lat) = (0usize, 90.0f64);
    for i in 0..n {
        let l = lat(&d, i).to_degrees().abs();
        if flag[i] > 0.5 {
            nf += 1;
            min_lat = min_lat.min(l);
            assert!(mask[i] && rho[i] >= 1029.0);
            assert!(flux[i] > 0.0);
        } else if mask[i] {
            assert!(flux[i] < 0.0 || nf == 0);
        }
        if l < 30.0 {
            assert!(flag[i] == 0.0, "no deep water at lat {l:.1}");
        }
    }
    println!("{nf} deep-water cells, lowest latitude {min_lat:.1}");
    assert!(nf > 0 && min_lat > 45.0);
    let (tot, abs) = (area_sum(&d, flux), (0..n).map(|i| d.mesh.area[i] * flux[i].abs()).sum::<f64>());
    assert!(tot.abs() <= 1e-12 * abs, "{tot} {abs}");
}

// ------------------------------------------------------------------------------------------------ sea ice
fn ice_world(sst: Vec<f64>, sal: Vec<f64>) -> (Domain, FieldStore) {
    let (d, mut f, _) = plane_world(8, 8, BoundaryKind::Wall, &|_, _| false, 1.0e-4, 0.0);
    f.put("sst", 1, sst).unwrap();
    f.put("salinity", 1, sal).unwrap();
    (d, f)
}

#[test]
fn sea_ice_forms_only_below_the_salinity_adjusted_freezing_point() {
    let n = 64;
    let sst: Vec<f64> = (0..n).map(|k| 268.0 + 0.1 * k as f64).collect(); // 268.0 .. 274.3 K
    let (d, mut f) = ice_world(sst.clone(), vec![35.0; n]);
    SeaIce::new().run(&d, &mut f, &DirtyRegion::All).unwrap();
    let (h, fr) = (f.data("sea_ice").unwrap(), f.data("sea_ice_fraction").unwrap());
    for k in 0..n {
        let tf = 271.35;
        assert_eq!(h[k] > 0.0, sst[k] < tf, "k {k} sst {}", sst[k]);
        assert!((0.0..=1.0).contains(&fr[k]) && (fr[k] > 0.0) == (h[k] > 0.0));
        if k > 0 {
            assert!(h[k] <= h[k - 1] + 1e-15, "thickness decreases with warming");
        }
    }
    // Fresher water freezes at a higher temperature: 271.5 K ices at S = 30 (T_f = 271.62) but not at S = 35 (271.35).
    let (d2, mut f2) = ice_world(vec![271.5; n], vec![30.0; n]);
    SeaIce::new().run(&d2, &mut f2, &DirtyRegion::All).unwrap();
    assert!(f2.data("sea_ice").unwrap().iter().all(|&v| v > 0.0));
    let (d3, mut f3) = ice_world(vec![271.5; n], vec![35.0; n]);
    SeaIce::new().run(&d3, &mut f3, &DirtyRegion::All).unwrap();
    assert!(f3.data("sea_ice").unwrap().iter().all(|&v| v == 0.0));
    // Thickness cap.
    let (d4, mut f4) = ice_world(vec![200.0; n], vec![35.0; n]);
    SeaIce::new().run(&d4, &mut f4, &DirtyRegion::All).unwrap();
    assert!(f4.data("sea_ice").unwrap().iter().all(|&v| (v - 5.0).abs() < 1e-12));
}

#[test]
fn the_sea_ice_albedo_variant_brightens_iced_ocean_and_leaves_land_alone() {
    let (d, mut f, mask) = plane_world(8, 8, BoundaryKind::Wall, &|i, _| i < 2, 1.0e-4, 0.0);
    let n = d.n_cells();
    f.put("temperature", 1, vec![290.0; n]).unwrap(); // warm: the temperature ramp gives no ice anywhere
    f.put("sea_ice_fraction", 1, (0..n).map(|k| if k % 8 >= 6 { 1.0 } else { 0.0 }).collect()).unwrap();
    let mut a = AlbedoModel::with_sea_ice();
    a.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let (alb, ice) = (f.data("albedo").unwrap().to_vec(), f.data("ice_fraction").unwrap().to_vec());
    let mut g = copy_fields(&f);
    AlbedoModel::new().run(&d, &mut g, &DirtyRegion::All).unwrap();
    let (alb0, ice0) = (g.data("albedo").unwrap(), g.data("ice_fraction").unwrap());
    for k in 0..n {
        if k % 8 >= 6 {
            assert!(mask[k] && ice[k] == 1.0 && ice0[k] == 0.0);
            assert!(alb[k] > alb0[k] + 0.2, "ice raises albedo: {} vs {}", alb[k], alb0[k]);
        } else {
            assert_eq!((alb[k], ice[k]), (alb0[k], ice0[k]), "cell {k} unchanged");
        }
    }
    assert!(AlbedoModel::with_sea_ice().spec().inputs.contains(&"sea_ice_fraction".to_string()));
    assert!(!AlbedoModel::new().spec().inputs.contains(&"sea_ice_fraction".to_string()));
}

// ------------------------------------------------------------------------------------------------ nutrients
#[test]
fn nutrients_follow_upwelling_and_the_coast_and_are_bounded() {
    let (d, mut f, mask) = plane_world(8, 8, BoundaryKind::Wall, &|i, _| i == 0, 1.0e-4, 0.0);
    let n = d.n_cells();
    // w = +1e-5 everywhere on a coast-free (huge distance) ocean: 1 - 1/e.
    f.put("coast_distance", 1, vec![1.0e12; n]).unwrap();
    f.put("upwelling", 1, vec![1.0e-5; n]).unwrap();
    Nutrients::new().run(&d, &mut f, &DirtyRegion::All).unwrap();
    let nu = f.data("nutrients").unwrap();
    for k in 0..n {
        let want = if mask[k] { 1.0 - (-1.0f64).exp() } else { 0.0 };
        assert!((nu[k] - want).abs() < 1e-12, "k {k}");
    }
    // Downwelling gives nothing; river input adds; the index stays below 1.
    f.put("upwelling", 1, vec![-1.0e-4; n]).unwrap();
    Nutrients::new().run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("nutrients").unwrap().iter().all(|&v| v == 0.0));
    let mut m = Nutrients::new();
    m.set_param("river_input", 0.5).unwrap();
    m.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let nu = f.data("nutrients").unwrap();
    assert!(nu.iter().zip(&mask).all(|(&v, &o)| if o { (v - (1.0 - (-0.5f64).exp())).abs() < 1e-12 } else { v == 0.0 }));
    // Coast proximity: nearer is richer, monotonic. `coast_distance` also counts the domain wall as coast, so use a tall plane and a mid row
    // (20 cells from the top and bottom walls); 48 columns keep the east wall (also coast) more than 33 cells away, so only the west strip matters for i < 14.
    let (nx2, ny2) = (48usize, 40usize);
    let (d2, mut f2, mask2) = plane_world(nx2, ny2, BoundaryKind::Wall, &|i, _| i == 0, 1.0e-4, 0.0);
    f2.put("upwelling", 1, vec![0.0; d2.n_cells()]).unwrap();
    Nutrients::new().run(&d2, &mut f2, &DirtyRegion::All).unwrap();
    let nu2 = f2.data("nutrients").unwrap();
    let j = ny2 / 2;
    for i in 1..14 {
        let (a, b) = (nu2[j * nx2 + i], nu2[j * nx2 + i + 1]);
        assert!(mask2[j * nx2 + i] && a > b && (0.0..1.0).contains(&a), "i {i}: {a} {b}");
    }
}

// ------------------------------------------------------------------------------------------------ switches
fn outputs_of(d: &Domain, f0: &FieldStore, up_on: f64, th_on: f64, si_on: f64) -> Vec<(String, Vec<f64>)> {
    let mut f = copy_fields(f0);
    let mut u = Upwelling::new();
    u.set_param("upwelling_on", up_on).unwrap();
    u.run(d, &mut f, &DirtyRegion::All).unwrap();
    let mut t = Thermohaline::new();
    t.set_param("thermohaline_on", th_on).unwrap();
    t.run(d, &mut f, &DirtyRegion::All).unwrap();
    f.put("sst", 1, (0..d.n_cells()).map(|i| 273.15 + 30.0 * lat(d, i).cos() - 28.0).collect()).unwrap();
    let mut s = SeaIce::new();
    s.set_param("sea_ice_on", si_on).unwrap();
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    Nutrients::new().run(d, &mut f, &DirtyRegion::All).unwrap();
    ["upwelling", "ekman_pumping", "coastal_upwelling", "salinity", "ocean_density", "deep_water_formation", "thermohaline_flux", "sea_ice", "sea_ice_fraction", "nutrients"]
        .iter()
        .map(|k| (k.to_string(), f.data(k).unwrap().to_vec()))
        .collect()
}

#[test]
fn each_switch_off_zeroes_only_its_own_outputs() {
    let (d, f0, _) = sphere_world(3, 3.0e-5);
    let base = outputs_of(&d, &f0, 1.0, 1.0, 1.0);
    let get = |v: &Vec<(String, Vec<f64>)>, k: &str| v.iter().find(|(n, _)| n == k).unwrap().1.clone();
    let nonzero = |v: &[f64]| v.iter().any(|&x| x != 0.0);
    for k in ["upwelling", "salinity", "deep_water_formation", "thermohaline_flux", "sea_ice", "nutrients"] {
        assert!(nonzero(&get(&base, k)), "baseline {k} should be nonzero");
    }
    let cases: [(&str, (f64, f64, f64), &[&str], &[&str]); 3] = [
        ("upwelling_on", (0.0, 1.0, 1.0), &["upwelling", "ekman_pumping", "coastal_upwelling"], &["salinity", "ocean_density", "deep_water_formation", "thermohaline_flux", "sea_ice", "sea_ice_fraction"]),
        ("thermohaline_on", (1.0, 0.0, 1.0), &["deep_water_formation", "thermohaline_flux"], &["upwelling", "ekman_pumping", "coastal_upwelling", "salinity", "ocean_density", "sea_ice", "sea_ice_fraction", "nutrients"]),
        ("sea_ice_on", (1.0, 1.0, 0.0), &["sea_ice", "sea_ice_fraction"], &["upwelling", "ekman_pumping", "coastal_upwelling", "salinity", "ocean_density", "deep_water_formation", "thermohaline_flux", "nutrients"]),
    ];
    for (name, (u, t, s), zeroed, same) in cases {
        let o = outputs_of(&d, &f0, u, t, s);
        for k in zeroed {
            assert!(get(&o, k).iter().all(|&x| x == 0.0), "{name}: {k} must be zero");
        }
        for k in same {
            assert_eq!(get(&o, k), get(&base, k), "{name}: {k} must be unchanged");
        }
    }
    // Nutrients depend on upwelling by definition: they change when it is switched off, and only through it.
    let o = outputs_of(&d, &f0, 0.0, 1.0, 1.0);
    assert_ne!(get(&o, "nutrients"), get(&base, "nutrients"));
}

// ------------------------------------------------------------------------------------------------ interface
#[test]
fn parameters_specs_and_errors() {
    let mut u = Upwelling::new();
    assert_eq!(u.params().get("upwelling_on"), Some(1.0));
    assert!(u.set_param("upwelling_on", 0.5).is_err() && u.set_param("rho_water", 0.0).is_err() && u.set_param("nope", 1.0).is_err() && u.set_param("f_floor", f64::NAN).is_err());
    assert_eq!(u.spec().outputs, vec!["upwelling".to_string(), "ekman_pumping".to_string(), "coastal_upwelling".to_string()]);
    let mut t = Thermohaline::new();
    assert_eq!((t.params().get("thermohaline_on"), t.params().get("s_ref")), (Some(1.0), Some(35.0)));
    assert!(t.set_param("thermohaline_on", 2.0).is_err() && t.set_param("mixed_depth_m", 0.0).is_err() && t.set_param("sal_dev_max", -1.0).is_err());
    assert!(t.spec().outputs.contains(&"deep_water_formation".to_string()));
    let mut s = SeaIce::new();
    assert!(s.set_param("sea_ice_on", -1.0).is_err() && s.set_param("h_full", 0.0).is_err());
    assert_eq!(s.spec().inputs, vec!["sst".to_string(), "salinity".to_string(), "ocean_mask".to_string()]);
    let mut nu = Nutrients::new();
    assert_eq!(nu.params().get("river_input"), Some(0.0));
    assert!(nu.set_param("coast_length_m", 0.0).is_err() && nu.set_param("river_input", -1.0).is_err());
    assert!(OceanHeat::with_thermohaline().spec().inputs.contains(&"thermohaline_flux".to_string()));
    assert!(!OceanHeat::new().spec().inputs.contains(&"thermohaline_flux".to_string()));
    assert_eq!(OceanHeat::new().params().get("t_freeze"), Some(150.0));
    // Missing input -> error through the scheduler.
    let (d, mut f, _) = sphere_world(2, 0.0);
    let mut sc = Scheduler::new();
    sc.register(Box::new(SeaIce::new())).unwrap();
    assert!(sc.run(&d, &mut f, &DirtyRegion::All).is_err()); // no sst
}

#[test]
fn a_world_without_ocean_runs_and_gives_all_zero_fields() {
    let d = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 6, ny: 6 }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap();
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    for (k, l, v) in [("ocean_mask", 1, 0.0), ("coast_distance", 1, 0.0), ("coriolis_f", 1, 1e-4), ("precipitation", 1, 0.0), ("evaporation", 1, 0.0), ("temperature_sl", 1, 280.0), ("wind_stress", 3, 0.0), ("sst", 1, 0.0)] {
        f.put(k, l, vec![v; l * n]).unwrap();
    }
    let mut sc = Scheduler::new();
    add_ocean_secondary_stage(&mut sc).unwrap();
    sc.run(&d, &mut f, &DirtyRegion::All).unwrap();
    SeaIce::new().run(&d, &mut f, &DirtyRegion::All).unwrap();
    for k in ["upwelling", "salinity", "ocean_density", "deep_water_formation", "thermohaline_flux", "nutrients", "sea_ice"] {
        assert!(f.data(k).unwrap().iter().all(|&v| v == 0.0), "{k}");
    }
}

// ------------------------------------------------------------------------------------------------ real pipeline
fn full_schedulers() -> (Scheduler, Scheduler) {
    let mut a = Scheduler::new();
    add_atmosphere_ocean_pipeline_full(&mut a).unwrap();
    let mut b = Scheduler::new();
    add_ocean_climate_loop_full(&mut b).unwrap();
    (a, b)
}

#[test]
fn the_full_pipeline_produces_finite_bounded_fields_and_converges() {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let (mut a, mut b) = full_schedulers();
    let rep = run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 2).unwrap();
    println!("{rep:?}");
    assert!(rep.heat_loop_converged.iter().all(|&c| c), "ocean_climate loop must converge");
    let n = d.n_cells();
    for k in ["upwelling", "ekman_pumping", "coastal_upwelling", "salinity", "ocean_density", "deep_water_formation", "thermohaline_flux", "nutrients", "sea_ice", "sea_ice_fraction", "sst", "ocean_heat_flux", "albedo", "ice_fraction"] {
        assert!(f.data(k).unwrap().iter().all(|v| v.is_finite()), "{k}");
    }
    let mask = f.data("ocean_mask").unwrap();
    let (nu, h, fr) = (f.data("nutrients").unwrap(), f.data("sea_ice").unwrap(), f.data("sea_ice_fraction").unwrap());
    for i in 0..n {
        assert!((0.0..1.0).contains(&nu[i]));
        assert!(mask[i] > 0.5 || (nu[i] == 0.0 && h[i] == 0.0 && fr[i] == 0.0));
        assert!((0.0..=5.0).contains(&h[i]) && (0.0..=1.0).contains(&fr[i]));
    }
    // The heat flux (including the thermohaline nudge) still integrates to zero.
    let q = f.data("ocean_heat_flux").unwrap();
    let (tot, abs) = (area_sum(&d, q), (0..n).map(|i| d.mesh.area[i] * q[i].abs()).sum::<f64>());
    assert!(tot.abs() <= 1e-6 * abs.max(1.0), "{tot} {abs}");
    // Salinity mean conserved on the real world.
    let s = f.data("salinity").unwrap();
    let (mut aa, mut m) = (0.0, 0.0);
    for i in 0..n {
        if mask[i] > 0.5 {
            aa += d.mesh.area[i];
            m += d.mesh.area[i] * s[i];
        }
    }
    assert!((m / aa - 35.0).abs() < 1e-9);
    // Ice is where the sst is cold (loop relaxation leaves up to ~1 K of slack).
    let sst = f.data("sst").unwrap();
    for i in 0..n {
        if mask[i] > 0.5 {
            assert!(h[i] == 0.0 || sst[i] < 271.35 + 1.0, "ice at sst {}", sst[i]);
            assert!(h[i] > 0.0 || sst[i] > 271.35 - 1.0, "no ice at sst {}", sst[i]);
        }
    }
}

/// Loop B only (no currents): cold polar ocean ices up, the ice raises the albedo, and a stronger greenhouse shrinks the ice.
fn ice_state(greenhouse: f64, sea_ice_on: f64) -> (Domain, Vec<f64>, Vec<f64>, Vec<f64>) {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let n = d.n_cells();
    let z = f.data("elevation").unwrap().to_vec();
    let (mask, comp) = ocean_cells(&d, &z, &vec![0.0; n], 1.0);
    f.put("ocean_mask", 1, mask.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("ocean_component", 1, comp.iter().map(|&c| c as f64).collect()).unwrap();
    f.put("current", 3, vec![0.0; 3 * n]).unwrap();
    f.put("salinity", 1, mask.iter().map(|&o| if o { 35.0 } else { 0.0 }).collect()).unwrap();
    seed_ocean_heat_flux(&mut f).unwrap();
    let mut b = Scheduler::new();
    add_ocean_climate_loop_full(&mut b).unwrap();
    b.set_param("energy_balance", "greenhouse", greenhouse).unwrap();
    b.set_param("sea_ice", "sea_ice_on", sea_ice_on).unwrap();
    b.run(&d, &mut f, &DirtyRegion::All).unwrap();
    (d, f.data("sea_ice_fraction").unwrap().to_vec(), f.data("albedo").unwrap().to_vec(), f.data("sst").unwrap().to_vec())
}

#[test]
fn sea_ice_forms_in_the_cold_ocean_raises_the_albedo_and_shrinks_when_the_planet_warms() {
    let (d, fr, alb, sst) = ice_state(0.39, 1.0);
    let (_, fr_off, alb_off, _) = ice_state(0.39, 0.0);
    let (_, fr_warm, _, _) = ice_state(0.55, 1.0);
    let n = d.n_cells();
    let ice_area = |v: &[f64]| area_sum(&d, v);
    println!("ice area (area-weighted fraction): base {:.3e}, warmer {:.3e}, off {:.3e}", ice_area(&fr), ice_area(&fr_warm), ice_area(&fr_off));
    assert!(ice_area(&fr) > 0.0, "the cold polar ocean must ice up");
    assert!(fr_off.iter().all(|&v| v == 0.0));
    assert!(ice_area(&fr_warm) < ice_area(&fr), "warming shrinks the ice");
    // Ice only on cold water; albedo is higher where ice is (on vs. the switched-off run), on average.
    let (mut a_on, mut a_off, mut cnt) = (0.0, 0.0, 0usize);
    for i in 0..n {
        if fr[i] > 0.5 {
            assert!(sst[i] < 271.35 + 1.0);
            a_on += alb[i];
            a_off += alb_off[i];
            cnt += 1;
        }
    }
    assert!(cnt > 0);
    assert!(a_on / cnt as f64 > a_off / cnt as f64, "albedo over ice: {} vs {}", a_on / cnt as f64, a_off / cnt as f64);
}

#[test]
fn the_full_pipeline_is_deterministic() {
    let run = || {
        let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
        let (mut a, mut b) = full_schedulers();
        run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 2).unwrap();
        (f.data("sst").unwrap().to_vec(), f.data("upwelling").unwrap().to_vec(), f.data("salinity").unwrap().to_vec())
    };
    assert_eq!(run(), run());
}

/// Acceptance: "a cold coastal current yields a drier adjacent coast". Compares precipitation before and after the SST feedback (outer pass 1 vs 2
/// of the real coupled pipeline) on mid-latitude land next to ocean on its west (eastern-boundary, cold current) and on its east (western-boundary,
/// warm current), relative to the whole land band (the band itself dries slightly). Measured on `earth_like_world(4)` (kg/m^2/s; 1 mm/day = 1.16e-5):
/// cold coast -3.84e-7 (n 20), warm coast +5.81e-7 (n 18), band -1.91e-7 (n 338), i.e. -1.9e-7 and +7.7e-7 relative to the band. The thresholds below are
/// about a quarter of the measured margins.
#[test]
fn a_cold_coastal_current_yields_a_drier_coast_and_a_warm_one_a_wetter_coast() {
    let (d, mut f) = enbuilder::testworld::earth_like_world(4).unwrap();
    let (mut a, mut b) = full_schedulers();
    run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 1).unwrap();
    let p1 = f.data("precipitation").unwrap().to_vec();
    run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 2).unwrap();
    let p2 = f.data("precipitation").unwrap().to_vec();
    let mask: Vec<bool> = f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let (mut we, mut wn, mut ee, mut en, mut ra, mut rn) = (0.0, 0usize, 0.0, 0usize, 0.0, 0usize);
    for i in 0..d.n_cells() {
        let l = lat(&d, i).to_degrees().abs();
        if mask[i] || !(20.0..=55.0).contains(&l) {
            continue;
        }
        ra += p2[i] - p1[i];
        rn += 1;
        for k in d.mesh.adj_offsets[i] as usize..d.mesh.adj_offsets[i + 1] as usize {
            let j = d.mesh.adj[k] as usize;
            if !mask[j] {
                continue;
            }
            let dv = [d.mesh.centroid[j][0] - d.mesh.centroid[i][0], d.mesh.centroid[j][1] - d.mesh.centroid[i][1], d.mesh.centroid[j][2] - d.mesh.centroid[i][2]];
            let ew = dot(dv, d.frames[i].east);
            let len = dot(dv, dv).sqrt();
            if ew < -0.5 * len { ee += p2[i] - p1[i]; en += 1; }
            if ew > 0.5 * len { we += p2[i] - p1[i]; wn += 1; }
        }
    }
    let (cold, warm, band) = (ee / en.max(1) as f64, we / wn.max(1) as f64, ra / rn.max(1) as f64);
    println!("d(precip) next to ocean on the west (cold current): {cold:+.3e} (n {en}); on the east (warm current): {warm:+.3e} (n {wn}); land band mean {band:+.3e} (n {rn})");
    assert!(en > 10 && wn > 10 && rn > 100);
    assert!(cold - band < -5.0e-8, "cold coast relative to the band: {}", cold - band);
    assert!(warm - band > 1.5e-7, "warm coast relative to the band: {}", warm - band);
    assert!(warm > cold + 2.0e-7, "warm {warm} cold {cold}");
}
