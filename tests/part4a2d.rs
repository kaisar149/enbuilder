//! Part 4a-2d acceptance tests: upwinded beta term, BiCGSTAB, western intensification.
//! Part (i): the non-symmetric solver on synthetic problems with a known answer. Part (ii): gyres with beta on.
use enbuilder::generators::*;
use enbuilder::math::{dot, length, scale, sub, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_currents::*;
use enbuilder::ocean_forcing::*;
use enbuilder::*;

const R: f64 = 6.371e6;
const OMEGA: f64 = 7.292e-5;
const BIG: f64 = 1.0e9;

fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn lat_of(d: &Domain, i: usize) -> f64 { d.mesh.up[i][2].asin() }
fn lon_deg(d: &Domain, i: usize) -> f64 { d.mesh.up[i][1].atan2(d.mesh.up[i][0]).to_degrees() }

fn masks(d: &Domain, land: &[bool]) -> (Vec<bool>, Vec<i64>) {
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -1000.0 }).collect();
    ocean_cells(d, &z, &vec![0.0; n], 1.0)
}

struct World { d: Domain, ocean: Vec<bool>, comp: Vec<i64>, curl: Vec<f64>, beta: Vec<f64> }

fn earth_beta(d: &Domain) -> Vec<f64> {
    let f: Vec<f64> = (0..d.n_cells()).map(|i| 2.0 * OMEGA * d.mesh.up[i][2]).collect();
    beta_of(d, &f)
}

/// Same world as 4a-2c: two meridional land strips (|y| < 0.13 R, |lat| < 70 deg) on a sphere, stress `-0.1 cos(3 lat)` eastward.
fn gyre_world(level: u32) -> World {
    let d = sphere(level);
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| d.mesh.centroid[i][1].abs() < 0.13 * R && lat_of(&d, i).abs() < 70f64.to_radians()).collect();
    let (ocean, comp) = masks(&d, &land);
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat_of(&d, i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let beta = earth_beta(&d);
    World { d, ocean, comp, curl, beta }
}

fn fields(w: &World, beta: &[f64]) -> FieldStore {
    let n = w.d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("ocean_mask", 1, w.ocean.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("ocean_component", 1, w.comp.iter().map(|&c| c as f64).collect()).unwrap();
    f.put("stress_curl", 1, w.curl.clone()).unwrap();
    f.put("beta", 1, beta.to_vec()).unwrap();
    f
}

fn run(w: &World, beta: &[f64], params: &[(&str, f64)]) -> (OceanCurrents, FieldStore) {
    let mut f = fields(w, beta);
    let mut m = OceanCurrents::new();
    for &(k, v) in params {
        m.set_param(k, v).unwrap();
    }
    m.run(&w.d, &mut f, &DirtyRegion::All).unwrap();
    (m, f)
}

fn unpack(f: &FieldStore) -> Vec<Vec3> { unpack_vec3(f.data("current").unwrap()) }
fn max_abs(a: &[f64]) -> f64 { a.iter().fold(0.0f64, |m, v| m.max(v.abs())) }

/// Peak `current_speed` within 15 degrees of the west coast and of the east coast of each of the four subtropical gyres
/// (two hemispheres x two basins, latitude band 10..40). Basin A is lon 0..180 (west coast at lon ~0+), basin B is lon 180..360.
fn flank_peaks(w: &World, speed: &[f64]) -> Vec<(String, f64, f64)> {
    let d = &w.d;
    let mut out = vec![];
    for (hemi, lo, hi) in [("N", 10.0, 40.0), ("S", -40.0, -10.0)] {
        for basin in 0..2 {
            let (mut west, mut east) = (0.0f64, 0.0f64);
            for i in 0..d.n_cells() {
                if !w.ocean[i] {
                    continue;
                }
                let l = lat_of(d, i).to_degrees();
                if l < lo || l > hi {
                    continue;
                }
                let mut lam = lon_deg(d, i) + if basin == 1 { 180.0 } else { 0.0 };
                if lam < -180.0 { lam += 360.0; }
                if lam > 180.0 { lam -= 360.0; }
                if !(lam > 0.0 && lam < 180.0) {
                    continue;
                }
                let edge = (0.13 / lat_of(d, i).cos()).asin().to_degrees(); // land strip half width in longitude at this latitude
                if lam < edge + 15.0 { west = west.max(speed[i]); }
                if lam > 180.0 - edge - 15.0 { east = east.max(speed[i]); }
            }
            out.push((format!("{hemi}{basin}"), west, east));
        }
    }
    out
}

fn deep_mask(d: &Domain, ocean: &[bool]) -> Vec<bool> {
    (0..d.n_cells())
        .map(|i| ocean[i] && d.neighbors(i).iter().all(|&j| ocean[j as usize] && d.neighbors(j as usize).iter().all(|&k| ocean[k as usize])))
        .collect()
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
    assert!(c > 20);
    s / c as f64
}

// ---------------------------------------------------------------------------------------------
// Part (i): solver and operator on synthetic problems
// ---------------------------------------------------------------------------------------------

/// Independent matrix-free `(-L + N + sigma I) x` using only the public graph data (coef, flux, land_inflow), written the
/// "textbook" way (per edge, upwind value, difference form), not as the solver's matrix.
fn apply_full(g: &OceanGraph, adv: &OceanAdvection, sigma: f64, x: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; g.n];
    for i in 0..g.n {
        let (a, b) = (g.offsets[i] as usize, g.offsets[i + 1] as usize);
        let mut lap = -g.land_coef[i] * x[i];
        let mut nn = 0.0;
        for q in a..b {
            let j = g.adj[q] as usize;
            lap += g.coef[q] * (x[j] - x[i]);
            let v = adv.flux[q]; // outflow positive; upstream is i for outflow, the neighbour for inflow
            let up = if v >= 0.0 { x[i] } else { x[j] };
            nn += v * (up - x[i]);
        }
        // land neighbours: inflow edges (v < 0) have upstream value 0, so v (0 - x_i) = |v| x_i and land_inflow = sum |v|
        nn += adv.land_inflow[i] * x[i];
        out[i] = -lap / g.area[i] + nn / g.area[i] + sigma * x[i];
    }
    out
}

fn land_border_plane(nx: usize, ny: usize) -> (Domain, Vec<bool>, Vec<i64>) {
    let d = plane(nx, ny);
    let land: Vec<bool> = (0..nx * ny).map(|i| { let (x, y) = (i % nx, i / nx); x == 0 || y == 0 || x == nx - 1 || y == ny - 1 }).collect();
    let (ocean, comp) = masks(&d, &land);
    (d, ocean, comp)
}

fn smooth_plus_noise(d: &Domain, g: &OceanGraph, scale_len: f64) -> Vec<f64> {
    let mut seed = 12345u64;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64 / (1u64 << 31) as f64) - 0.5
    };
    g.cells
        .iter()
        .map(|&c| {
            let p = d.mesh.centroid[c as usize];
            (p[0] / scale_len).sin() * (p[1] / scale_len).cos() + 0.2 * rnd()
        })
        .collect()
}

#[test]
fn bicgstab_with_zero_advection_agrees_with_cg() {
    let (d, ocean, comp) = land_border_plane(40, 30);
    let g = OceanGraph::new(&d, &ocean, &comp).unwrap();
    let adv = g.advection(&d, &vec![[0.0; 3]; d.n_cells()]).unwrap();
    assert_eq!(adv.max_abs_flux(), 0.0);
    let b = smooth_plus_noise(&d, &g, 80_000.0);
    let cfg = CgConfig::default();
    let cg = g.solve(0.0, &b, &cfg).unwrap();
    let bi = g.solve_advective(0.0, &adv, &b, &cfg).unwrap();
    let scale_x = cg.psi.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let err = cg.psi.iter().zip(&bi.psi).fold(0.0f64, |m, (a, c)| m.max((a - c).abs())) / scale_x;
    println!("zero advection: BiCGSTAB vs CG max rel diff {err:.2e}, iterations {} vs {}", bi.total_iterations, cg.total_iterations);
    assert!(err < 1e-8, "{err}");
    assert_eq!(bi.components[0].restarts, 0);
    // also with a shift
    let cg2 = g.solve(1e-9, &b, &cfg).unwrap();
    let bi2 = g.solve_advective(1e-9, &adv, &b, &cfg).unwrap();
    let s2 = cg2.psi.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(cg2.psi.iter().zip(&bi2.psi).all(|(a, c)| (a - c).abs() < 1e-8 * s2));
}

#[test]
fn manufactured_nonsymmetric_problems_have_the_known_answer() {
    // Plane with a uniform "wind" c, and the sphere with the real -(beta/r) east field: rhs := K psi_true, solve, compare.
    let cfg = CgConfig::default();
    // (a) plane, oblique uniform advection, strong (cell Peclet ~ 5)
    let (d, ocean, comp) = land_border_plane(40, 30);
    let g = OceanGraph::new(&d, &ocean, &comp).unwrap();
    let c: Vec<Vec3> = (0..d.n_cells()).map(|_| [-5.0e-4, 2.0e-4, 0.0]).collect(); // 1/m; |c| dx = 5.4
    let adv = g.advection(&d, &c).unwrap();
    let truth = smooth_plus_noise(&d, &g, 70_000.0);
    let b = apply_full(&g, &adv, 0.0, &truth);
    let sol = g.solve_advective(0.0, &adv, &b, &cfg).unwrap();
    let err = truth.iter().zip(&sol.psi).fold(0.0f64, |m, (a, c)| m.max((a - c).abs()));
    println!("plane Pe~5: max err {err:.2e} in {} iterations, restarts {}", sol.total_iterations, sol.components[0].restarts);
    assert!(err < 1e-8, "plane error {err}");
    // (b) sphere gyre world, beta advection with r = 1e-5
    for level in [3u32, 4] {
        let w = gyre_world(level);
        let g = OceanGraph::new(&w.d, &w.ocean, &w.comp).unwrap();
        let r = 1.0e-5;
        let c: Vec<Vec3> = (0..w.d.n_cells()).map(|i| if w.ocean[i] { scale(w.d.frames[i].east, -w.beta[i] / r) } else { [0.0; 3] }).collect();
        let adv = g.advection(&w.d, &c).unwrap();
        assert!(adv.max_abs_flux() > 0.0);
        let truth: Vec<f64> = g.cells.iter().map(|&i| (3.0 * lat_of(&w.d, i as usize)).sin() * (w.d.mesh.up[i as usize][0]).cos() + 0.1 * w.d.mesh.up[i as usize][1]).collect();
        let b = apply_full(&g, &adv, 0.0, &truth);
        let sol = g.solve_advective(0.0, &adv, &b, &cfg).unwrap();
        let err = truth.iter().zip(&sol.psi).fold(0.0f64, |m, (a, c)| m.max((a - c).abs()));
        println!("sphere L{level}: max err {err:.2e}, iterations {}, restarts {}", sol.total_iterations, sol.components[0].restarts);
        assert!(err < 1e-7, "L{level}: {err}");
    }
}

#[test]
fn advection_edge_fluxes_are_antisymmetric_and_exact_on_linear_fields() {
    // Antisymmetry on a sphere with a smooth swirling field.
    let w = gyre_world(3);
    let g = OceanGraph::new(&w.d, &w.ocean, &w.comp).unwrap();
    let c: Vec<Vec3> = (0..w.d.n_cells()).map(|i| scale(w.d.frames[i].east, 1.0 + w.d.mesh.up[i][2])).collect();
    let adv = g.advection(&w.d, &c).unwrap();
    let mut checked = 0;
    for i in 0..g.n {
        for q in g.offsets[i] as usize..g.offsets[i + 1] as usize {
            let j = g.adj[q] as usize;
            let back = (g.offsets[j] as usize..g.offsets[j + 1] as usize).find(|&p| g.adj[p] as usize == i).expect("symmetric adjacency");
            assert_eq!(adv.flux[q], -adv.flux[back], "edge {i}-{j}");
            checked += 1;
        }
    }
    assert!(checked > 1000);
    // Uniform field c on an all-ocean walled plane: N psi = c . grad psi exactly for a linear psi at interior cells (upwind is exact on lines).
    let d = plane(30, 20);
    let n = d.n_cells();
    let g = OceanGraph::new(&d, &vec![true; n], &vec![0; n]).unwrap();
    let c = vec![[-3.0e-4, 1.0e-4, 0.0]; n];
    let adv = g.advection(&d, &c).unwrap();
    let (a, b) = (2.0e-6, -5.0e-7); // psi = a x + b y
    let psi: Vec<f64> = (0..n).map(|i| a * d.mesh.centroid[i][0] + b * d.mesh.centroid[i][1]).collect();
    let mut npsi = vec![0.0; n];
    for i in 0..g.n {
        let mut s = 0.0;
        for q in g.offsets[i] as usize..g.offsets[i + 1] as usize {
            let v = adv.flux[q];
            if v < 0.0 {
                s += v * (psi[g.adj[q] as usize] - psi[i]);
            }
        }
        npsi[i] = s / g.area[i];
    }
    let want = -3.0e-4 * a + 1.0e-4 * b;
    let mut interior = 0;
    for i in 0..n {
        let (x, y) = (i % 30, i / 30);
        if x > 0 && x < 29 && y > 0 && y < 19 {
            assert!((npsi[i] - want).abs() < 1e-12 * want.abs() + 1e-18, "cell {i}: {} vs {want}", npsi[i]);
            interior += 1;
        }
    }
    assert_eq!(interior, 28 * 18);
    // Constants are annihilated.
    let ones = vec![1.0; g.n];
    let nz = apply_full(&g, &adv, 0.0, &ones);
    assert!(nz.iter().zip(0..).filter(|(_, i)| { let (x, y) = (i % 30, i / 30); x > 0 && x < 29 && y > 0 && y < 19 }).all(|(v, _)| v.abs() < 1e-18));
}

#[test]
fn one_dimensional_stommel_layer_matches_the_exact_discrete_solution_and_sits_in_the_west() {
    // Strip: land at x = 0 and x = nx-1, ocean between, 3 rows (psi uniform in y; y walls are no-flux). 1-D discrete problem
    //   (-L + N) psi = b with upwind N for c = -b0 e_x, solved exactly with the Thomas algorithm and compared with BiCGSTAB.
    let (nx, ny) = (60usize, 3usize);
    let d = plane(nx, ny);
    let land: Vec<bool> = (0..nx * ny).map(|i| i % nx == 0 || i % nx == nx - 1).collect();
    let (ocean, comp) = masks(&d, &land);
    let g = OceanGraph::new(&d, &ocean, &comp).unwrap();
    let dx = 10_000.0;
    let b0 = 3.0 / (nx as f64 * dx) * 20.0; // layer thickness 1/b0 = nx dx / 60 = dx: right at the resolution limit
    let c = vec![[-b0, 0.0, 0.0]; d.n_cells()];
    let adv = g.advection(&d, &c).unwrap();
    // rhs: uniform source in x, so continuous solution has a Sverdrup interior and a west layer
    let rhs = vec![-1.0e-9; g.n];
    let sol = g.solve_advective(0.0, &adv, &rhs, &CgConfig::default()).unwrap();
    // Thomas on the 1-D chain of interior columns x = 1..nx-2 (the equation per cell is the same on every row).
    let m = nx - 2;
    let (mut lo, mut di, mut up_, mut r) = (vec![0.0; m], vec![0.0; m], vec![0.0; m], vec![rhs[0]; m]);
    let dif = 1.0 / (dx * dx); // (psi_{i-1} - 2 psi_i + psi_{i+1}) / dx^2 with -L: +2 psi - neighbours
    let adv_k = b0 / dx; // upwind with c = -b0: inflow from the east: N psi = -b0 (psi_{i+1} - psi_i)/dx
    for i in 0..m {
        di[i] = 2.0 * dif + adv_k; // west land cell has psi = 0 (Dirichlet) and east land too
        lo[i] = -dif;
        up_[i] = -dif - adv_k;
    }
    // forward elimination
    for i in 1..m {
        let w = lo[i] / di[i - 1];
        di[i] -= w * up_[i - 1];
        r[i] -= w * r[i - 1];
    }
    let mut x = vec![0.0; m];
    x[m - 1] = r[m - 1] / di[m - 1];
    for i in (0..m - 1).rev() {
        x[i] = (r[i] - up_[i] * x[i + 1]) / di[i];
    }
    let mut err = 0.0f64;
    for (l, &cg) in g.cells.iter().enumerate() {
        let col = cg as usize % nx;
        err = err.max((sol.psi[l] - x[col - 1]).abs());
    }
    let sc = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    println!("1-D Stommel: max |BiCGSTAB - Thomas| = {err:.2e} (psi scale {sc:.2e})");
    assert!(err < 1e-8 * sc, "{err} vs {sc}");
    // Western intensification: the steepest psi gradient is in the west half, and much steeper than in the east.
    let grad = |i: usize| (x[i + 1] - x[i]).abs();
    let west = (0..10).map(grad).fold(0.0f64, f64::max);
    let east = (m - 11..m - 1).map(grad).fold(0.0f64, f64::max);
    println!("1-D Stommel: max |dpsi| west 10 cols {west:.3e}, east 10 cols {east:.3e}");
    assert!(west > 3.0 * east);
    // And reversing the advection puts the layer in the east.
    let c2 = vec![[b0, 0.0, 0.0]; d.n_cells()];
    let adv2 = g.advection(&d, &c2).unwrap();
    let sol2 = g.solve_advective(0.0, &adv2, &rhs, &CgConfig::default()).unwrap();
    for (l, &cg) in g.cells.iter().enumerate() {
        let col = cg as usize % nx;
        assert!((sol2.psi[l] - x[m - col]).abs() < 1e-8 * sc, "mirror symmetry col {col}"); // exact mirror image
    }
}

#[test]
fn solver_errors_and_no_convergence() {
    let (d, ocean, comp) = land_border_plane(40, 30);
    let g = OceanGraph::new(&d, &ocean, &comp).unwrap();
    let c = vec![[-5.0e-4, 0.0, 0.0]; d.n_cells()];
    let adv = g.advection(&d, &c).unwrap();
    let b = smooth_plus_noise(&d, &g, 80_000.0);
    let cfg = CgConfig::default();
    assert!(matches!(g.solve_advective(-1.0, &adv, &b, &cfg), Err(Error::InvalidParam(_))));
    assert!(matches!(g.solve_advective(0.0, &adv, &b, &CgConfig { tol: 0.0, max_iters: 10 }), Err(Error::InvalidParam(_))));
    assert!(matches!(g.solve_advective(0.0, &adv, &b[1..], &cfg), Err(Error::InvalidParam(_))));
    let mut bad = b.clone();
    bad[5] = f64::NAN;
    assert!(matches!(g.solve_advective(0.0, &adv, &bad, &cfg), Err(Error::NonFinite(_))));
    // advection belonging to another graph
    let (d2, o2, c2) = land_border_plane(20, 20);
    let g2 = OceanGraph::new(&d2, &o2, &c2).unwrap();
    assert!(matches!(g2.solve_advective(0.0, &adv, &vec![0.0; g2.n], &cfg), Err(Error::InvalidParam(_))));
    assert!(matches!(g.advection(&d, &c[1..]), Err(Error::InvalidParam(_))));
    // max_iters
    match g.solve_advective(0.0, &adv, &b, &CgConfig { tol: 1e-10, max_iters: 3 }) {
        Err(Error::NoConvergence { solver, iterations, residual }) => {
            assert!(solver.contains("bicgstab"), "{solver}");
            assert_eq!(iterations, 3);
            assert!(residual > 1e-10);
        }
        other => panic!("expected NoConvergence, got {other:?}"),
    }
    // zero rhs: zero solution in zero iterations; deterministic otherwise
    let z = g.solve_advective(0.0, &adv, &vec![0.0; g.n], &cfg).unwrap();
    assert!(z.psi.iter().all(|&v| v == 0.0) && z.total_iterations == 0);
    let s1 = g.solve_advective(0.0, &adv, &b, &cfg).unwrap();
    let s2 = g.solve_advective(0.0, &adv, &b, &cfg).unwrap();
    assert_eq!(s1, s2);
}

// ---------------------------------------------------------------------------------------------
// Part (ii): gyres with beta
// ---------------------------------------------------------------------------------------------

#[test]
fn beta_off_reproduces_4a2c_exactly() {
    let w = gyre_world(4);
    let n = w.d.n_cells();
    // Reference: the 4a-2c pipeline written out from the public pieces (CG, gradient, slip, clamp). A garbage `beta` must be ignored.
    let (h, r, rho, vmax) = (100.0, 5.0e-8, 1025.0, 3.0);
    let g = OceanGraph::new(&w.d, &w.ocean, &w.comp).unwrap();
    let denom = rho * h * r;
    let b: Vec<f64> = g.cells.iter().map(|&c| -w.curl[c as usize] / denom).collect();
    let psi = g.to_global(&g.solve(0.0, &b, &CgConfig::default()).unwrap().psi);
    let mut u = current_from_psi(&w.d, &enbuilder::gradient::GradientOp::new(&w.d), &psi, &w.ocean);
    apply_coast_slip(&w.d, &w.ocean, &mut u);
    clamp_speed(&mut u, vmax);
    let garbage: Vec<f64> = (0..n).map(|i| 1.0e3 * (i as f64).sin()).collect();
    let (m, f) = run(&w, &garbage, &[("beta_on", 0.0), ("r_friction", r)]);
    assert_eq!(f.data("psi").unwrap(), &psi[..], "psi must be bit-identical");
    let got = unpack(&f);
    for i in 0..n {
        assert_eq!(got[i], u[i], "current cell {i}");
    }
    assert_eq!(m.params().get("solver_kind"), Some(0.0));
    assert!(m.warning().is_none());
    assert_eq!(max_abs(f.data("boundary_layer_m").unwrap()), 0.0);
}

#[test]
fn zero_beta_with_beta_on_is_bit_identical_to_beta_off() {
    let w = gyre_world(4);
    let n = w.d.n_cells();
    let (_, f0) = run(&w, &vec![0.0; n], &[("beta_on", 0.0)]);
    let (m1, f1) = run(&w, &vec![0.0; n], &[("beta_on", 1.0)]);
    assert_eq!(f0.data("psi").unwrap(), f1.data("psi").unwrap());
    assert_eq!(f0.data("current").unwrap(), f1.data("current").unwrap());
    assert_eq!(m1.params().get("solver_kind"), Some(0.0), "no advection -> the CG path");
}

#[test]
fn western_flank_is_faster_than_eastern_flank_in_both_hemispheres() {
    for level in [3u32, 4, 5] {
        let w = gyre_world(level);
        let (m, f) = run(&w, &w.beta, &[("max_current", BIG)]);
        assert_eq!(m.params().get("solver_kind"), Some(1.0));
        let speed = f.data("current_speed").unwrap().to_vec();
        let peaks = flank_peaks(&w, &speed);
        let mut line = format!("L{level}:");
        for (name, west, east) in &peaks {
            line += &format!(" {name} W {west:.3} E {east:.3} ({:.1}x);", west / east);
            assert!(*west > 2.0 * east, "L{level} {name}: west peak {west} vs east peak {east}");
        }
        println!("{line}");
        // Beta off is left-right symmetric (same measurement), so the asymmetry above is the beta term.
        let (_, f0) = run(&w, &w.beta, &[("max_current", BIG), ("beta_on", 0.0)]);
        for (name, west, east) in flank_peaks(&w, f0.data("current_speed").unwrap()) {
            assert!((west / east - 1.0).abs() < 1e-6, "L{level} {name} beta off: {west} vs {east}");
        }
    }
}

#[test]
fn reversed_rotation_puts_the_intensification_on_the_east_coast() {
    let w = gyre_world(4);
    let neg: Vec<f64> = w.beta.iter().map(|b| -b).collect();
    let (_, f) = run(&w, &neg, &[("max_current", BIG)]);
    for (name, west, east) in flank_peaks(&w, f.data("current_speed").unwrap()) {
        println!("beta < 0 {name}: W {west:.3} E {east:.3}");
        assert!(east > 2.0 * west, "{name}: east peak {east} vs west peak {west}");
    }
}

#[test]
fn subtropical_gyres_still_rotate_in_opposite_senses() {
    let w = gyre_world(4);
    let (_, f) = run(&w, &w.beta, &[("max_current", BIG)]);
    let psi = f.data("psi").unwrap().to_vec();
    let deep = deep_mask(&w.d, &w.ocean);
    let (pn, ps) = (band_mean(&w.d, &deep, &psi, 10.0, 40.0), band_mean(&w.d, &deep, &psi, -40.0, -10.0));
    println!("mean psi N {pn:.4e}, S {ps:.4e}");
    // Northern subtropics: clockwise (positive-psi hill), southern: counter-clockwise (negative), mirror images.
    assert!(pn > 0.0 && ps < 0.0);
    assert!((pn / ps + 1.0).abs() < 0.05, "N/S not antisymmetric: {}", pn / ps);
    // Same sign structure as the beta-off gyres (beta moves the flow, it does not reverse it).
    let (_, f0) = run(&w, &w.beta, &[("max_current", BIG), ("beta_on", 0.0)]);
    let psi0 = f0.data("psi").unwrap();
    let corr: f64 = (0..psi.len()).map(|i| psi[i] * psi0[i]).sum::<f64>() / (psi.iter().map(|v| v * v).sum::<f64>().sqrt() * psi0.iter().map(|v| v * v).sum::<f64>().sqrt());
    println!("corr(psi beta on, psi beta off) = {corr:.3}");
    assert!(corr > 0.7);
    // The Sverdrup interior flows equatorward in the subtropics of both hemispheres (v = curl/(rho H beta)).
    let u = unpack(&f);
    let v_north: Vec<f64> = (0..w.d.n_cells()).map(|i| dot(u[i], w.d.frames[i].north)).collect();
    assert!(band_mean(&w.d, &deep, &v_north, 15.0, 35.0) < 0.0, "NH interior meridional flow must be equatorward");
    assert!(band_mean(&w.d, &deep, &v_north, -35.0, -15.0) > 0.0, "SH interior meridional flow must be equatorward");
}

#[test]
fn solver_converges_on_sphere_levels_3_to_5_and_cube() {
    let cfg = CgConfig::default();
    for level in [3u32, 4, 5] {
        let w = gyre_world(level);
        let (m, f) = run(&w, &w.beta, &[]);
        let it = m.params().get("cg_iterations").unwrap();
        println!("sphere L{level}: {it} BiCGSTAB iterations, restarts {}", m.params().get("solver_restarts").unwrap());
        assert!(it > 0.0 && (it as usize) < cfg.max_iters / 10, "L{level}: {it} iterations");
        assert!(f.data("psi").unwrap().iter().chain(f.data("current").unwrap()).all(|v| v.is_finite()), "L{level} NaN");
    }
    // Cube 14/edge: one land face (the +x face), zonal-like wind from the rotation axis.
    let d = cube();
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| d.mesh.up[i][0] > 0.6).collect();
    let (ocean, comp) = masks(&d, &land);
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat_of(&d, i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let beta = earth_beta(&d);
    let w = World { d, ocean, comp, curl, beta };
    let (m, f) = run(&w, &w.beta, &[]);
    let it = m.params().get("cg_iterations").unwrap();
    println!("cube 14/edge: {it} iterations, kind {}", m.params().get("solver_kind").unwrap());
    assert!(it > 0.0 && (it as usize) < cfg.max_iters / 10);
    assert_eq!(m.params().get("solver_kind"), Some(1.0));
    assert!(f.data("psi").unwrap().iter().chain(f.data("current").unwrap()).all(|v| v.is_finite()));
    for i in 0..n {
        if !w.ocean[i] {
            assert_eq!(f.data("psi").unwrap()[i], 0.0);
        }
    }
    assert!(max_abs(f.data("psi").unwrap()) > 0.0);
}

#[test]
fn flat_plane_without_beta_has_symmetric_gyres_and_a_beta_plane_does_not() {
    let (nx, ny) = (40usize, 30usize);
    let (d, ocean, comp) = land_border_plane(nx, ny);
    let n = d.n_cells();
    assert!(dot(d.frames[0].east, [1.0, 0.0, 0.0]) > 0.99, "plane frames: east = +x assumed by this test");
    let ly = ny as f64 * 10_000.0;
    let tau: Vec<Vec3> = (0..n).map(|i| [-0.1 * (2.0 * std::f64::consts::PI * d.mesh.centroid[i][1] / ly).cos(), 0.0, 0.0]).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let w = World { d, ocean, comp, curl, beta: vec![0.0; n] };
    // f-plane: beta = 0 -> mirror symmetric in x
    let (m0, f0) = run(&w, &w.beta, &[("max_current", BIG)]);
    assert_eq!(m0.params().get("solver_kind"), Some(0.0));
    let psi0 = f0.data("psi").unwrap().to_vec();
    let sc = max_abs(&psi0);
    assert!(sc > 0.0);
    let asym = (0..n).map(|i| { let (x, y) = (i % nx, i / nx); (psi0[i] - psi0[y * nx + (nx - 1 - x)]).abs() }).fold(0.0f64, f64::max) / sc;
    println!("f-plane x-asymmetry of psi: {asym:.2e}");
    assert!(asym < 1e-9, "{asym}");
    // beta-plane (beta = 2.3e-11 everywhere, east = +x): west-intensified
    let beta = vec![2.3e-11; n];
    let (m1, f1) = run(&w, &beta, &[("max_current", BIG), ("r_friction", 7.0e-7)]); // layer r/beta = 30 km = 3 cells in a 400 km basin
    assert_eq!(m1.params().get("solver_kind"), Some(1.0));
    let sp = f1.data("current_speed").unwrap();
    let col_peak = |x0: usize, x1: usize| (0..n).filter(|&i| i % nx >= x0 && i % nx < x1).map(|i| sp[i]).fold(0.0f64, f64::max);
    let (west, east) = (col_peak(1, 7), col_peak(nx - 7, nx - 1));
    println!("beta-plane: peak speed west 6 columns {west:.3e}, east 6 columns {east:.3e}");
    assert!(west > 2.0 * east);
    // And the f-plane run's speeds are mirror symmetric, west == east.
    let sp0 = f0.data("current_speed").unwrap();
    let (w0, e0) = ((0..n).filter(|&i| i % nx >= 1 && i % nx < 7).map(|i| sp0[i]).fold(0.0f64, f64::max), (0..n).filter(|&i| i % nx >= nx - 7 && i % nx < nx - 1).map(|i| sp0[i]).fold(0.0f64, f64::max));
    assert!((w0 / e0 - 1.0).abs() < 1e-6);
}

/// Least-squares divergence consistent with `GradientOp`: `sum_k coef_k . (u_j - u_i)`.
fn div_ls(d: &Domain, coef: &[Vec3], u: &[Vec3], i: usize) -> f64 {
    let m = &d.mesh;
    let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
    (a..b).map(|k| dot(coef[k], sub(u[m.adj[k] as usize], u[i]))).sum()
}

#[test]
fn no_nan_and_the_current_stays_divergence_free_with_beta() {
    // Plane, beta-plane: central-difference commutation makes the current non-divergent to roundoff, whatever psi is.
    let (nx, ny) = (40usize, 30usize);
    let (d, ocean, comp) = land_border_plane(nx, ny);
    let n = d.n_cells();
    let ly = ny as f64 * 10_000.0;
    let tau: Vec<Vec3> = (0..n).map(|i| [-0.1 * (2.0 * std::f64::consts::PI * d.mesh.centroid[i][1] / ly).cos(), 0.0, 0.0]).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let w = World { d, ocean, comp, curl, beta: vec![2.3e-11; n] };
    let (_, f) = run(&w, &w.beta, &[("max_current", BIG), ("r_friction", 4.0e-6)]);
    let u = unpack(&f);
    let g = enbuilder::gradient::GradientOp::new(&w.d);
    let coef = g.coef();
    let umax = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    let deep = deep_mask(&w.d, &w.ocean);
    let (mut worst, mut cnt) = (0.0f64, 0);
    for i in 0..n {
        if deep[i] {
            worst = worst.max(div_ls(&w.d, coef, &u, i).abs());
            cnt += 1;
        }
    }
    let rel = worst / (umax / 10_000.0);
    println!("beta-plane: max |div| / (umax/dx) = {rel:.2e} over {cnt} deep cells");
    assert!(cnt > 500 && rel < 1e-9, "{rel}");
    // Sphere L4 with beta: same measure as 4a-2c, relative to the largest vorticity scale of the current.
    let w = gyre_world(4);
    let (_, f) = run(&w, &w.beta, &[("max_current", BIG)]);
    let u = unpack(&f);
    assert!(f.data("current").unwrap().iter().chain(f.data("psi").unwrap()).chain(f.data("current_speed").unwrap()).all(|v| v.is_finite()));
    let g = enbuilder::gradient::GradientOp::new(&w.d);
    let deep = deep_mask(&w.d, &w.ocean);
    let zeta = curl_of_masked(&w.d, &u, &w.ocean);
    let (mut worst_div, mut worst_zeta) = (0.0f64, 0.0f64);
    for i in 0..w.d.n_cells() {
        if deep[i] {
            worst_div = worst_div.max(div_ls(&w.d, g.coef(), &u, i).abs());
            worst_zeta = worst_zeta.max(zeta[i].abs());
        }
    }
    println!("sphere L4 beta on: max |div| / max |zeta| (deep cells) = {:.3}", worst_div / worst_zeta);
    assert!(worst_div / worst_zeta < 0.1);
}

#[test]
fn boundary_layer_report_and_resolution_warning() {
    let w = gyre_world(4);
    let n = w.d.n_cells();
    // Exact relation r = width * |beta| and the ratio to the cell size.
    let rep = boundary_layer_report(&w.d, &w.beta, &w.ocean, 1.0e-5);
    for i in 0..n {
        if w.ocean[i] && w.beta[i].abs() > 1e-30 {
            assert!((rep.width_m[i] * w.beta[i].abs() - 1.0e-5).abs() < 1e-18);
        } else {
            assert_eq!(rep.width_m[i], 0.0);
        }
    }
    // Earth equator: r/beta = 1e-5 / 2.28e-11 = 438 km
    let eq = (0..n).filter(|&i| w.ocean[i] && lat_of(&w.d, i).abs() < 0.05).map(|i| rep.width_m[i]).fold(f64::MAX, f64::min);
    println!("equatorial layer width {:.0} km", eq / 1e3);
    assert!((eq / 1e3 - 438.0).abs() < 40.0);
    // Refining the mesh by one level halves the cell size, so the width in cells doubles.
    let r4 = boundary_layer_report(&w.d, &w.beta, &w.ocean, 1.0e-5);
    let w5 = gyre_world(5);
    let r5 = boundary_layer_report(&w5.d, &w5.beta, &w5.ocean, 1.0e-5);
    println!("median layer in cells: L4 {:.2}, L5 {:.2}", r4.cells_median, r5.cells_median);
    assert!((r5.cells_median / r4.cells_median - 2.0).abs() < 0.25);
    // Default friction: L3 is under-resolved (warn), L5 is not. The module reports it and exposes the text.
    let (m3, _) = run(&gyre_world(3), &gyre_world(3).beta, &[]);
    assert_eq!(m3.params().get("under_resolved"), Some(1.0));
    let text = m3.warning().expect("L3 default friction must warn");
    println!("{text}");
    assert!(text.contains("thinner than one mesh cell"));
    let (m5, _) = run(&w5, &w5.beta, &[]);
    assert_eq!(m5.params().get("under_resolved"), Some(0.0));
    assert!(m5.warning().is_none());
    assert!(m5.params().get("layer_cells_median").unwrap() > 1.5);
    // A thicker layer from larger friction clears the L3 warning; a thinner one raises it at L5.
    let (m3b, _) = run(&gyre_world(3), &gyre_world(3).beta, &[("r_friction", 4.0e-5)]);
    assert!(m3b.warning().is_none());
    let (m5b, _) = run(&w5, &w5.beta, &[("r_friction", 2.0e-6)]);
    assert!(m5b.warning().is_some());
    // The field is written and equals the pure report.
    let (_, f4) = run(&w, &w.beta, &[]);
    assert_eq!(f4.data("boundary_layer_m").unwrap(), &rep.width_m[..]);
}

#[test]
fn wiring_through_the_scheduler_with_the_real_forcing_stages() {
    let d = sphere(3);
    let n = d.n_cells();
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |name: &str| order.iter().position(|x| x == name).unwrap_or_else(|| panic!("{name} missing from {order:?}"));
    assert!(pos("beta_term") < pos("ocean_currents") && pos("wind_stress_curl") < pos("ocean_currents"), "{order:?}");
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, (0..n).map(|i| if d.mesh.centroid[i][1].abs() < 0.13 * R && lat_of(&d, i).abs() < 70f64.to_radians() { 300.0 } else { -3000.0 }).collect()).unwrap();
    // A zonal wind pattern: trades / westerlies.
    f.put("wind", 3, (0..n).flat_map(|i| scale(d.frames[i].east, -8.0 * (3.0 * lat_of(&d, i)).cos())).collect()).unwrap();
    f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
    f.put("coriolis_f", 1, (0..n).map(|i| 2.0 * OMEGA * d.mesh.up[i][2]).collect()).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert_eq!(s.module_param("ocean_currents", "solver_kind"), Some(1.0));
    assert!(max_abs(f.data("psi").unwrap()) > 0.0);
    assert!(f.has("boundary_layer_m") && f.has("beta"));
    // Spec, defaults, parameter rules.
    let m = OceanCurrents::new();
    let p = m.params();
    assert_eq!((p.get("r_friction"), p.get("beta_on"), p.get("max_current")), (Some(DEFAULT_R_FRICTION), Some(1.0), Some(3.0)));
    assert_eq!(DEFAULT_R_FRICTION, 1.0e-5);
    let mut m = OceanCurrents::new();
    assert!(m.set_param("beta_on", 2.0).is_err() && m.set_param("beta_on", 0.0).is_ok() && m.set_param("beta_on", 1.0).is_ok());
    for ro in ["solver_kind", "solver_restarts", "layer_cells_min", "layer_cells_median", "under_resolved_fraction", "under_resolved"] {
        assert!(m.set_param(ro, 1.0).is_err(), "{ro} must be output-only");
    }
    // beta_on = 1 with no `beta` field is an error (beta_on = 0 does not need it).
    let w = gyre_world(3);
    let mut g = fields(&w, &w.beta);
    let mut g2 = FieldStore::new(n);
    g2.put("ocean_mask", 1, g.data("ocean_mask").unwrap().to_vec()).unwrap();
    g2.put("ocean_component", 1, g.data("ocean_component").unwrap().to_vec()).unwrap();
    g2.put("stress_curl", 1, w.curl.clone()).unwrap();
    assert!(OceanCurrents::new().run(&w.d, &mut g2, &DirtyRegion::All).is_err());
    let mut off = OceanCurrents::new();
    off.set_param("beta_on", 0.0).unwrap();
    off.run(&w.d, &mut g2, &DirtyRegion::All).unwrap();
    // NaN beta
    let mut bad = w.beta.clone();
    bad[7] = f64::NAN;
    let mut g3 = FieldStore::new(n);
    g3.put("ocean_mask", 1, g.data("ocean_mask").unwrap().to_vec()).unwrap();
    g3.put("ocean_component", 1, g.data("ocean_component").unwrap().to_vec()).unwrap();
    g3.put("stress_curl", 1, w.curl.clone()).unwrap();
    g3.put("beta", 1, bad).unwrap();
    assert!(matches!(OceanCurrents::new().run(&w.d, &mut g3, &DirtyRegion::All), Err(Error::NonFinite(_))));
    let _ = &mut g;
}

#[test]
fn all_ocean_sphere_with_beta_is_finite_and_deterministic() {
    // No coast anchor: the pinned-cell path of the singular case.
    let d = sphere(3);
    let n = d.n_cells();
    let ocean = vec![true; n];
    let comp = vec![0i64; n];
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat_of(&d, i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let beta = earth_beta(&d);
    let w = World { d, ocean, comp, curl, beta };
    let (m, f) = run(&w, &w.beta, &[]);
    let (_, f2) = run(&w, &w.beta, &[]);
    assert_eq!(f.data("psi").unwrap(), f2.data("psi").unwrap());
    assert!(f.data("psi").unwrap().iter().chain(f.data("current").unwrap()).all(|v| v.is_finite()));
    assert_eq!(m.params().get("solver_kind"), Some(1.0));
    // area-weighted mean zero
    let psi = f.data("psi").unwrap();
    let sa: f64 = w.d.mesh.area.iter().sum();
    let mean = (0..w.d.n_cells()).map(|i| psi[i] * w.d.mesh.area[i]).sum::<f64>() / sa;
    assert!(mean.abs() < 1e-9 * max_abs(psi), "mean {mean}");
}
