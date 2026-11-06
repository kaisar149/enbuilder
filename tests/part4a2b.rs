//! Part 4a-2b acceptance tests: wind-stress curl (discrete Stokes circulation) and the beta term.
//! Analytic checks on plane, sphere (two orientations) and cube. NOT compiled when written (no toolchain); see ARCHITECTURE.md.
use enbuilder::generators::*;
use enbuilder::math::{add, cross, dot, length, normalize, scale, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_forcing::*;
use enbuilder::*;

const R: f64 = 6.371e6;
const OMEGA: f64 = 7.292e-5;

fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(nx: usize, ny: usize, b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}

fn rotate(v: Vec3, axis: Vec3, ang: f64) -> Vec3 {
    let k = normalize(axis);
    let (s, c) = ang.sin_cos();
    add(add(scale(v, c), scale(cross(k, v), s)), scale(k, dot(k, v) * (1.0 - c)))
}

/// Same sphere with the whole icosahedral mesh turned relative to the (fixed) spin axis +z.
fn rotated_sphere(level: u32) -> Domain {
    let d = sphere(level);
    let mut mesh = d.mesh.clone();
    for i in 0..mesh.n_cells {
        mesh.centroid[i] = rotate(mesh.centroid[i], [1.0, 2.0, 3.0], 0.7);
        mesh.up[i] = rotate(mesh.up[i], [1.0, 2.0, 3.0], 0.7);
    }
    let gravity: Vec<Vec3> = mesh.up.iter().map(|&u| scale(u, -9.81)).collect();
    Domain::new(DomainKind::Sphere, mesh, gravity, BoundaryKind::Closed, [0.0, 0.0, 1.0], 1).unwrap()
}

fn solid_body(d: &Domain, axis: Vec3, om: f64) -> Vec<Vec3> { d.mesh.centroid.iter().map(|&c| scale(cross(axis, c), om)).collect() }
fn flat(v: &[Vec3]) -> Vec<f64> { v.iter().flat_map(|p| p.to_vec()).collect() }
fn max_abs(a: &[f64]) -> f64 { a.iter().fold(0.0f64, |m, v| m.max(v.abs())) }

// ---------------------------------------------------------------------------------------------
// curl
// ---------------------------------------------------------------------------------------------

#[test]
fn plane_solid_body_rotation_has_constant_curl_two_omega() {
    for b in [BoundaryKind::Wall, BoundaryKind::Void] {
        let d = plane(40, 30, b);
        let om = 2.0e-5;
        let v = solid_body(&d, d.mesh.up[0], om);
        let c = curl_of(&d, &v);
        let mut worst_all = 0.0f64;
        let mut worst_interior = 0.0f64;
        for i in 0..d.n_cells() {
            let e = (c[i] / (2.0 * om) - 1.0).abs();
            worst_all = worst_all.max(e);
            if d.mesh.boundary_len[i] == 0.0 {
                worst_interior = worst_interior.max(e);
            }
        }
        println!("plane {b:?}: solid-body curl rel err interior {worst_interior:.2e}, all cells {worst_all:.2e}");
        assert!(worst_interior < 1e-6, "interior {worst_interior:.3e}");
        // The consistency correction keeps linear fields exact even where wall edges are missing.
        assert!(worst_all < 1e-6, "wall cells {worst_all:.3e}");
    }
}

#[test]
fn sphere_solid_body_rotation_matches_two_omega_sin_lat() {
    // Radial vorticity of rotation about the polar axis is 2 omega sin(lat) = 2 omega (z_hat . up).
    // (PART text says cos(lat); that is cos of the colatitude. Recorded in ARCHITECTURE.md.)
    let mut prev = f64::INFINITY;
    for level in [3u32, 4] {
        let d = sphere(level);
        let v = solid_body(&d, [0.0, 0.0, 1.0], OMEGA);
        let c = curl_of(&d, &v);
        let err = (0..d.n_cells()).map(|i| (c[i] - 2.0 * OMEGA * d.mesh.up[i][2]).abs()).fold(0.0, f64::max) / (2.0 * OMEGA);
        println!("sphere level {level}: solid-body curl max err / (2 omega) = {err:.3e}");
        assert!(err < 0.02, "level {level}: {err:.3e}");
        assert!(err < prev, "error should shrink with resolution");
        prev = err;
    }
}

#[test]
fn curl_of_gradient_vanishes_with_resolution() {
    let mut errs = Vec::new();
    for level in [3u32, 4, 5] {
        let d = sphere(level);
        // phi = x z / R; surface gradient = tangent part of (z, 0, x) / R.
        let g: Vec<Vec3> = d
            .mesh
            .centroid
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let amb = scale([c[2], 0.0, c[0]], 1.0 / R);
                let up = d.mesh.up[i];
                [amb[0] - dot(amb, up) * up[0], amb[1] - dot(amb, up) * up[1], amb[2] - dot(amb, up) * up[2]]
            })
            .collect();
        let gmax = g.iter().map(|&p| length(p)).fold(0.0, f64::max);
        let c = curl_of(&d, &g);
        let e = max_abs(&c) / (gmax / R);
        println!("sphere level {level}: max |curl grad phi| / (|grad phi|/R) = {e:.3e}");
        errs.push(e);
    }
    assert!(errs[1] < 0.7 * errs[0], "{errs:?}");
    assert!(errs[2] < 0.7 * errs[1], "{errs:?}");
    assert!(errs[2] < 0.01, "{errs:?}");
}

#[test]
fn uniform_field_has_exactly_zero_curl_on_every_topology() {
    let worlds = vec![
        ("sphere", sphere(4)),
        ("cube", cube()),
        ("plane wall", plane(20, 12, BoundaryKind::Wall)),
        ("plane wrap", plane(20, 12, BoundaryKind::Wrap)),
    ];
    for (name, d) in worlds {
        let v: Vec<Vec3> = vec![[1.0, 2.0, 3.0]; d.n_cells()];
        let c = curl_of(&d, &v);
        assert!(c.iter().all(|x| x.abs() < 1e-30), "{name}: uniform field gave nonzero curl");
        // Rough tangent-ish field: must be finite everywhere (cube corners, pentagons, wrapped edges).
        let w: Vec<Vec3> = d.mesh.centroid.iter().map(|&p| cross(d.mesh.up[0], p)).collect();
        assert!(curl_of(&d, &w).iter().all(|x| x.is_finite()), "{name}: non-finite curl");
    }
}

#[test]
fn masked_curl_uses_own_value_across_land_edges() {
    let d = plane(30, 20, BoundaryKind::Wall);
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| { let (x, y) = (i % 30, i / 30); (10..14).contains(&x) && (5..12).contains(&y) }).collect();
    let ocean: Vec<bool> = land.iter().map(|&l| !l).collect();
    let garbage = [1.0e9, -1.0e9, 1.0e9];

    // 1. Uniform ocean value, garbage on land: curl is exactly 0 on ocean cells (land values are never read), 0 on land.
    let v: Vec<Vec3> = (0..n).map(|i| if land[i] { garbage } else { [3.0, -4.0, 0.0] }).collect();
    let c = curl_of_masked(&d, &v, &ocean);
    assert!(c.iter().all(|x| x.abs() < 1e-30));

    // 2. Solid body: cells with no land neighbour equal the unmasked curl; cells next to land equal the hand-summed
    //    stencil over ocean neighbours only.
    let om = 1.0e-5;
    let mut sb = solid_body(&d, d.mesh.up[0], om);
    for i in 0..n {
        if land[i] {
            sb[i] = garbage;
        }
    }
    let op = CurlOp::new(&d);
    let masked = op.apply_masked(&d, &sb, &ocean);
    let mut clean = solid_body(&d, d.mesh.up[0], om);
    for i in 0..n {
        if land[i] {
            clean[i] = [0.0; 3];
        }
    }
    let unmasked = op.apply(&d, &clean);
    let mut n_coast = 0;
    for i in 0..n {
        if land[i] {
            assert_eq!(masked[i], 0.0);
            continue;
        }
        let (a, b) = (d.mesh.adj_offsets[i] as usize, d.mesh.adj_offsets[i + 1] as usize);
        let touches_land = (a..b).any(|k| land[d.mesh.adj[k] as usize]);
        if touches_land {
            n_coast += 1;
            let mut expect = 0.0;
            for k in a..b {
                let j = d.mesh.adj[k] as usize;
                if ocean[j] {
                    expect += dot(op.coef()[k], [sb[j][0] - sb[i][0], sb[j][1] - sb[i][1], sb[j][2] - sb[i][2]]);
                }
            }
            assert!((masked[i] - expect).abs() <= 1e-12 * (2.0 * om), "cell {i}");
            assert!(masked[i].is_finite());
        } else {
            assert!((masked[i] - unmasked[i]).abs() <= 1e-12 * (2.0 * om), "cell {i}");
        }
    }
    assert!(n_coast > 10);
}

// ---------------------------------------------------------------------------------------------
// beta
// ---------------------------------------------------------------------------------------------

/// (max relative error for |lat| < 60 deg, mean relative error per 10-degree band, positive for |lat| < 80 deg, all finite)
fn beta_stats(d: &Domain) -> (f64, Vec<f64>, bool, bool) {
    let f: Vec<f64> = (0..d.n_cells()).map(|i| 2.0 * OMEGA * dot(d.spin_axis, d.mesh.up[i])).collect();
    let b = beta_of(d, &f);
    let mut worst = 0.0f64;
    let mut sum = vec![0.0; 12];
    let mut cnt = vec![0usize; 12];
    let mut positive = true;
    for i in 0..d.n_cells() {
        let z = d.mesh.up[i][2];
        let lat = z.asin().to_degrees();
        if lat.abs() < 80.0 && !(b[i] > 0.0) {
            positive = false;
        }
        if lat.abs() < 60.0 {
            let truth = 2.0 * OMEGA * (1.0 - z * z).sqrt() / R;
            let rel = b[i] / truth - 1.0;
            worst = worst.max(rel.abs());
            let band = (((lat + 60.0) / 10.0).floor() as usize).min(11);
            sum[band] += rel;
            cnt[band] += 1;
        }
    }
    let means: Vec<f64> = (0..12).map(|k| if cnt[k] > 0 { sum[k] / cnt[k] as f64 } else { 0.0 }).collect();
    (worst, means, positive, b.iter().all(|v| v.is_finite()))
}

#[test]
fn beta_on_earth_like_sphere_matches_two_omega_cos_lat_over_r() {
    for level in [3u32, 4] {
        let d = sphere(level);
        let (worst, _means, positive, finite) = beta_stats(&d);
        println!("sphere level {level}: beta max rel err (|lat| < 60) = {worst:.3e}");
        assert!(worst < 0.05, "level {level}: {worst:.3e}");
        assert!(positive, "beta must be positive (f increases northward) away from the poles");
        assert!(finite);
    }
}

#[test]
fn beta_does_not_depend_on_mesh_orientation() {
    let a = sphere(4);
    let b = rotated_sphere(4);
    assert!(
        (0..5).any(|i| length([a.mesh.up[i][0] - b.mesh.up[i][0], a.mesh.up[i][1] - b.mesh.up[i][1], a.mesh.up[i][2] - b.mesh.up[i][2]]) > 0.05),
        "the rotated mesh should really differ"
    );
    let (wa, ma, pa, fa) = beta_stats(&a);
    let (wb, mb, pb, fb) = beta_stats(&b);
    println!("orientation: max rel err {wa:.3e} vs {wb:.3e}");
    assert!(wa < 0.05 && wb < 0.05);
    assert!(pa && pb && fa && fb);
    for k in 0..12 {
        assert!((ma[k] - mb[k]).abs() < 0.02, "band {k}: {:.4} vs {:.4}", ma[k], mb[k]);
    }
}

#[test]
fn beta_is_zero_on_an_f_plane_and_exact_on_a_beta_plane() {
    let d = plane(32, 24, BoundaryKind::Wall);
    let b0 = beta_of(&d, &vec![3.0e-5; d.n_cells()]);
    assert!(b0.iter().all(|v| v.abs() < 1e-30), "f-plane must give beta = 0");
    // f = f0 + beta0 * (position . north): beta recovered exactly (linear field, least-squares gradient).
    let north = d.frames[0].north;
    let beta0 = 2.0e-11;
    let f: Vec<f64> = d.mesh.centroid.iter().map(|&c| 1.0e-4 + beta0 * dot(c, north)).collect();
    let b = beta_of(&d, &f);
    // GradientOp regularises its 2x2 least-squares matrix by eps = 1e-6 (sxx + syy), which shrinks every gradient by
    // about 2e-6 relative (observed: 1.999996e-11 for 2e-11). That is Part 1 behaviour, so the tolerance is 1e-5.
    for i in 0..d.n_cells() {
        assert!((b[i] / beta0 - 1.0).abs() < 1e-5, "cell {i}: {:.6e}", b[i]);
    }
}

// ---------------------------------------------------------------------------------------------
// modules
// ---------------------------------------------------------------------------------------------

#[test]
fn modules_write_fields_zero_curl_on_land_and_match_pure_functions() {
    let d = sphere(4);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, (0..n).map(|i| if d.mesh.centroid[i][2] > 0.5 * R { 300.0 } else { -3000.0 }).collect()).unwrap();
    let tau = solid_body(&d, [0.0, 0.0, 1.0], 1.0e-9);
    f.put("wind_stress", 3, flat(&tau)).unwrap();
    let cf: Vec<f64> = (0..n).map(|i| 2.0 * OMEGA * dot(d.spin_axis, d.mesh.up[i])).collect();
    f.put("coriolis_f", 1, cf.clone()).unwrap();

    let mut s = Scheduler::new();
    add_ocean_mask_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();

    let mask = f.data("ocean_mask").unwrap().to_vec();
    let curl = f.data("stress_curl").unwrap().to_vec();
    assert_eq!(curl.len(), n);
    assert!(mask.iter().any(|&m| m < 0.5) && mask.iter().any(|&m| m > 0.5));
    assert!(curl.iter().all(|c| c.is_finite()));
    for i in 0..n {
        if mask[i] < 0.5 {
            assert_eq!(curl[i], 0.0, "land cell {i}");
        }
    }
    let ocean: Vec<bool> = mask.iter().map(|&m| m > 0.5).collect();
    let expect = curl_of_masked(&d, &tau, &ocean);
    assert_eq!(curl, expect);
    assert!(max_abs(&curl) > 0.0);

    assert_eq!(f.data("beta").unwrap().to_vec(), beta_of(&d, &cf));
}

#[test]
fn module_input_errors_and_no_parameters() {
    let d = plane(8, 8, BoundaryKind::Wall);
    let n = d.n_cells();
    let mut m = WindStressCurl::new();
    assert!(m.set_param("anything", 1.0).is_err());
    assert!(BetaTerm::new().set_param("anything", 1.0).is_err());
    let mut f = FieldStore::new(n);
    f.put("wind_stress", 1, vec![0.0; n]).unwrap(); // wrong layer count
    f.put("ocean_mask", 1, vec![1.0; n]).unwrap();
    assert!(matches!(m.run(&d, &mut f, &DirtyRegion::All), Err(Error::InvalidParam(_))));
    let mut f2 = FieldStore::new(n);
    let mut bad = vec![0.0; 3 * n];
    bad[4] = f64::NAN;
    f2.put("wind_stress", 3, bad).unwrap();
    f2.put("ocean_mask", 1, vec![1.0; n]).unwrap();
    assert!(matches!(m.run(&d, &mut f2, &DirtyRegion::All), Err(Error::NonFinite(_))));
    // Spec lists the registered names.
    let spec = WindStressCurl::new().spec();
    assert_eq!((spec.inputs, spec.outputs), (vec!["wind_stress".to_string(), "ocean_mask".to_string()], vec!["stress_curl".to_string()]));
}
