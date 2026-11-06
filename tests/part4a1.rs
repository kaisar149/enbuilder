//! Part 4a-1 acceptance tests: shared sea level, ocean mask, coast distance, wind stress.
//! On sphere, cube and (walled and wrapped) plane. NOT compiled when written (no toolchain); see ARCHITECTURE.md.
use enbuilder::generators::*;
use enbuilder::math::{cross, dot, length, scale, sub};
use enbuilder::ocean::*;
use enbuilder::*;

const R: f64 = 6.371e6;

fn sphere() -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: 4, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 40, ny: 24 }, cell_size: 10_000.0, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// Elevation: a continent (cos > 0.3 about +x as seen from the domain centre direction), a small deep pit (lake
/// candidate) inside it, ocean elsewhere. Works on any topology because it only uses centroids.
fn elevation(d: &Domain) -> Vec<f64> {
    let c: Vec<[f64; 3]> = d.mesh.centroid.clone();
    let mid = {
        let mut m = [0.0; 3];
        for p in &c { for k in 0..3 { m[k] += p[k] / c.len() as f64; } }
        m
    };
    let ext = c.iter().map(|p| length(sub(*p, mid))).fold(0.0, f64::max).max(1.0);
    let pit_center = c[d.n_cells() / 3];
    (0..d.n_cells())
        .map(|i| {
            let p = c[i];
            let dist_pit = length(sub(p, pit_center));
            if dist_pit < 0.06 * ext { return -50.0; } // small enclosed basin if surrounded by land, else part of the ocean
            if (p[0] - mid[0]) / ext > 0.3 { 200.0 } else { -3000.0 }
        })
        .collect()
}

fn run_mask(d: &Domain, z: Vec<f64>, sea: f64, min_area: f64) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z).unwrap();
    let mut s = Scheduler::new();
    add_ocean_mask_stage(&mut s).unwrap();
    s.set_param("sea_level", "sea_level", sea).unwrap();
    s.set_param("ocean_mask", "min_ocean_area_m2", min_area).unwrap();
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    f
}

fn domains() -> Vec<(&'static str, Domain)> {
    vec![("sphere", sphere()), ("cube", cube()), ("plane_wall", plane(BoundaryKind::Wall)), ("plane_wrap", plane(BoundaryKind::Wrap))]
}

/// Independent reference: flood-fill components with a plain BFS, compare to the module's ocean cells.
fn reference_ocean(d: &Domain, z: &[f64], sea: f64, min_area: f64) -> Vec<bool> {
    let n = d.n_cells();
    let thr = if min_area > 0.0 { min_area } else { 0.01 * d.mesh.total_area() };
    let mut seen = vec![false; n];
    let mut out = vec![false; n];
    for s in 0..n {
        if seen[s] || z[s] >= sea { continue; }
        let mut q = std::collections::VecDeque::from([s]);
        seen[s] = true;
        let mut members = vec![];
        while let Some(i) = q.pop_front() {
            members.push(i);
            for &j in d.mesh.neighbors(i) {
                let j = j as usize;
                if !seen[j] && z[j] < sea { seen[j] = true; q.push_back(j); }
            }
        }
        let a: f64 = members.iter().map(|&i| d.mesh.area[i]).sum();
        if a >= thr { for i in members { out[i] = true; } }
    }
    out
}

#[test]
fn ocean_cells_are_below_sea_level_and_in_big_components() {
    for (name, d) in domains() {
        let z = elevation(&d);
        let f = run_mask(&d, z.clone(), 0.0, 0.0);
        let mask = f.data("ocean_mask").unwrap();
        let want = reference_ocean(&d, &z, 0.0, 0.0);
        let n_ocean = mask.iter().filter(|&&m| m > 0.5).count();
        assert!(n_ocean > 0 && n_ocean < d.n_cells(), "{name}: degenerate test world ({n_ocean} ocean cells)");
        for i in 0..d.n_cells() {
            assert_eq!(mask[i] > 0.5, want[i], "{name}: cell {i} mask mismatch");
            if mask[i] > 0.5 { assert!(z[i] < 0.0, "{name}: ocean cell {i} above sea level"); }
        }
    }
}

#[test]
fn shared_sea_level_moves_the_coast_and_depth_is_relative_to_it() {
    let d = sphere();
    let z = elevation(&d);
    let lo = run_mask(&d, z.clone(), 0.0, 0.0);
    let hi = run_mask(&d, z.clone(), 500.0, 0.0); // flood the 200 m continent
    let (m0, m1) = (lo.data("ocean_mask").unwrap(), hi.data("ocean_mask").unwrap());
    assert!(m1.iter().sum::<f64>() > m0.iter().sum::<f64>(), "raising sea level must add ocean");
    let (dp, sl) = (hi.data("depth").unwrap(), hi.data("sea_level").unwrap());
    for i in 0..d.n_cells() {
        assert_eq!(sl[i], 500.0);
        if m1[i] > 0.5 { assert!((dp[i] - (500.0 - z[i])).abs() < 1e-9); } else { assert_eq!(dp[i], 0.0); }
    }
}

#[test]
fn small_enclosed_basin_is_a_lake_not_ocean() {
    let d = plane(BoundaryKind::Wall);
    let n = d.n_cells();
    // Plane is centred on the origin (x in -200..200 km). Land except a big sea at x < -50 km and a 2x2 pit near x = 100 km.
    let mut z = vec![300.0; n];
    for i in 0..n {
        let c = d.mesh.centroid[i];
        if c[0] < -50_000.0 { z[i] = -2000.0; }
    }
    let pit = (0..n).filter(|&i| { let c = d.mesh.centroid[i]; (c[0] - 100_000.0).abs() < 11_000.0 && c[1].abs() < 11_000.0 }).collect::<Vec<_>>();
    assert_eq!(pit.len(), 4, "pit should be 2x2 cells");
    for &i in &pit { z[i] = -100.0; }
    let f = run_mask(&d, z, 0.0, 0.0);
    let (m, id) = (f.data("ocean_mask").unwrap(), f.data("ocean_component").unwrap());
    for &i in &pit { assert_eq!(m[i], 0.0, "pit cell {i} must be a lake"); assert_eq!(id[i], -1.0); }
    assert!(m.iter().any(|&v| v > 0.5), "big sea must be ocean");
    // Lowering the threshold below the pit area turns it into ocean (its own component).
    let z2: Vec<f64> = (0..n).map(|i| if pit.contains(&i) { -100.0 } else if d.mesh.centroid[i][0] < -50_000.0 { -2000.0 } else { 300.0 }).collect();
    let g = run_mask(&d, z2, 0.0, 1.0);
    let ids: std::collections::BTreeSet<i64> = g.data("ocean_component").unwrap().iter().filter(|&&v| v >= 0.0).map(|&v| v as i64).collect();
    assert_eq!(ids.len(), 2, "sea and pit are separate components");
    assert_eq!(*ids.iter().next().unwrap(), 0);
}

#[test]
fn coast_distance_zero_on_coast_and_grows_away() {
    for (name, d) in domains() {
        let z = elevation(&d);
        let f = run_mask(&d, z, 0.0, 0.0);
        let (m, cd) = (f.data("ocean_mask").unwrap(), f.data("coast_distance").unwrap());
        let wall = matches!(d.boundary, BoundaryKind::Wall | BoundaryKind::Closed);
        for i in 0..d.n_cells() {
            if m[i] < 0.5 { assert_eq!(cd[i], 0.0, "{name}: land cell {i} must have 0"); continue; }
            let coast = d.mesh.neighbors(i).iter().any(|&j| m[j as usize] < 0.5) || (wall && d.mesh.boundary_len[i] > 0.0);
            if coast {
                assert_eq!(cd[i], 0.0, "{name}: coast cell {i}");
            } else {
                assert!(cd[i] > 0.0, "{name}: interior ocean cell {i} has distance 0");
                // Dijkstra optimality: a strictly closer ocean neighbour exists, and no edge changes it by more than its length.
                let nb = d.mesh.neighbors(i);
                let ds = d.mesh.edge_dists(i);
                let mut has_closer = false;
                for q in 0..nb.len() {
                    let j = nb[q] as usize;
                    if m[j] < 0.5 { continue; }
                    assert!((cd[i] - cd[j]).abs() <= ds[q] * (1.0 + 1e-12), "{name}: not 1-Lipschitz at {i}->{j}");
                    if cd[j] < cd[i] { has_closer = true; }
                }
                assert!(has_closer, "{name}: cell {i} has no closer neighbour (not monotone)");
            }
        }
        assert!(cd.iter().cloned().fold(0.0, f64::max) > 0.0, "{name}: no interior ocean");
    }
}

#[test]
fn walled_domain_edge_counts_as_coast_and_wrap_does_not() {
    let all_sea = |d: &Domain| vec![-1000.0; d.n_cells()];
    let w = plane(BoundaryKind::Wall);
    let fw = run_mask(&w, all_sea(&w), 0.0, 0.0);
    let cd = fw.data("coast_distance").unwrap();
    for i in 0..w.n_cells() { if w.mesh.boundary_len[i] > 0.0 { assert_eq!(cd[i], 0.0); } }
    assert!(cd.iter().any(|&v| v > 0.0));
    // Wrapped all-ocean plane has no coast anywhere: finite sentinel, no panic.
    let p = plane(BoundaryKind::Wrap);
    let fp = run_mask(&p, all_sea(&p), 0.0, 0.0);
    let want = p.mesh.total_area().sqrt();
    assert!(fp.data("coast_distance").unwrap().iter().all(|&v| v.is_finite() && (v - want).abs() < 1e-6));
}

#[test]
fn world_with_no_ocean_gives_all_zero_mask() {
    for (name, d) in domains() {
        let z = vec![100.0; d.n_cells()];
        let f = run_mask(&d, z, 0.0, 0.0);
        for k in ["ocean_mask", "coast_distance", "depth"] {
            assert!(f.data(k).unwrap().iter().all(|&v| v == 0.0), "{name}: {k} not all zero");
        }
        assert!(f.data("ocean_component").unwrap().iter().all(|&v| v == -1.0), "{name}: component ids");
    }
}

#[test]
fn all_ocean_sphere_is_one_component() {
    let d = sphere();
    let f = run_mask(&d, vec![-4000.0; d.n_cells()], 0.0, 0.0);
    assert!(f.data("ocean_component").unwrap().iter().all(|&v| v == 0.0));
}

#[test]
fn ocean_stage_is_deterministic() {
    let d = cube();
    let z = elevation(&d);
    let a = run_mask(&d, z.clone(), 0.0, 0.0);
    let b = run_mask(&d, z, 0.0, 0.0);
    for k in ["ocean_mask", "ocean_component", "coast_distance", "depth"] {
        assert_eq!(a.data(k).unwrap(), b.data(k).unwrap(), "{k}");
    }
}

// ---- wind stress -------------------------------------------------------------------------------

/// Tangent wind of ~8 m/s with varying direction.
fn tangent_wind(d: &Domain) -> Vec<f64> {
    let mut w = Vec::new();
    for i in 0..d.n_cells() {
        let up = d.mesh.up[i];
        let c = d.mesh.centroid[i];
        let a = [1.0 + c[1] * 1e-7, 0.5 - c[2] * 1e-7, 0.3 + c[0] * 1e-7];
        let t = sub(a, scale(up, dot(a, up)));
        let t = if length(t) < 1e-9 { cross(up, [0.0, 0.0, 1.0]) } else { t };
        w.extend_from_slice(&scale(t, 8.0 / length(t).max(1e-9)));
    }
    w
}

fn run_stress(d: &Domain, ice: Vec<f64>) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, elevation(d)).unwrap();
    f.put("wind", 3, tangent_wind(d)).unwrap();
    f.put("ice_fraction", 1, ice).unwrap();
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    f
}

#[test]
fn stress_is_parallel_to_wind_magnitude_correct_and_zero_on_land() {
    for (name, d) in domains() {
        let f = run_stress(&d, vec![0.0; d.n_cells()]);
        let (w, s, m) = (f.data("wind").unwrap(), f.data("wind_stress").unwrap(), f.data("ocean_mask").unwrap());
        let mut checked = 0;
        for i in 0..d.n_cells() {
            let (u, t) = ([w[3 * i], w[3 * i + 1], w[3 * i + 2]], [s[3 * i], s[3 * i + 1], s[3 * i + 2]]);
            if m[i] < 0.5 { assert_eq!(t, [0.0; 3], "{name}: land cell {i}"); continue; }
            let want = 1.2 * 1.3e-3 * length(u);
            let diff = length(sub(t, scale(u, want)));
            assert!(diff < 1e-9 * (1.0 + length(t)), "{name}: stress not rho*cd*|u|*u at {i}");
            assert!(length(cross(t, u)) < 1e-9 * (1.0 + length(t) * length(u)), "{name}: not parallel at {i}");
            assert!(dot(t, u) > 0.0);
            checked += 1;
        }
        assert!(checked > 0, "{name}: no ocean cells to check");
    }
}

#[test]
fn stress_scales_with_ice_and_vanishes_under_full_ice() {
    let d = sphere();
    let n = d.n_cells();
    let free = run_stress(&d, vec![0.0; n]);
    let half = run_stress(&d, vec![0.5; n]);
    let full = run_stress(&d, vec![1.0; n]);
    let (a, b, c) = (free.data("wind_stress").unwrap(), half.data("wind_stress").unwrap(), full.data("wind_stress").unwrap());
    assert!(a.iter().any(|&v| v != 0.0));
    for k in 0..3 * n {
        assert!((b[k] - 0.5 * a[k]).abs() < 1e-12);
        assert_eq!(c[k], 0.0);
    }
}

#[test]
fn stress_params_are_validated_and_adjustable() {
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    assert!(s.set_param("wind_stress", "cd_stress", -1.0).is_err());
    assert!(s.set_param("wind_stress", "nope", 1.0).is_err());
    assert!(s.set_param("ocean_mask", "min_ocean_area_m2", -5.0).is_err());
    s.set_param("wind_stress", "cd_stress", 2.6e-3).unwrap();
    let d = sphere();
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, elevation(&d)).unwrap();
    f.put("wind", 3, tangent_wind(&d)).unwrap();
    f.put("ice_fraction", 1, vec![0.0; d.n_cells()]).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let base = run_stress(&d, vec![0.0; d.n_cells()]);
    let (x, y) = (f.data("wind_stress").unwrap(), base.data("wind_stress").unwrap());
    for k in 0..x.len() { assert!((x[k] - 2.0 * y[k]).abs() < 1e-12); }
}

#[test]
fn scheduler_orders_sea_level_then_mask_then_stress() {
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |n: &str| order.iter().position(|m| m == n).unwrap();
    assert!(pos("sea_level") < pos("ocean_mask") && pos("ocean_mask") < pos("wind_stress"));
    let ext = s.external_inputs();
    for need in ["elevation", "wind", "ice_fraction"] { assert!(ext.contains(need), "{need} should be external"); }
}
