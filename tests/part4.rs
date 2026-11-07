//! Part 4a-2e: the ocean current stage on every supported world shape, tuned defaults, waterworld and wall rules, seam checks.
//! NOT COMPILED when written (both sessions that produced it had no Rust toolchain and no network). Every threshold comes from the numpy
//! mirror `reference/mirror_4a2e.py` (it reproduces the recorded 4a-2d numbers: Earth-like L5 boundary peak 0.867 and interior median
//! 0.124 m/s, one-island L4 0.443, plane west/east peaks 0.308/0.179, cube 964 ocean cells and max speed 0.893) with at least 1.3x
//! margin; the mirror value is quoted next to each assertion. Expect small compile slips on the first build.
//!
//! Worlds: (1) Earth-like sphere, (2) waterworld, (3) enclosed ocean on a plane, wrapped and walled (+ seam shift + wall pin),
//! (4) cube world with oceans spanning edges, (5) tidally locked sphere (non-zonal stress).
use enbuilder::generators::*;
use enbuilder::math::{add, cross, dot, length, normalize, scale, sub, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_currents::*;
use enbuilder::ocean_forcing::*;
use enbuilder::ocean_validation::*;
use enbuilder::*;

const R: f64 = 6.371e6;
const OMEGA: f64 = 7.292e-5;
const BIG: f64 = 1.0e9;
const RHO: f64 = 1025.0;

// ------------------------------------------------------------------------------------------------ helpers
fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane(nx: usize, ny: usize, boundary: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn lat_of(d: &Domain, i: usize) -> f64 { d.mesh.up[i][2].asin() }

fn masks(d: &Domain, land: &[bool]) -> (Vec<bool>, Vec<i64>) {
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -1000.0 }).collect();
    ocean_cells(d, &z, &vec![0.0; n], 1.0)
}

fn earth_beta(d: &Domain, omega: f64) -> Vec<f64> {
    let f: Vec<f64> = (0..d.n_cells()).map(|i| 2.0 * omega * d.mesh.up[i][2]).collect();
    beta_of(d, &f)
}

/// Eastward zonal stress `-tau0 cos(3 lat)` (trades at the equator, westerlies at 60 degrees).
fn zonal_stress(d: &Domain, tau0: f64) -> Vec<Vec3> { (0..d.n_cells()).map(|i| scale(d.frames[i].east, -tau0 * (3.0 * lat_of(d, i)).cos())).collect() }

/// Everything one run needs.
struct Case { d: Domain, mask: Vec<bool>, comp: Vec<i64>, curl: Vec<f64>, beta: Vec<f64> }

impl Case {
    fn new(d: Domain, land: &[bool], tau: &[Vec3], beta: Vec<f64>) -> Case {
        let (mask, comp) = masks(&d, land);
        let curl = curl_of_masked(&d, tau, &mask);
        Case { d, mask, comp, curl, beta }
    }
    fn fields(&self) -> FieldStore {
        let n = self.d.n_cells();
        let mut f = FieldStore::new(n);
        f.put("ocean_mask", 1, self.mask.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
        f.put("ocean_component", 1, self.comp.iter().map(|&c| c as f64).collect()).unwrap();
        f.put("stress_curl", 1, self.curl.clone()).unwrap();
        f.put("beta", 1, self.beta.clone()).unwrap();
        f
    }
    /// Fresh module with the DEFAULT parameters plus the given overrides.
    fn run(&self, params: &[(&str, f64)]) -> (OceanCurrents, FieldStore) {
        let mut f = self.fields();
        let mut m = OceanCurrents::new();
        for &(k, v) in params {
            m.set_param(k, v).unwrap();
        }
        m.run(&self.d, &mut f, &DirtyRegion::All).unwrap();
        (m, f)
    }
    fn eff(&self) -> EffectiveOcean { effective_ocean(&self.d, &self.mask, &self.comp).unwrap() }
}

fn unpack(f: &FieldStore) -> Vec<Vec3> { unpack_vec3(f.data("current").unwrap()) }
fn max_abs(a: &[f64]) -> f64 { a.iter().fold(0.0f64, |m, v| m.max(v.abs())) }
fn param(m: &OceanCurrents, k: &str) -> f64 { m.params().get(k).unwrap() }
fn area_fraction(d: &Domain, mask: &[bool]) -> f64 { (0..d.n_cells()).filter(|&i| mask[i]).map(|i| d.mesh.area[i]).sum::<f64>() / d.mesh.total_area() }

/// Four land caps (axis, half angle in degrees), about 32% of the sphere: three continents and a polar cap.
const EARTH_CAPS: [([f64; 3], f64); 4] = [([1.0, 0.2, 0.3], 42.0), ([-0.8, 0.5, -0.1], 36.0), ([0.1, -0.9, -0.4], 30.0), ([0.0, 0.0, -1.0], 20.0)];

fn earth_land(d: &Domain) -> Vec<bool> {
    (0..d.n_cells()).map(|i| EARTH_CAPS.iter().any(|(ax, th)| dot(d.mesh.up[i], normalize(*ax)) > th.to_radians().cos())).collect()
}

fn earth_case(level: u32) -> Case {
    let d = sphere(level);
    let land = earth_land(&d);
    let tau = zonal_stress(&d, 0.1);
    let beta = earth_beta(&d, OMEGA);
    Case::new(d, &land, &tau, beta)
}

/// Least-squares divergence consistent with `GradientOp`: `sum_k coef_k . (u_j - u_i)`.
fn div_ls(d: &Domain, coef: &[Vec3], u: &[Vec3], i: usize) -> f64 {
    let m = &d.mesh;
    let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
    (a..b).map(|k| dot(coef[k], sub(u[m.adj[k] as usize], u[i]))).sum()
}

/// Net coast flux per coast cell must be zero after the slip (the 4a-2c rule): `|F_i| <= 1e-9 L_i umax`.
fn assert_no_net_coast_flux(d: &Domain, ocean: &[bool], u: &[Vec3]) {
    let cv = coast_vectors(d, ocean);
    let flux = coast_outflux(d, ocean, u);
    let umax = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    let mut checked = 0;
    for i in 0..d.n_cells() {
        if cv[i].1 > 0.0 {
            checked += 1;
            assert!(flux[i].abs() <= 1e-9 * cv[i].1 * umax + 1e-300, "net coast flux at cell {i}: {}", flux[i]);
        }
    }
    assert!(checked > 0, "no coast cells to check");
}

// ------------------------------------------------------------------------------------------------ 1. Earth-like sphere
#[test]
fn earth_like_sphere_has_plausible_peak_speeds_at_the_default_parameters() {
    // Mirror (L5, defaults H 20, r 1e-5, clamp 3): ocean fraction 0.678, boundary peak 0.867, interior median 0.125, clamped 0.
    // Layout B (other caps) gave 1.05 / 0.126 at L5, 0.68 / 0.123 at L4, so the tuning is not specific to this layout.
    let c = earth_case(5);
    let frac = area_fraction(&c.d, &c.mask);
    assert!(frac > 0.6 && frac < 0.8, "ocean fraction {frac}");
    let (m, f) = c.run(&[]);
    assert_eq!(param(&m, "clamped_cells"), 0.0, "the 3 m/s clamp must stay inactive on the Earth-like world");
    assert_eq!(param(&m, "unanchored_cells"), 0.0);
    assert_eq!(param(&m, "solver_kind"), 1.0);
    let eff = c.eff();
    let rep = speed_report(&c.d, &eff.ocean, f.data("current_speed").unwrap(), 10.0, 60.0);
    println!("Earth-like L5: boundary peak {:.3} m/s (n {}), interior median {:.3} m/s (n {})", rep.boundary_peak, rep.n_boundary, rep.interior_median, rep.n_interior);
    assert!(rep.n_boundary > 100 && rep.n_interior > 1000);
    assert!(rep.boundary_peak >= 0.5 && rep.boundary_peak <= 2.0, "boundary peak {}", rep.boundary_peak); // mirror 0.867
    assert!(rep.interior_median >= 0.05 && rep.interior_median <= 0.2, "interior median {}", rep.interior_median); // mirror 0.125
    assert!(f.data("current_speed").unwrap().iter().all(|v| v.is_finite()));
}

#[test]
fn earth_like_sphere_has_gyres_of_the_right_sense_and_western_intensification() {
    // Mirror L5: mean psi 10..40N +3.0e5, 10..40S -2.6e5 (deep cells); west-coast mean speed 0.429 vs east-coast 0.094 (ratio 4.59; L4 3.23).
    let c = earth_case(5);
    let (_, f) = c.run(&[]);
    let eff = c.eff();
    let d = &c.d;
    let (psi, sp) = (f.data("psi").unwrap(), f.data("current_speed").unwrap());
    let hops = coast_hops(d, &eff.ocean);
    for (lo, hi, sign) in [(10.0, 40.0, 1.0), (-40.0, -10.0, -1.0)] {
        let (mut s, mut cnt) = (0.0, 0usize);
        for i in 0..d.n_cells() {
            let l = lat_of(d, i).to_degrees();
            if eff.ocean[i] && hops[i] >= 3 && hops[i] != u32::MAX && l > lo && l < hi {
                s += psi[i];
                cnt += 1;
            }
        }
        assert!(cnt > 100);
        assert!(sign * s / cnt as f64 > 0.0, "band {lo}..{hi}: mean psi {}", s / cnt as f64);
    }
    // Coast cells with land to the west (western boundary) against land to the east, 10..50 degrees.
    let cv = coast_vectors(d, &eff.ocean);
    let (mut ws, mut wn, mut es, mut en) = (0.0, 0usize, 0.0, 0usize);
    for i in 0..d.n_cells() {
        let (s, l) = cv[i];
        let l_deg = lat_of(d, i).to_degrees().abs();
        if !(l > 0.0) || l_deg < 10.0 || l_deg > 50.0 {
            continue;
        }
        let ew = dot(s, d.frames[i].east);
        let sl = length(s);
        if ew < -0.5 * sl {
            ws += sp[i];
            wn += 1;
        } else if ew > 0.5 * sl {
            es += sp[i];
            en += 1;
        }
    }
    assert!(wn > 20 && en > 20, "west {wn} east {en}");
    let (wm, em) = (ws / wn as f64, es / en as f64);
    println!("Earth-like L5: western-boundary mean speed {wm:.3}, eastern {em:.3}, ratio {:.2}", wm / em);
    assert!(wm > 1.8 * em, "west {wm} east {em}"); // mirror 4.59, threshold 1.8
}

#[test]
fn earth_like_sphere_closes_the_mass_budget_of_every_basin() {
    // psi = 0 on every non-ocean cell, so the stream-function transport between ANY two coast points of a section (psi_B - psi_A)
    // is exactly 0: no net transport across any cross-section of a closed basin. The cell-centred current carries the 4a-2c coast
    // slip (net flux through land edges = 0 to roundoff, mirror 7e-17) but only a discretisation-level divergence (see the plane test).
    let c = earth_case(4);
    let (m, f) = c.run(&[]);
    assert_eq!(param(&m, "clamped_cells"), 0.0);
    let psi = f.data("psi").unwrap();
    for i in 0..c.d.n_cells() {
        if !c.mask[i] {
            assert_eq!(psi[i], 0.0, "psi on land cell {i}");
        }
    }
    assert_no_net_coast_flux(&c.d, &c.eff().ocean, &unpack(&f));
}

#[test]
fn icosahedron_pentagons_show_no_current_spike() {
    // Mirror L5 (detector: ocean, >= 2 hops from the coast, all neighbours ocean, neighbour mean >= 0.1 m/s): 5 of 12 pentagons evaluable,
    // worst spike 0.061 (limit 0.2). The same detector on ordinary hexagon cells reaches 0.160, so a pentagon is not worse than a normal
    // cell. (At L4 the detector is too noisy to use: one pentagon shows 0.23 but ordinary hexagons reach 0.77.)
    let c = earth_case(5);
    let (_, f) = c.run(&[]);
    let eff = c.eff();
    let pent = pentagon_cells(&c.d);
    assert_eq!(pent.len(), 12);
    let rep = seam_spikes(&c.d, &eff.ocean, f.data("current_speed").unwrap(), &pent, 0.2, 0.1);
    println!("pentagons: worst spike {:.3}, evaluable {}/{}", rep.worst, rep.evaluable, rep.candidates);
    assert!(rep.evaluable >= 3, "too few pentagons could be judged: {rep:?}");
    assert!(rep.worst <= 0.2, "{rep:?}");
}

#[test]
fn same_seed_gives_a_bit_identical_current() {
    let (a, b) = (earth_case(4), earth_case(4));
    let ((_, fa), (_, fb)) = (a.run(&[]), b.run(&[]));
    assert_eq!(fa.data("psi").unwrap(), fb.data("psi").unwrap());
    assert_eq!(fa.data("current").unwrap(), fb.data("current").unwrap());
}

// ------------------------------------------------------------------------------------------------ 2. waterworld
#[test]
fn waterworld_gets_zero_flow_and_a_note() {
    // Rule (4a-2e): a basin with no land-adjacent cell has no boundary condition (the problem is singular); it gets zero flow.
    let d = sphere(4);
    let n = d.n_cells();
    let tau = zonal_stress(&d, 0.1);
    let beta = earth_beta(&d, OMEGA);
    let c = Case::new(d, &vec![false; n], &tau, beta);
    assert!(c.mask.iter().all(|&o| o));
    let eff = c.eff();
    assert_eq!((eff.unanchored, eff.wall_pinned, eff.graph.n), (n, 0, 0));
    let (m, f) = c.run(&[]);
    assert_eq!(max_abs(f.data("psi").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current_speed").unwrap()), 0.0);
    assert_eq!(param(&m, "unanchored_cells"), n as f64);
    assert_eq!(param(&m, "cg_iterations"), 0.0);
    assert!(m.warning().unwrap().contains("no land coast"), "{:?}", m.warning());
    // Determinism
    let (_, f2) = c.run(&[]);
    assert_eq!(f.data("psi").unwrap(), f2.data("psi").unwrap());
}

#[test]
fn waterworld_through_the_scheduler_and_a_wrapped_plane_ocean_are_zero_too() {
    let d = sphere(3);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, vec![-3000.0; n]).unwrap();
    f.put("wind_stress", 3, (0..n).flat_map(|i| scale(d.frames[i].east, -0.05 * (3.0 * lat_of(&d, i)).cos())).collect()).unwrap();
    f.put("coriolis_f", 1, (0..n).map(|i| 2.0 * OMEGA * d.mesh.up[i][2]).collect()).unwrap();
    let mut s = Scheduler::new();
    add_ocean_mask_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("ocean_mask").unwrap().iter().all(|&m| m > 0.5));
    assert_eq!(max_abs(f.data("psi").unwrap()), 0.0);
    assert_eq!(max_abs(f.data("current").unwrap()), 0.0);
    assert_eq!(s.module_param("ocean_currents", "unanchored_cells"), Some(n as f64));

    // Wrapped plane that is all ocean: a torus has no shore either.
    let (nx, ny) = (40usize, 30usize);
    let p = plane(nx, ny, BoundaryKind::Wrap);
    let pn = p.n_cells();
    let ly = ny as f64 * 10_000.0;
    let tau: Vec<Vec3> = (0..pn).map(|i| [-0.005 * (2.0 * std::f64::consts::PI * p.mesh.centroid[i][1] / ly).cos(), 0.0, 0.0]).collect();
    let c = Case::new(p, &vec![false; pn], &tau, vec![2.3e-11; pn]);
    let (m, g) = c.run(&[("max_current", BIG)]);
    assert_eq!(max_abs(g.data("psi").unwrap()), 0.0);
    assert_eq!(param(&m, "unanchored_cells"), pn as f64);
}

#[test]
fn a_single_island_anchors_the_ocean_and_stays_finite() {
    // One land cell is enough to anchor psi = 0 (islands carry no separate circulation: documented limit). Mirror L4: max speed 0.443.
    let d = sphere(4);
    let n = d.n_cells();
    let mut land = vec![false; n];
    land[0] = true;
    let tau = zonal_stress(&d, 0.1);
    let beta = earth_beta(&d, OMEGA);
    let c = Case::new(d, &land, &tau, beta);
    let (m, f) = c.run(&[]);
    assert_eq!(param(&m, "unanchored_cells"), 0.0);
    assert_eq!(param(&m, "clamped_cells"), 0.0);
    assert!(param(&m, "max_speed_raw") < 3.0, "{}", param(&m, "max_speed_raw"));
    assert!(f.data("psi").unwrap().iter().chain(f.data("current").unwrap()).all(|v| v.is_finite()));
    assert!(max_abs(f.data("psi").unwrap()) > 0.0);
    assert_eq!(f.data("psi").unwrap()[0], 0.0);
    assert_eq!(f.data("current_speed").unwrap()[0], 0.0);
}

// ------------------------------------------------------------------------------------------------ 3. plane worlds
const NX: usize = 60;
const NY: usize = 40;

/// Ocean on the cells with `x` (mod NX) in `xs` and `y` in `ys`, land elsewhere.
fn basin_land(xs: &[usize], ys: std::ops::Range<usize>) -> Vec<bool> {
    let mut land = vec![true; NX * NY];
    for y in ys {
        for &x in xs {
            land[y * NX + x % NX] = false;
        }
    }
    land
}

/// Eastward stress `-tau0 cos(2 pi y / Ly)` on the plane (centred coordinates, as in the 4a-2d tests).
fn plane_stress(d: &Domain, tau0: f64) -> Vec<Vec3> {
    let ly = NY as f64 * 10_000.0;
    (0..d.n_cells()).map(|i| [-tau0 * (2.0 * std::f64::consts::PI * d.mesh.centroid[i][1] / ly).cos(), 0.0, 0.0]).collect()
}

fn plane_case(boundary: BoundaryKind, land: &[bool]) -> Case {
    let d = plane(NX, NY, boundary);
    let tau = plane_stress(&d, 0.005);
    Case::new(d, land, &tau, vec![2.3e-11; NX * NY])
}

/// Plane runs use a beta plane (beta 2.3e-11, east = +x), friction 7e-7 (layer 3 cells), no clamp.
const PLANE_PARAMS: [(&str, f64); 2] = [("r_friction", 7.0e-7), ("max_current", BIG)];

#[test]
fn an_enclosed_ocean_is_the_same_wrapped_or_walled_and_closes_its_mass_budget() {
    // Mirror: wall and wrap bit-identical (diff 0); west peak 0.308 vs east peak 0.179 (ratio 1.72); coast flux 9e-18; plane divergence
    // 1e-16 * umax / h; section quadrature residual 0.062 of the absolute flux.
    let land = basin_land(&(8..52).collect::<Vec<_>>(), 6..34);
    let (cw, cr) = (plane_case(BoundaryKind::Wall, &land), plane_case(BoundaryKind::Wrap, &land));
    let (mw, fw) = cw.run(&PLANE_PARAMS);
    let (_, fr) = cr.run(&PLANE_PARAMS);
    assert_eq!((param(&mw, "unanchored_cells"), param(&mw, "wall_pinned_cells")), (0.0, 0.0)); // the basin never touches the wall
    let (pw, pr) = (fw.data("psi").unwrap(), fr.data("psi").unwrap());
    let sc = max_abs(pw);
    assert!(sc > 0.0);
    for i in 0..NX * NY {
        assert!((pw[i] - pr[i]).abs() <= 1e-12 * sc, "walled vs wrapped psi at {i}");
    }
    let (uw, ur) = (fw.data("current").unwrap(), fr.data("current").unwrap());
    let us = max_abs(uw);
    for k in 0..uw.len() {
        assert!((uw[k] - ur[k]).abs() <= 1e-12 * us);
    }
    // Western intensification in the basin
    let sp = fw.data("current_speed").unwrap();
    let peak = |x0: usize, x1: usize| (6..34).flat_map(|y| (x0..x1).map(move |x| y * NX + x)).map(|k| sp[k]).fold(0.0f64, f64::max);
    let (west, east) = (peak(8, 16), peak(44, 52));
    println!("enclosed ocean: west peak {west:.3}, east peak {east:.3} m/s");
    assert!(west > 1.3 * east, "west {west} east {east}"); // mirror 0.308 / 0.179
    // Mass: psi zero off the ocean, no net coast flux, non-divergent interior, section flux small against the absolute flux.
    let u = unpack(&fw);
    for i in 0..NX * NY {
        if land[i] {
            assert_eq!(pw[i], 0.0);
        }
    }
    let ocean: Vec<bool> = land.iter().map(|&l| !l).collect();
    assert_no_net_coast_flux(&cw.d, &ocean, &u);
    let coef = enbuilder::gradient::GradientOp::new(&cw.d).coef().to_vec();
    let hops = coast_hops(&cw.d, &ocean);
    let umax = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
    let worst = (0..NX * NY).filter(|&i| hops[i] >= 3 && hops[i] != u32::MAX).map(|i| div_ls(&cw.d, &coef, &u, i).abs()).fold(0.0f64, f64::max);
    assert!(worst * 10_000.0 / umax < 1e-9, "interior divergence {}", worst * 10_000.0 / umax);
    // Exact transport identity (acceptance: net transport across any cross-section of a closed basin is 0 to 1e-6 relative). With
    // u = k x grad(psi) and central differences, the transport through a full row telescopes to the psi values at its two ends, which are
    // land (psi = 0): rows give sum_x (psi[x+1] - psi[x-1])/2, columns the same along y.
    for y in 6..34 {
        let (mut net, mut ab) = (0.0f64, 0.0f64);
        for x in 1..NX - 1 {
            let t = 0.5 * (pw[y * NX + x + 1] - pw[y * NX + x - 1]);
            net += t;
            ab += t.abs();
        }
        assert!(ab > 0.0 && net.abs() <= 1e-6 * ab, "row {y}: net {net} abs {ab}");
    }
    for x in 8..52 {
        let (mut net, mut ab) = (0.0f64, 0.0f64);
        for y in 1..NY - 1 {
            let t = 0.5 * (pw[(y + 1) * NX + x] - pw[(y - 1) * NX + x]);
            net += t;
            ab += t.abs();
        }
        assert!(ab > 0.0 && net.abs() <= 1e-6 * ab, "column {x}: net {net} abs {ab}");
    }
    // Net northward flux across the zonal section y = 20 against the sum of absolute fluxes (cell-centred quadrature, mirror 0.062).
    let row: Vec<usize> = (0..NX).map(|x| 20 * NX + x).collect();
    let net: f64 = row.iter().map(|&k| u[k][1]).sum();
    let abs: f64 = row.iter().map(|&k| u[k][1].abs()).sum();
    println!("section y=20: net/abs = {:.3}", net.abs() / abs);
    assert!(net.abs() < 0.15 * abs, "net {net} abs {abs}");
}

#[test]
fn a_basin_straddling_the_wrap_seam_matches_the_same_basin_in_the_middle() {
    // Translation invariance on the torus: shifting the basin by half the plane must not change anything (mirror: 4e-16).
    let straddle: Vec<usize> = (50..60).chain(0..10).collect();
    let centred: Vec<usize> = (20..40).collect();
    let (c1, c2) = (plane_case(BoundaryKind::Wrap, &basin_land(&straddle, 6..34)), plane_case(BoundaryKind::Wrap, &basin_land(&centred, 6..34)));
    let (_, f1) = c1.run(&PLANE_PARAMS);
    let (_, f2) = c2.run(&PLANE_PARAMS);
    let (s1, s2) = (f1.data("current_speed").unwrap(), f2.data("current_speed").unwrap());
    let top = max_abs(s1);
    assert!(top > 0.0);
    let shift = |k: usize| (k / NX) * NX + ((k % NX) + 30) % NX;
    let mut crossing = 0;
    for k in 0..NX * NY {
        assert!((s1[k] - s2[shift(k)]).abs() <= 1e-7 * top, "cell {k}: {} vs {}", s1[k], s2[shift(k)]);
        if (k % NX < 10 || k % NX >= 50) && s1[k] > 0.0 {
            crossing += 1;
        }
    }
    assert!(crossing > 100, "the current must be non-zero on both sides of the seam");
}

#[test]
fn an_ocean_touching_a_wall_is_pinned_like_land_and_forms_a_closed_basin() {
    // Wall rule (4a-2e): on a Wall domain, ocean cells with boundary_len > 0 are pinned (psi = 0, no current). Pinning the outer ring
    // must equal declaring that ring land (mirror: identical, max speed 0.516).
    let ring: Vec<bool> = (0..NX * NY).map(|k| !(k % NX > 0 && k % NX < NX - 1 && k / NX > 0 && k / NX < NY - 1)).collect();
    let d = plane(NX, NY, BoundaryKind::Wall);
    let tau = plane_stress(&d, 0.005);
    // Stress curl computed with the interior mask for BOTH runs, so the two differ only in how the ring is treated.
    let interior: Vec<bool> = ring.iter().map(|&r| !r).collect();
    let curl = curl_of_masked(&d, &tau, &interior);
    let beta = vec![2.3e-11; NX * NY];
    let (mask_all, comp_all) = masks(&d, &vec![false; NX * NY]);
    let (mask_in, comp_in) = masks(&d, &ring);
    let a = Case { d: d.clone(), mask: mask_all, comp: comp_all, curl: curl.clone(), beta: beta.clone() };
    let b = Case { d: d.clone(), mask: mask_in, comp: comp_in, curl, beta };
    let eff = a.eff();
    assert_eq!((eff.wall_pinned, eff.unanchored, eff.graph.n), (2 * (NX + NY) - 4, 0, (NX - 2) * (NY - 2)));
    let (ma, fa) = a.run(&PLANE_PARAMS);
    let (mb, fb) = b.run(&PLANE_PARAMS);
    assert_eq!(param(&ma, "wall_pinned_cells"), (2 * (NX + NY) - 4) as f64);
    assert_eq!(param(&mb, "wall_pinned_cells"), 0.0);
    assert!(ma.warning().unwrap().contains("pinned"));
    let (pa, pb) = (fa.data("psi").unwrap(), fb.data("psi").unwrap());
    let sc = max_abs(pb);
    assert!(sc > 0.0);
    for i in 0..NX * NY {
        assert!((pa[i] - pb[i]).abs() <= 1e-12 * sc, "psi at {i}");
        if ring[i] {
            assert_eq!(pa[i], 0.0);
            assert_eq!(fa.data("current_speed").unwrap()[i], 0.0);
        }
    }
    let (ua, ub) = (fa.data("current").unwrap(), fb.data("current").unwrap());
    let us = max_abs(ub);
    for k in 0..ua.len() {
        assert!((ua[k] - ub[k]).abs() <= 1e-12 * us);
    }
    // A Void plane is not pinned.
    let v = plane(NX, NY, BoundaryKind::Void);
    let (vm, vc) = masks(&v, &vec![false; NX * NY]);
    let e = effective_ocean(&v, &vm, &vc).unwrap();
    assert_eq!(e.wall_pinned, 0);
    assert_eq!(e.unanchored, NX * NY, "all-ocean Void plane has no land coast, so it is a waterworld");
}

// ------------------------------------------------------------------------------------------------ 4. cube world
fn cube_land() -> Vec<bool> {
    // Face 5 (-z) is land, plus a 5x5 patch in the middle of face 0 (+x): ocean on faces 0..4 spans many edges.
    let n = 14usize;
    (0..6 * n * n).map(|k| {
        let (face, i, j) = (k / (n * n), (k % (n * n)) % n, (k % (n * n)) / n);
        face == 5 || (face == 0 && (i as f64 - 6.5).abs() <= 2.0 && (j as f64 - 6.5).abs() <= 2.0)
    }).collect()
}

#[test]
fn the_current_crosses_cube_edges_without_a_seam() {
    // Mirror (cube 14/edge, defaults): 964 ocean cells, max speed 0.893, per-face max 0.89 / 0.48 / 0.49 / 0.46 / 0.42 / 0 (land),
    // seam detector with floor 0.15 m/s: 121 evaluable of 312 candidates, worst spike 0.072 (limit 0.2); the same detector on the
    // NON-seam cells reaches 0.36, so seam cells are not worse than ordinary ones. Coast flux 6e-17.
    let d = cube();
    let tau = zonal_stress(&d, 0.1);
    let beta = earth_beta(&d, OMEGA);
    let c = Case::new(d, &cube_land(), &tau, beta);
    let (m, f) = c.run(&[]);
    assert_eq!((param(&m, "unanchored_cells"), param(&m, "clamped_cells")), (0.0, 0.0));
    let sp = f.data("current_speed").unwrap();
    assert!(sp.iter().chain(f.data("psi").unwrap()).all(|v| v.is_finite()));
    let per_face = 14 * 14;
    for face in 0..6 {
        let top = (face * per_face..(face + 1) * per_face).map(|k| sp[k]).fold(0.0f64, f64::max);
        if face == 5 {
            assert_eq!(top, 0.0);
        } else {
            assert!(top > 0.2, "face {face}: max speed {top}"); // mirror >= 0.419
        }
    }
    let eff = c.eff();
    let seam = cross_face_cells(&c.d, 14);
    let rep = seam_spikes(&c.d, &eff.ocean, sp, &seam, 0.2, 0.15);
    let others: Vec<usize> = (0..c.d.n_cells()).filter(|i| !seam.contains(i)).collect();
    let ctl = seam_spikes(&c.d, &eff.ocean, sp, &others, 0.2, 0.15);
    println!("cube edges: worst spike {:.3}, evaluable {}/{}; non-seam control worst {:.3}", rep.worst, rep.evaluable, rep.candidates, ctl.worst);
    assert!(rep.evaluable >= 50, "too few seam cells could be judged: {rep:?}"); // mirror 121
    assert!(rep.worst <= 0.2, "{rep:?}"); // mirror 0.072
    assert!(rep.worst <= ctl.worst, "seam cells are worse than ordinary cells: {rep:?} vs {ctl:?}"); // mirror 0.072 vs 0.36
    assert_no_net_coast_flux(&c.d, &eff.ocean, &unpack(&f));
    let psi = f.data("psi").unwrap();
    for i in 0..c.d.n_cells() {
        if !c.mask[i] {
            assert_eq!(psi[i], 0.0);
        }
    }
}

// ------------------------------------------------------------------------------------------------ 5. tidally locked sphere
/// Rotation period 27.3 days (slow: beta is 30 times smaller than on Earth).
fn omega_tidal() -> f64 { 2.0 * std::f64::consts::PI / (27.3 * 86400.0) }

/// Substellar point on +x, spin axis +z. Surface stress `tau0 sin(theta) (cos a t_s + sgn(lat) sin a (up x t_s))`: inflow toward the
/// substellar point deflected by the Coriolis force (30 degrees, right in the north and left in the south). Not zonal.
fn tidal_stress(d: &Domain, tau0: f64) -> Vec<Vec3> {
    let s: Vec3 = [1.0, 0.0, 0.0];
    let alpha = 30f64.to_radians();
    (0..d.n_cells())
        .map(|i| {
            let up = d.mesh.up[i];
            let c = dot(up, s);
            let raw = sub(s, scale(up, c));
            let nn = length(raw);
            if nn < 1e-9 {
                return [0.0; 3];
            }
            let ts = scale(raw, 1.0 / nn);
            let sin_th = (1.0 - c * c).max(0.0).sqrt();
            let sgn = if up[2] > 0.0 { 1.0 } else if up[2] < 0.0 { -1.0 } else { 0.0 };
            let rot = cross(up, ts);
            scale(add(scale(ts, alpha.cos()), scale(rot, sgn * alpha.sin())), tau0 * sin_th)
        })
        .collect()
}

#[test]
fn a_tidally_locked_sphere_with_non_zonal_stress_gives_a_finite_balanced_non_zonal_current() {
    // Mirror (L4, tau0 0.05, Omega 2.66e-6, H 20, r 1e-5): max speed 0.115; non-zonal fraction of curl second moment 0.853 (zonal control
    // 0.008); of psi 0.603 (control 0.125); vorticity-balance residual 0.136 on 1056 cells (>= 3 hops from the coast; the Earth-like
    // world gives 0.175 with the same definition); doubling the stress doubles psi exactly.
    let d = sphere(4);
    let land = earth_land(&d);
    let beta = earth_beta(&d, omega_tidal());
    let tidal = tidal_stress(&d, 0.05);
    let zonal = zonal_stress(&d, 0.05);
    let ct = Case::new(sphere(4), &land, &tidal, beta.clone());
    let cz = Case::new(sphere(4), &land, &zonal, beta.clone());
    let c2 = Case::new(sphere(4), &land, &tidal_stress(&d, 0.1), beta.clone());
    let (mt, ft) = ct.run(&[("max_current", BIG)]);
    let (_, fz) = cz.run(&[("max_current", BIG)]);
    let (_, f2) = c2.run(&[("max_current", BIG)]);
    let eff = ct.eff();
    assert_eq!(param(&mt, "unanchored_cells"), 0.0);
    let (psi, u) = (ft.data("psi").unwrap(), unpack(&ft));
    assert!(psi.iter().chain(ft.data("current").unwrap()).all(|v| v.is_finite()));
    let smax = max_abs(ft.data("current_speed").unwrap());
    println!("tidally locked L4: max speed {smax:.4} m/s");
    assert!(smax > 0.0 && smax < 1.0, "{smax}"); // mirror 0.115
    // tangent to the surface, zero on land
    for i in 0..d.n_cells() {
        assert!(dot(u[i], d.mesh.up[i]).abs() <= 1e-9 * smax);
        if !ct.mask[i] {
            assert_eq!(length(u[i]), 0.0);
        }
    }
    // Non-zonal forcing and response
    let hops = coast_hops(&ct.d, &eff.ocean);
    let deep: Vec<bool> = (0..d.n_cells()).map(|i| eff.ocean[i] && hops[i] >= 3 && hops[i] != u32::MAX).collect();
    let (nz_t, nz_z) = (nonzonal_fraction(&ct.d, &deep, &ct.curl, 5.0), nonzonal_fraction(&cz.d, &deep, &cz.curl, 5.0));
    assert!(nz_t > 0.5, "tidal curl non-zonal fraction {nz_t}"); // 0.853
    assert!(nz_z < 0.05, "zonal control curl non-zonal fraction {nz_z}"); // 0.008
    let (pz_t, pz_z) = (nonzonal_fraction(&ct.d, &eff.ocean, psi, 5.0), nonzonal_fraction(&cz.d, &eff.ocean, fz.data("psi").unwrap(), 5.0));
    println!("non-zonal fraction of psi: tidal {pz_t:.3}, zonal control {pz_z:.3}");
    assert!(pz_t > 0.3, "{pz_t}"); // 0.603
    assert!(pz_z < 0.25, "{pz_z}"); // 0.125
    assert!(pz_t > 2.0 * pz_z);
    // Momentum balance r zeta + beta v = curl/(rho H) away from the coast
    let (res, cnt) = vorticity_balance(&ct.d, &eff.ocean, &u, &ct.beta, &ct.curl, DEFAULT_R_FRICTION, RHO, DEFAULT_H_MIXED_M, 3);
    println!("vorticity balance residual {res:.3} on {cnt} cells");
    assert!(cnt > 500); // mirror 1056
    assert!(res < 0.25, "{res}"); // mirror 0.136
    // Linear problem: doubling the stress doubles psi
    let p2 = f2.data("psi").unwrap();
    for i in 0..d.n_cells() {
        assert!((p2[i] - 2.0 * psi[i]).abs() <= 1e-9 * max_abs(psi), "psi doubling at {i}");
    }
}

// ------------------------------------------------------------------------------------------------ 6. real pipeline, parameters, interface
fn pipeline_run() -> (Domain, FieldStore, Scheduler) {
    use enbuilder::budget::{add_climate_pipeline, run_season};
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    add_ocean_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    run_season(&d, &mut f, &mut s, 0.0).unwrap();
    (d, f, s)
}

#[test]
fn the_real_climate_pipeline_drives_the_current_stage_end_to_end_and_is_deterministic() {
    // No magnitude is asserted: the wind stress of the real atmosphere is not covered by the mirror (run `examples/current_probe`).
    let (d, f, s) = pipeline_run();
    let n = d.n_cells();
    let mask = f.data("ocean_mask").unwrap();
    let frac = (0..n).filter(|&i| mask[i] > 0.5).map(|i| d.mesh.area[i]).sum::<f64>() / d.mesh.total_area();
    assert!(frac > 0.5 && frac < 0.9, "{frac}");
    for name in ["psi", "current", "current_speed", "stress_curl", "beta", "boundary_layer_m"] {
        assert!(f.has(name), "{name} missing");
        assert!(f.data(name).unwrap().iter().all(|v| v.is_finite()), "{name} not finite");
    }
    assert!(max_abs(f.data("psi").unwrap()) > 0.0);
    assert_eq!(s.module_param("ocean_currents", "unanchored_cells"), Some(0.0));
    let sp = f.data("current_speed").unwrap();
    let psi = f.data("psi").unwrap();
    for i in 0..n {
        if mask[i] < 0.5 {
            assert_eq!(sp[i], 0.0);
            assert_eq!(psi[i], 0.0);
        }
    }
    let (_, f2, _) = pipeline_run();
    assert_eq!(f.data("current").unwrap(), f2.data("current").unwrap());
    assert_eq!(f.data("psi").unwrap(), f2.data("psi").unwrap());
}

#[test]
fn tuned_defaults_and_the_frozen_interface_are_as_documented() {
    let m = OceanCurrents::new();
    let p = m.params();
    assert_eq!(
        (p.get("H_mixed_m"), p.get("r_friction"), p.get("rho_water"), p.get("max_current"), p.get("beta_on"), p.get("coast_slip")),
        (Some(20.0), Some(1.0e-5), Some(1025.0), Some(3.0), Some(1.0), Some(1.0))
    );
    assert_eq!((DEFAULT_H_MIXED_M, DEFAULT_R_FRICTION), (20.0, 1.0e-5));
    // Output-only parameters of 4a-2e
    let mut m = OceanCurrents::new();
    for ro in ["unanchored_cells", "wall_pinned_cells"] {
        assert!(m.params().get(ro).is_some() && m.set_param(ro, 1.0).is_err(), "{ro}");
    }
    // The fields 4a-3 may rely on, and only these names are promised (see ARCHITECTURE.md, "Interface freeze").
    let spec = m.spec();
    for out in ["psi", "current", "current_speed"] {
        assert!(spec.outputs.iter().any(|o| o == out), "{out}");
    }
    let mut s = Scheduler::new();
    add_ocean_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |name: &str| order.iter().position(|x| x == name).unwrap_or_else(|| panic!("{name} missing from {order:?}"));
    assert!(pos("ocean_mask") < pos("ocean_currents"));
    // ocean_mask and coast_distance come from the mask stage (frozen outputs of 4a-1)
    let mask_spec = OceanMask::new().spec();
    for out in ["ocean_mask", "coast_distance"] {
        assert!(mask_spec.outputs.iter().any(|o| o == out), "{out}");
    }
}