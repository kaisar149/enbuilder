//! Part 4a-2a acceptance tests: ocean subgraph, Dirichlet operator, per-component PCG. No physics.
use enbuilder::generators::*;
use enbuilder::ocean::*;
use enbuilder::rng::Rng;
use enbuilder::*;

const R: f64 = 6.371e6;

fn sphere() -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: 4, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(nx: usize, ny: usize, b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// Graph from a land predicate (true = land). min area tiny so even one cell counts as ocean.
fn graph_with_land(d: &Domain, land: impl Fn(usize) -> bool) -> OceanGraph {
    let z: Vec<f64> = (0..d.n_cells()).map(|i| if land(i) { 100.0 } else { -1000.0 }).collect();
    let sl = vec![0.0; d.n_cells()];
    let (ocean, comp) = ocean_cells(d, &z, &sl, 1.0);
    OceanGraph::new(d, &ocean, &comp).unwrap()
}

fn rough_field(n: usize, seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    (0..n).map(|_| r.range(-1.0, 1.0)).collect()
}

/// Smooth + rough manufactured solution on the ocean cells.
fn psi_true(g: &OceanGraph, d: &Domain, seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    let n = g.n;
    (0..n)
        .map(|l| {
            let c = d.mesh.centroid[g.cells[l] as usize];
            let s = 1.0e-6;
            (c[0] * s).sin() * (c[1] * 1.3 * s).cos() + 0.3 * (c[2] * 0.7 * s).sin() + 0.2 * r.range(-1.0, 1.0)
        })
        .collect()
}

fn max_abs_diff(a: &[f64], b: &[f64]) -> f64 { a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max) }
fn max_abs(a: &[f64]) -> f64 { a.iter().fold(0.0f64, |m, v| m.max(v.abs())) }

fn manufactured(g: &OceanGraph, d: &Domain, sigma: f64, tol: f64) -> (f64, usize) {
    let truth = psi_true(g, d, 7);
    let b = g.apply_shifted(sigma, &truth);
    let sol = g.solve(sigma, &b, &CgConfig::default()).unwrap();
    let err = max_abs_diff(&sol.psi, &truth) / max_abs(&truth);
    assert!(err < tol, "manufactured error {err:.3e} >= {tol:.1e} (sigma {sigma})");
    (err, sol.total_iterations)
}

// ---------------------------------------------------------------------------------------------

#[test]
fn manufactured_solution_plane_64x64_land_border() {
    let d = plane(64, 64, BoundaryKind::Wall);
    let g = graph_with_land(&d, |i| {
        let (x, y) = (i % 64, i / 64);
        x == 0 || y == 0 || x == 63 || y == 63
    });
    assert_eq!(g.n, 62 * 62);
    for sigma in [0.0, 1e-12, 1e-10] {
        let (e, it) = manufactured(&g, &d, sigma, 1e-8);
        println!("plane 64x64 sigma {sigma:e}: err {e:.2e}, iters {it}");
    }
}

#[test]
fn manufactured_solution_sphere_level4_land_cap() {
    let d = sphere();
    let g = graph_with_land(&d, |i| d.mesh.centroid[i][2] > 0.6 * R);
    assert!(g.n > 0 && g.n < d.n_cells());
    for sigma in [0.0, 1e-12] {
        let (e, it) = manufactured(&g, &d, sigma, 1e-8);
        println!("sphere L4 sigma {sigma:e}: err {e:.2e}, iters {it}");
    }
}

#[test]
fn manufactured_solution_cube_one_land_face() {
    let d = cube();
    let nn = 14 * 14;
    let g = graph_with_land(&d, |i| i < nn); // face 0 (+x) is land
    assert_eq!(g.n, 5 * nn);
    for sigma in [0.0, 1e-12] {
        let (e, it) = manufactured(&g, &d, sigma, 1e-8);
        println!("cube sigma {sigma:e}: err {e:.2e}, iters {it}");
    }
}

#[test]
fn operator_is_symmetric_and_negative_semidefinite() {
    let worlds: Vec<(&str, Domain, Box<dyn Fn(&Domain, usize) -> bool>)> = vec![
        ("plane", plane(40, 30, BoundaryKind::Wall), Box::new(|_d, i| { let (x, y) = (i % 40, i / 40); x < 3 || y == 29 || (x > 20 && x < 24 && y < 20) })),
        ("sphere", sphere(), Box::new(|d, i| d.mesh.centroid[i][2] > 0.5 * R)),
        ("cube", cube(), Box::new(|_d, i| i < 14 * 14)),
    ];
    for (name, d, landf) in worlds {
        let g = graph_with_land(&d, |i| landf(&d, i));
        for t in 0..5u64 {
            let x = rough_field(g.n, 10 + t);
            let y = rough_field(g.n, 100 + t);
            let (lx, ly) = (g.apply_l(&x), g.apply_l(&y));
            // Area-weighted symmetry (the natural inner product of this operator).
            let (a, b) = (g.dot_area(&x, &ly), g.dot_area(&lx, &y));
            let scale = g.dot_area(&x, &x).sqrt() * g.dot_area(&ly, &ly).sqrt();
            assert!((a - b).abs() / scale < 1e-12, "{name}: area-weighted asymmetry {:.3e}", (a - b).abs() / scale);
            // Negative semi-definite.
            let q = g.dot_area(&x, &lx);
            assert!(q <= 0.0, "{name}: <x,Lx> = {q:.3e} > 0");
            // sigma >= 0 makes -L + sigma I strictly positive.
            assert!(g.dot_area(&x, &g.apply_shifted(0.5, &x)) > 0.0);
        }
        // With all areas equal (plane) it is symmetric in the plain inner product too.
        if name == "plane" {
            let x = rough_field(g.n, 3);
            let y = rough_field(g.n, 4);
            let (lx, ly) = (g.apply_l(&x), g.apply_l(&y));
            let a: f64 = x.iter().zip(&ly).map(|(p, q)| p * q).sum();
            let b: f64 = lx.iter().zip(&y).map(|(p, q)| p * q).sum();
            let sc = x.iter().map(|v| v * v).sum::<f64>().sqrt() * ly.iter().map(|v| v * v).sum::<f64>().sqrt();
            assert!((a - b).abs() / sc < 1e-12, "plane Euclidean asymmetry");
        }
    }
}

#[test]
fn land_neighbour_contributes_dirichlet_term() {
    // 1 ocean cell, 4 land neighbours on a wall plane: (L psi) = -(sum w) psi exactly.
    let d = plane(5, 5, BoundaryKind::Wall);
    let g = graph_with_land(&d, |i| i != 12);
    assert_eq!(g.n, 1);
    assert!(g.coast_adjacent[0]);
    let w_total = 4.0 * (10_000.0 / 10_000.0) / d.mesh.area[12];
    let l = g.apply_l(&[2.0]);
    assert!((l[0] + w_total * 2.0).abs() < 1e-18 + 1e-12 * w_total);
}

#[test]
fn two_disjoint_oceans_solve_independently() {
    let d = plane(64, 40, BoundaryKind::Wall);
    // Land stripe at x = 30..33 splits the plane; land border.
    let land_all = |i: usize| { let (x, y) = (i % 64, i / 64); x == 0 || x == 63 || y == 0 || y == 39 || (30..33).contains(&x) };
    let g = graph_with_land(&d, land_all);
    assert_eq!(g.n_components(), 2);
    let truth = psi_true(&g, &d, 21);
    let b = g.apply_shifted(0.0, &truth);
    let joint = g.solve(0.0, &b, &CgConfig::default()).unwrap();
    assert!(max_abs_diff(&joint.psi, &truth) / max_abs(&truth) < 1e-8);

    // Each alone: turn the other ocean into land, keep the same b on the surviving cells.
    for keep in 0..2 {
        let in_keep = |i: usize| { let x = i % 64; if keep == 0 { x < 30 } else { x > 32 } };
        let alone_g = graph_with_land(&d, |i| land_all(i) || !in_keep(i));
        assert_eq!(alone_g.n_components(), 1);
        let b_alone: Vec<f64> = alone_g.cells.iter().map(|&c| b[g.local_of[c as usize] as usize]).collect();
        let alone = alone_g.solve(0.0, &b_alone, &CgConfig::default()).unwrap();
        for (l, &c) in alone_g.cells.iter().enumerate() {
            assert_eq!(alone.psi[l], joint.psi[g.local_of[c as usize] as usize], "component {keep} differs from solving it alone");
        }
        assert_eq!(alone.components[0].iterations, joint.components[keep].iterations);
    }
    // Changing the rhs in ocean 1 must not move ocean 0 at all.
    let mut b2 = b.clone();
    for &l in &g.comp_cells[1] { b2[l as usize] += 5.0; }
    let s2 = g.solve(0.0, &b2, &CgConfig::default()).unwrap();
    for &l in &g.comp_cells[0] { assert_eq!(s2.psi[l as usize], joint.psi[l as usize]); }
}

#[test]
fn empty_and_single_cell_oceans_do_not_error() {
    let d = plane(8, 8, BoundaryKind::Wall);
    // No ocean at all.
    let g0 = graph_with_land(&d, |_| true);
    assert_eq!(g0.n, 0);
    assert_eq!(g0.n_components(), 0);
    assert!(g0.split_by_component(&[]).is_empty());
    let s0 = g0.solve(0.0, &[], &CgConfig::default()).unwrap();
    assert!(s0.psi.is_empty() && s0.total_iterations == 0 && s0.components.is_empty());
    assert_eq!(g0.to_global(&s0.psi), vec![0.0; 64]);
    // One ocean cell: psi = A b / (sum_c + sigma A), exact in one iteration.
    let g1 = graph_with_land(&d, |i| i != 27);
    assert_eq!(g1.n, 1);
    assert_eq!(g1.split_by_component(&[3.0]), vec![vec![3.0]]);
    for sigma in [0.0, 2e-9] {
        let s = g1.solve(sigma, &[3.0], &CgConfig::default()).unwrap();
        let a = d.mesh.area[27];
        let expect = a * 3.0 / (4.0 + sigma * a);
        assert!((s.psi[0] - expect).abs() < 1e-12 * expect.abs());
        assert_eq!(s.components[0].n_cells, 1);
        assert!(s.components[0].iterations <= 2);
    }
    // Zero rhs: zero solution, zero iterations.
    let z = g1.solve(0.0, &[0.0], &CgConfig::default()).unwrap();
    assert_eq!((z.psi[0], z.total_iterations), (0.0, 0));
}

#[test]
fn coast_adjacent_flag_matches_independent_check() {
    for (name, d) in [("sphere", sphere()), ("cube", cube()), ("plane", plane(30, 20, BoundaryKind::Wrap))] {
        let n = d.n_cells();
        let land: Vec<bool> = (0..n).map(|i| (d.mesh.centroid[i][0] - d.mesh.centroid[0][0]).abs() < 0.25 * R * if name == "sphere" { 1.0 } else { 0.3 }).collect();
        let g = graph_with_land(&d, |i| land[i]);
        for l in 0..g.n {
            let c = g.cells[l] as usize;
            let expect = d.mesh.neighbors(c).iter().any(|&j| land[j as usize]);
            assert_eq!(g.coast_adjacent[l], expect, "{name} cell {c}");
            // Local CSR only keeps ocean neighbours.
            assert_eq!(g.neighbors(l).len(), d.mesh.neighbors(c).iter().filter(|&&j| !land[j as usize]).count());
        }
    }
}

#[test]
fn coastless_ocean_is_singular_but_solves_with_mean_zero() {
    // All-ocean sphere (no Dirichlet anchor): sigma = 0 is singular; the solver must still return a mean-zero solution.
    let d = sphere();
    let g = graph_with_land(&d, |_| false);
    assert_eq!(g.n, d.n_cells());
    assert!(g.coast_adjacent.iter().all(|&c| !c));
    let mut truth = psi_true(&g, &d, 5);
    let m = g.dot_area(&truth, &vec![1.0; g.n]) / d.mesh.total_area();
    for v in truth.iter_mut() { *v -= m; }
    let b = g.apply_l(&truth).iter().map(|v| -v).collect::<Vec<_>>();
    let s = g.solve(0.0, &b, &CgConfig::default()).unwrap();
    assert!(s.components[0].singular);
    assert!(max_abs_diff(&s.psi, &truth) / max_abs(&truth) < 1e-8);
    // With sigma > 0 it is an ordinary SPD problem.
    let (_e, _) = manufactured(&g, &d, 1e-12, 1e-8);
    let s2 = g.solve(1e-12, &g.apply_shifted(1e-12, &truth), &CgConfig::default()).unwrap();
    assert!(!s2.components[0].singular);
}

#[test]
fn iteration_count_reported_deterministic_and_guarded() {
    let d = plane(64, 64, BoundaryKind::Wall);
    let g = graph_with_land(&d, |i| { let (x, y) = (i % 64, i / 64); x == 0 || y == 0 || x == 63 || y == 63 });
    let truth = psi_true(&g, &d, 7);
    let b = g.apply_shifted(0.0, &truth);
    let s1 = g.solve(0.0, &b, &CgConfig::default()).unwrap();
    let s2 = g.solve(0.0, &b, &CgConfig::default()).unwrap();
    assert!(s1.total_iterations > 0);
    assert_eq!(s1.total_iterations, s1.components[0].iterations);
    assert_eq!(s1, s2, "solve must be bit-identical on repeat");
    assert!(s1.components[0].rel_residual <= 1e-10);
    // Guard returns NoConvergence instead of looping.
    match g.solve(0.0, &b, &CgConfig { tol: 1e-10, max_iters: 3 }) {
        Err(Error::NoConvergence { iterations, .. }) => assert_eq!(iterations, 3),
        other => panic!("expected NoConvergence, got {other:?}"),
    }
    // Bad inputs are errors, not panics.
    assert!(matches!(g.solve(-1.0, &b, &CgConfig::default()), Err(Error::InvalidParam(_))));
    assert!(matches!(g.solve(0.0, &b[1..], &CgConfig::default()), Err(Error::InvalidParam(_))));
    let mut bad = b.clone();
    bad[0] = f64::NAN;
    assert!(matches!(g.solve(0.0, &bad, &CgConfig::default()), Err(Error::NonFinite(_))));
}

#[test]
fn builds_from_ocean_mask_stage_fields_and_round_trips_global_local() {
    let d = sphere();
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, (0..d.n_cells()).map(|i| if d.mesh.centroid[i][0] > 0.3 * R { 300.0 } else { -3000.0 }).collect()).unwrap();
    let mut s = Scheduler::new();
    add_ocean_mask_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let g = OceanGraph::from_fields(&d, &f).unwrap();
    let mask = f.data("ocean_mask").unwrap();
    assert_eq!(g.n, mask.iter().filter(|&&m| m > 0.5).count());
    let v: Vec<f64> = (0..d.n_cells()).map(|i| i as f64 * mask[i]).collect();
    assert_eq!(g.to_global(&g.to_local(&v)), v);
    let b = rough_field(g.n, 2);
    assert_eq!(g.merge_by_component(&g.split_by_component(&b)), b);
}
