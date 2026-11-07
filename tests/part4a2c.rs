//! Part 4a-2c acceptance tests: gyres and the current field from `r lap(psi) = curl(tau)/(rho H)` (beta off).
//! NOT compiled when written (no Rust toolchain in that session); thresholds come from `reference/mirror_gyres.py` and
//! `reference/mirror_gyres2.py` (numpy, same geometry and stencils) with at least ~1.5x margin. See ARCHITECTURE.md.
use enbuilder::generators::*;
use enbuilder::math::{dot, length, scale, sub, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_currents::*;
use enbuilder::ocean_forcing::*;
use enbuilder::*;

const R: f64 = 6.371e6;
const BIG: f64 = 1.0e9; // "no clamp"

fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}

fn masks(d: &Domain, land: &[bool]) -> (Vec<bool>, Vec<i64>) {
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -1000.0 }).collect();
    ocean_cells(d, &z, &vec![0.0; n], 1.0)
}

fn forcing_fields(n: usize, ocean: &[bool], comp: &[i64], curl: Vec<f64>) -> FieldStore {
    let mut f = FieldStore::new(n);
    f.put("ocean_mask", 1, ocean.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("ocean_component", 1, comp.iter().map(|&c| c as f64).collect()).unwrap();
    f.put("stress_curl", 1, curl).unwrap();
    f
}

fn lat_of(d: &Domain, i: usize) -> f64 { d.mesh.up[i][2].asin() }

/// Sphere with two meridional land strips (|y| < 0.13 R, |lat| < 70 deg): one ocean joined over the poles.
/// Stress `-0.1 cos(3 lat)` eastward (trades at the equator, westerlies at 60 deg). Same world as `mirror_gyres.py`.
fn gyre_world(level: u32) -> (Domain, Vec<bool>, Vec<i64>, Vec<f64>) {
    let d = sphere(level);
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| d.mesh.centroid[i][1].abs() < 0.13 * R && lat_of(&d, i).abs() < 70f64.to_radians()).collect();
    let (ocean, comp) = masks(&d, &land);
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat_of(&d, i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    (d, ocean, comp, curl)
}

/// `FieldStore` is not `Clone`: every run starts from a fresh store built from the pieces.
fn fresh(d: &Domain, ocean: &[bool], comp: &[i64], curl: &[f64]) -> FieldStore { forcing_fields(d.n_cells(), ocean, comp, curl.to_vec()) }

fn run_with(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) -> OceanCurrents {
    let mut m = OceanCurrents::new();
    // 4a-2d: these are the 4a-2c tests, so the module is pinned to its 4a-2c configuration (no beta term, friction 5e-8)
    // unless a test overrides it. The 4a-2d defaults (beta_on 1, r_friction 1e-5) are tested in tests/part4a2d.rs.
    m.set_param("beta_on", 0.0).unwrap();
    m.set_param("r_friction", 5.0e-8).unwrap();
    for &(k, v) in params {
        m.set_param(k, v).unwrap();
    }
    m.run(d, f, &DirtyRegion::All).unwrap();
    m
}

fn currents(f: &FieldStore) -> Vec<Vec3> { unpack_vec3(f.data("current").unwrap()) }
fn max_abs(a: &[f64]) -> f64 { a.iter().fold(0.0f64, |m, v| m.max(v.abs())) }

/// Ocean cell with only ocean neighbours, and only ocean neighbours-of-neighbours (away from every coast).
fn deep_mask(d: &Domain, ocean: &[bool]) -> Vec<bool> {
    (0..d.n_cells())
        .map(|i| ocean[i] && d.neighbors(i).iter().all(|&j| ocean[j as usize] && d.neighbors(j as usize).iter().all(|&k| ocean[k as usize])))
        .collect()
}

/// Least-squares divergence consistent with `GradientOp`: `sum_k coef_k . (u_j - u_i)`.
fn div_ls(d: &Domain, coef: &[Vec3], u: &[Vec3], i: usize) -> f64 {
    let m = &d.mesh;
    let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
    (a..b).map(|k| dot(coef[k], sub(u[m.adj[k] as usize], u[i]))).sum()
}

fn band_mean(d: &Domain, mask: &[bool], vals: &[f64], lo_deg: f64, hi_deg: f64) -> f64 {
    let (mut s, mut c) = (0.0, 0usize);
    for i in 0..d.n_cells() {
        let l = lat_of(d, i).to_degrees();
        if mask[i] && l > lo_deg && l < hi_deg {
            s += vals[i];
            c += 1;
        }
    }
    assert!(c > 20, "band {lo_deg}..{hi_deg} has only {c} cells");
    s / c as f64
}

// ---------------------------------------------------------------------------------------------

#[test]
fn psi_zero_on_land_small_next_to_coast_and_no_net_flow_through_coast() {
    let (d, ocean, comp, curl) = gyre_world(4);
    let mut f = fresh(&d, &ocean, &comp, &curl);
    let n = d.n_cells();
    let m = run_with(&d, &mut f, &[("max_current", BIG)]);
    let psi = f.data("psi").unwrap().to_vec();
    for i in 0..n {
        if !ocean[i] {
            assert_eq!(psi[i], 0.0, "non-ocean cell {i}");
        }
    }
    // Next to the coast psi is the first interior value: well below the deep-ocean level (mirror ring/deep = 0.248 at L4).
    let deep = deep_mask(&d, &ocean);
    let ring: Vec<bool> = (0..n).map(|i| ocean[i] && d.neighbors(i).iter().any(|&j| !ocean[j as usize])).collect();
    let mean_abs = |mask: &[bool]| {
        let v: Vec<f64> = (0..n).filter(|&i| mask[i]).map(|i| psi[i].abs()).collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    let ratio = mean_abs(&ring) / mean_abs(&deep);
    println!("ring/deep mean |psi| = {ratio:.3}");
    assert!(ratio < 0.4, "ring/deep {ratio}");

    // Net flux through the land edges of every coast cell is zero with the slip rule on.
    let u = currents(&f);
    let flux = coast_outflux(&d, &ocean, &u);
    let cv = coast_vectors(&d, &ocean);
    let umax = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    let mut n_coast = 0usize;
    for i in 0..n {
        if cv[i].1 > 0.0 {
            n_coast += 1;
            assert!(flux[i].abs() <= 1e-9 * cv[i].1 * umax, "cell {i}: flux {} vs scale {}", flux[i], cv[i].1 * umax);
        } else {
            assert_eq!(flux[i], 0.0);
        }
    }
    assert!(n_coast > 50);
    assert_eq!(m.params().get("slipped_cells").unwrap() as usize, n_coast);

    // Without the slip the raw stream-function current does flow into the coast edges (mirror 0.58 at L4): the rule does work.
    let mut f2 = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut f2, &[("max_current", BIG), ("coast_slip", 0.0)]);
    let u2 = currents(&f2);
    let flux2 = coast_outflux(&d, &ocean, &u2);
    let umax2 = u2.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    let worst = (0..n).filter(|&i| cv[i].1 > 0.0).map(|i| flux2[i].abs() / (cv[i].1 * umax2)).fold(0.0f64, f64::max);
    println!("raw (no slip) max |F|/(L umax) = {worst:.3}");
    assert!(worst > 0.2);
}

#[test]
fn subtropical_gyres_opposite_senses_and_sign_follows_curl() {
    let (d, ocean, comp, curl) = gyre_world(4);
    let mut f = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut f, &[("max_current", BIG)]);
    let u = currents(&f);
    let psi = f.data("psi").unwrap().to_vec();
    let zeta = curl_of_masked(&d, &u, &ocean); // relative vorticity of the current
    let deep = deep_mask(&d, &ocean);
    let scale_r = 1025.0 * 100.0 * 5.0e-8;
    let expect: Vec<f64> = curl.iter().map(|c| c / scale_r).collect(); // r zeta = curl / (rho H)
    let (zn, zs) = (band_mean(&d, &deep, &zeta, 10.0, 40.0), band_mean(&d, &deep, &zeta, -40.0, -10.0));
    let (en, es) = (band_mean(&d, &deep, &expect, 10.0, 40.0), band_mean(&d, &deep, &expect, -40.0, -10.0));
    println!("zeta N {zn:.4e} (expect {en:.4e}), S {zs:.4e} (expect {es:.4e})");
    // Northern subtropics: negative curl (trades below, westerlies above) -> anticyclonic (negative) vorticity, psi > 0 hill.
    assert!(en < 0.0 && es > 0.0);
    assert!(zn < 0.0 && zs > 0.0, "gyre senses: N {zn}, S {zs}");
    assert!((zn / zs + 1.0).abs() < 0.1, "N/S not antisymmetric: {}", zn / zs);
    assert!((zn / en - 1.0).abs() < 0.1 && (zs / es - 1.0).abs() < 0.1, "vorticity vs curl/(rho H r): {} {}", zn / en, zs / es);
    assert!(band_mean(&d, &deep, &psi, 10.0, 40.0) > 0.0 && band_mean(&d, &deep, &psi, -40.0, -10.0) < 0.0);
}

#[test]
fn opposite_forcing_reverses_gyres_and_friction_scales_psi_inversely() {
    let (d, ocean, comp, curl) = gyre_world(4);
    let n = d.n_cells();
    let mut f1 = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut f1, &[]); // default clamp (3 m/s) is direction-symmetric
    let negc: Vec<f64> = curl.iter().map(|c| -c).collect();
    let mut neg = fresh(&d, &ocean, &comp, &negc);
    run_with(&d, &mut neg, &[]);
    let (p1, p2) = (f1.data("psi").unwrap(), neg.data("psi").unwrap());
    let scale_p = max_abs(p1);
    assert!(scale_p > 1.0);
    for i in 0..n {
        assert!((p1[i] + p2[i]).abs() <= 1e-12 * scale_p, "psi cell {i}");
    }
    let (c1, c2) = (f1.data("current").unwrap(), neg.data("current").unwrap());
    let scale_c = max_abs(c1);
    for k in 0..c1.len() {
        assert!((c1[k] + c2[k]).abs() <= 1e-12 * scale_c, "current component {k}");
    }
    // Doubling r_friction halves psi (a power-of-two rescale of the rhs is exact in floating point), and so does doubling H.
    let mut a = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut a, &[("max_current", BIG)]);
    let mut b = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut b, &[("max_current", BIG), ("r_friction", 1.0e-7)]);
    let mut c = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut c, &[("max_current", BIG), ("H_mixed_m", 200.0)]);
    let (pa, pb, pc) = (a.data("psi").unwrap(), b.data("psi").unwrap(), c.data("psi").unwrap());
    let sa = max_abs(pa);
    for i in 0..n {
        assert!((pb[i] - 0.5 * pa[i]).abs() <= 1e-12 * sa, "r doubling, cell {i}");
        assert!((pc[i] - 0.5 * pa[i]).abs() <= 1e-12 * sa, "H doubling, cell {i}");
    }
    // The current is linear in psi as well (no clamp here).
    let (ua, ub) = (a.data("current").unwrap(), b.data("current").unwrap());
    let su = max_abs(ua);
    for k in 0..ua.len() {
        assert!((ub[k] - 0.5 * ua[k]).abs() <= 1e-12 * su);
    }
}

#[test]
fn flat_plane_with_uniform_wind_has_no_flow() {
    let d = plane(40, 30);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, (0..n).map(|i| { let (x, y) = (i % 40, i / 40); if x == 0 || y == 0 || x == 39 || y == 29 { 100.0 } else { -1000.0 } }).collect()).unwrap();
    f.put("wind_stress", 3, (0..n).flat_map(|_| [5.0e-2, 3.0e-2, 0.0]).collect()).unwrap();
    f.put("coriolis_f", 1, vec![1.0e-4; n]).unwrap(); // f-plane: beta = 0, so even with beta_on = 1 (default) nothing is advected
    let mut s = Scheduler::new();
    add_ocean_mask_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("ocean_mask").unwrap().iter().any(|&m| m > 0.5));
    assert_eq!(max_abs(f.data("stress_curl").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("psi").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current_speed").unwrap()), 0.0);
    assert_eq!(s.module_param("ocean_currents", "cg_iterations"), Some(0.0));
    assert_eq!(s.module_param("ocean_currents", "clamped_cells"), Some(0.0));
}

#[test]
fn current_is_divergence_free_to_roundoff_on_the_plane() {
    let d = plane(40, 30);
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| { let (x, y) = (i % 40, i / 40); x == 0 || y == 0 || x == 39 || y == 29 }).collect();
    let (ocean, comp) = masks(&d, &land);
    let ly = 30.0 * 10_000.0;
    let tau: Vec<Vec3> = (0..n).map(|i| [-0.1 * (2.0 * std::f64::consts::PI * d.mesh.centroid[i][1] / ly).cos(), 0.0, 0.0]).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let mut f = forcing_fields(n, &ocean, &comp, curl);
    run_with(&d, &mut f, &[("max_current", BIG)]);
    let u = currents(&f);
    let umax = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    assert!(umax > 1.0, "umax {umax}");
    let deep = deep_mask(&d, &ocean);
    let g = GradientOp::new(&d);
    let mut worst = 0.0f64;
    for i in 0..n {
        if deep[i] {
            worst = worst.max(div_ls(&d, g.coef(), &u, i).abs());
        }
    }
    let rel = worst * 10_000.0 / umax;
    println!("plane: max |div| h / max|u| = {rel:.2e}");
    assert!(rel < 1e-9, "rel {rel}");
    // Straight coasts: the raw current also has no net coast flux after slip (checked in the sphere test); here just tangency.
    for i in 0..n {
        assert!(dot(u[i], d.frames[i].up).abs() <= 1e-12 * umax);
    }
}

#[test]
fn sphere_current_is_nearly_divergence_free() {
    // The stream-function construction is exactly non-divergent only in the continuum; on the hex-pent mesh the
    // discrete divergence is a discretisation error. Mirror: max 0.026, rms 0.012 of the vorticity scale at L4.
    let (d, ocean, comp, curl) = gyre_world(4);
    let mut f = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut f, &[("max_current", BIG)]);
    let u = currents(&f);
    let zeta = curl_of_masked(&d, &u, &ocean);
    let deep = deep_mask(&d, &ocean);
    let g = GradientOp::new(&d);
    let (mut dmax, mut zmax, mut d2, mut z2, mut cnt) = (0.0f64, 0.0f64, 0.0, 0.0, 0usize);
    for i in 0..d.n_cells() {
        if deep[i] {
            let dv = div_ls(&d, g.coef(), &u, i);
            dmax = dmax.max(dv.abs());
            zmax = zmax.max(zeta[i].abs());
            d2 += dv * dv;
            z2 += zeta[i] * zeta[i];
            cnt += 1;
        }
    }
    assert!(cnt > 500);
    let (rmax, rrms) = (dmax / zmax, (d2 / z2).sqrt());
    println!("sphere L4: max |div|/max|zeta| = {rmax:.4}, rms ratio = {rrms:.4}");
    assert!(rmax < 0.08, "max ratio {rmax}");
    assert!(rrms < 0.04, "rms ratio {rrms}");
}

#[test]
fn clamp_rule_and_count() {
    let (d, ocean, comp, curl) = gyre_world(4);
    let n = d.n_cells();
    let n_ocean = ocean.iter().filter(|&&o| o).count();
    let mut raw = fresh(&d, &ocean, &comp, &curl);
    let m_raw = run_with(&d, &mut raw, &[("max_current", BIG)]);
    assert_eq!(m_raw.params().get("clamped_cells"), Some(0.0));
    let ur = currents(&raw);
    let raw_max = ur.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    assert!((m_raw.params().get("max_speed_raw").unwrap() - raw_max).abs() <= 1e-12 * raw_max);

    for vmax in [3.0, 10.0] {
        let expected = ur.iter().filter(|v| length(**v) > vmax).count();
        let mut g = fresh(&d, &ocean, &comp, &curl);
        let m = run_with(&d, &mut g, &[("max_current", vmax)]);
        let reported = m.params().get("clamped_cells").unwrap() as usize;
        println!("max_current {vmax}: raw max {raw_max:.2} m/s, clamped {reported} of {n_ocean} ocean cells");
        assert_eq!(reported, expected);
        let uc = currents(&g);
        let sp = g.data("current_speed").unwrap();
        for i in 0..n {
            assert!(sp[i] <= vmax * (1.0 + 1e-12));
            assert!((sp[i] - length(uc[i])).abs() <= 1e-12 * vmax);
            let s = length(ur[i]);
            let want = if s > vmax { scale(ur[i], vmax / s) } else { ur[i] };
            assert!(length(sub(uc[i], want)) <= 1e-12 * (vmax + s), "cell {i}: direction or magnitude changed by the clamp");
        }
    }
    assert!(raw_max > 10.0, "with the default r_friction the friction-only gyre is far above max_current (raw max {raw_max})");

    // A friction of 4e-6 1/s gives ocean-like speeds (mirror: 0.24 m/s at L4) and nothing is clamped.
    let mut g = fresh(&d, &ocean, &comp, &curl);
    let m = run_with(&d, &mut g, &[("r_friction", 4.0e-6)]);
    let top = max_abs(g.data("current_speed").unwrap());
    println!("r_friction 4e-6: max speed {top:.3} m/s");
    assert_eq!(m.params().get("clamped_cells"), Some(0.0));
    assert!(top > 0.1 && top < 0.6, "top {top}");
}

#[test]
fn finite_tangent_zero_on_land_and_deterministic() {
    let (d, ocean, comp, curl) = gyre_world(4);
    let n = d.n_cells();
    let mut a = fresh(&d, &ocean, &comp, &curl);
    let mut b = fresh(&d, &ocean, &comp, &curl);
    run_with(&d, &mut a, &[]);
    run_with(&d, &mut b, &[]);
    for name in ["psi", "current", "current_speed"] {
        let (x, y) = (a.data(name).unwrap(), b.data(name).unwrap());
        assert!(x.iter().all(|v| v.is_finite()), "{name} not finite");
        assert_eq!(x, y, "{name} not deterministic");
    }
    assert_eq!(a.layers("current").unwrap(), 3);
    let u = currents(&a);
    let sp = a.data("current_speed").unwrap();
    let umax = max_abs(sp);
    assert!(umax > 0.0);
    for i in 0..n {
        if !ocean[i] {
            assert_eq!(u[i], [0.0; 3]);
            assert_eq!(sp[i], 0.0);
        } else {
            assert!(dot(u[i], d.frames[i].up).abs() <= 1e-12 * umax, "cell {i} not tangent");
        }
    }
}

#[test]
fn cube_smoke_one_land_face() {
    let d = cube();
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| d.mesh.centroid[i][2] > 2.9e6).collect();
    assert!(land.iter().any(|&l| l) && land.iter().any(|&l| !l));
    let (ocean, comp) = masks(&d, &land);
    let curl: Vec<f64> = (0..n).map(|i| if ocean[i] { 1.0e-8 * (d.mesh.centroid[i][0] / 1.0e6).sin() * (d.mesh.centroid[i][1] / 1.3e6).cos() } else { 0.0 }).collect();
    let mut f = forcing_fields(n, &ocean, &comp, curl);
    let m = run_with(&d, &mut f, &[("max_current", BIG)]);
    let psi = f.data("psi").unwrap();
    assert!(psi.iter().all(|v| v.is_finite()) && max_abs(psi) > 0.0);
    for i in 0..n {
        if !ocean[i] {
            assert_eq!(psi[i], 0.0);
        }
    }
    let u = currents(&f);
    let flux = coast_outflux(&d, &ocean, &u);
    let cv = coast_vectors(&d, &ocean);
    let umax = u.iter().fold(0.0f64, |mx, v| mx.max(length(*v)));
    for i in 0..n {
        assert!(flux[i].abs() <= 1e-9 * cv[i].1 * umax + 1e-300);
    }
    assert!(m.params().get("cg_iterations").unwrap() > 0.0);
}

#[test]
fn all_ocean_sphere_has_no_anchor_and_stays_finite() {
    let d = sphere(3);
    let n = d.n_cells();
    let ocean = vec![true; n];
    let comp = vec![0i64; n];
    let curl: Vec<f64> = (0..n).map(|i| 1.0e-8 * d.mesh.up[i][0] * d.mesh.up[i][2]).collect();
    let mut f = forcing_fields(n, &ocean, &comp, curl);
    let m = run_with(&d, &mut f, &[("max_current", BIG)]);
    let psi = f.data("psi").unwrap();
    assert!(psi.iter().all(|v| v.is_finite()));
    let sa: f64 = d.mesh.area.iter().sum();
    let mean = (0..n).map(|i| d.mesh.area[i] * psi[i]).sum::<f64>() / sa;
    assert!(mean.abs() <= 1e-9 * max_abs(psi), "area-weighted mean psi {mean}");
    assert_eq!(m.params().get("slipped_cells"), Some(0.0));
    assert!(max_abs(f.data("current").unwrap()).is_finite());
}

#[test]
fn no_ocean_gives_zero_fields() {
    let d = sphere(3);
    let n = d.n_cells();
    let mut f = forcing_fields(n, &vec![false; n], &vec![-1i64; n], vec![0.0; n]);
    run_with(&d, &mut f, &[]);
    assert_eq!(max_abs(f.data("psi").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current").unwrap()), 0.0);
}

#[test]
fn wiring_spec_defaults_and_errors() {
    let spec = OceanCurrents::new().spec();
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    assert_eq!(spec.inputs, v(&["stress_curl", "beta", "ocean_mask", "ocean_component"]));
    assert_eq!(spec.outputs, v(&["psi", "current", "current_speed", "boundary_layer_m"]));
    let mut m = OceanCurrents::new();
    let p = m.params().clone();
    assert_eq!((p.get("H_mixed_m"), p.get("r_friction"), p.get("rho_water"), p.get("max_current"), p.get("beta_on")), (Some(100.0), Some(1.0e-5), Some(1025.0), Some(3.0), Some(1.0))); // 4a-2d defaults
    for (k, bad) in [("H_mixed_m", 0.0), ("r_friction", -1.0), ("rho_water", f64::NAN), ("max_current", 0.0), ("beta_on", 0.5), ("coast_slip", 0.5)] {
        assert!(matches!(m.set_param(k, bad), Err(Error::InvalidParam(_))), "{k}={bad}");
    }
    assert!(m.set_param("clamped_cells", 1.0).is_err(), "output-only parameter must be read-only");
    assert!(m.set_param("nope", 1.0).is_err());
    assert!(m.set_param("beta_on", 0.0).is_ok() && m.set_param("beta_on", 1.0).is_ok() && m.set_param("coast_slip", 0.0).is_ok() && m.set_param("r_friction", 1e-6).is_ok());

    // Scheduler: sea_level -> ocean_mask -> (wind_stress_curl) -> ocean_currents, then a full run with zero wind.
    let d = sphere(3);
    let n = d.n_cells();
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |name: &str| order.iter().position(|x| x == name).unwrap_or_else(|| panic!("{name} missing from {order:?}"));
    assert!(pos("ocean_mask") < pos("wind_stress_curl") && pos("wind_stress_curl") < pos("ocean_currents"));
    assert!(pos("sea_level") < pos("ocean_mask"));
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, (0..n).map(|i| if d.mesh.centroid[i][2] > 0.5 * R { 300.0 } else { -3000.0 }).collect()).unwrap();
    f.put("wind", 3, vec![0.0; 3 * n]).unwrap();
    f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
    f.put("coriolis_f", 1, (0..n).map(|i| 1.4584e-4 * d.mesh.up[i][2]).collect()).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert_eq!(max_abs(f.data("psi").unwrap()), 0.0);
    assert!(f.has("current") && f.has("current_speed"));

    // Direct-run input errors.
    let mut m = OceanCurrents::new();
    let mut g = FieldStore::new(n);
    assert!(m.run(&d, &mut g, &DirtyRegion::All).is_err()); // nothing there
    g.put("ocean_mask", 1, vec![1.0; n]).unwrap();
    g.put("ocean_component", 1, vec![0.0; n]).unwrap();
    g.put("stress_curl", 3, vec![0.0; 3 * n]).unwrap();
    assert!(matches!(m.run(&d, &mut g, &DirtyRegion::All), Err(Error::InvalidParam(_))));
    let mut bad = vec![0.0; n];
    bad[3] = f64::NAN;
    // `put` cannot change a field's layer count, so use a fresh store for the NaN case.
    let mut g2 = FieldStore::new(n);
    g2.put("ocean_mask", 1, vec![1.0; n]).unwrap();
    g2.put("ocean_component", 1, vec![0.0; n]).unwrap();
    g2.put("stress_curl", 1, bad).unwrap();
    assert!(matches!(m.run(&d, &mut g2, &DirtyRegion::All), Err(Error::NonFinite(_))));
}
