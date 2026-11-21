//! Part 4c-1 acceptance tests: stream power erosion (`erosion`) and the `TerrainShaping::with_erosion` variant.
//! NOT compiled when written (no Rust toolchain in the sandbox). Hand numbers come from `reference/mirror_4c1.py`
//! (`python mirror_4c1.py`); randomized ordering invariants of the same scheme pass there.
use enbuilder::generators::*;
use enbuilder::math::{length, sub};
use enbuilder::rng::Rng;
use enbuilder::*;

const R: f64 = 6.371e6;
const CELL: f64 = 10_000.0;
const A: f64 = CELL * CELL;

fn close(a: f64, b: f64, rel: f64) -> bool { (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-300) }
fn plane_g(nx: usize, ny: usize, g: f64) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: CELL, boundary: BoundaryKind::Wall, surface_gravity: g, seed: 1 }).unwrap()
}
fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }

/// Same terrain as `tests/part4b2.rs` / `part4b3.rs`.
fn terrain(d: &Domain, seed: u64) -> Vec<f64> {
    let n = d.n_cells();
    let c = &d.mesh.centroid;
    let mut mid = [0.0; 3];
    for p in c {
        for k in 0..3 { mid[k] += p[k]; }
    }
    for k in 0..3 { mid[k] /= n as f64; }
    let ext = c.iter().map(|p| length(sub(*p, mid))).fold(1.0, f64::max);
    let mut rng = Rng::new(seed);
    (0..n).map(|i| 800.0 * ((c[i][0] - mid[0]) / ext) - 150.0 + 300.0 * rng.next_f64()).collect()
}

// ---------------------------------------------------------------------------------------------
// Hand-built chain: plane 6 x 3, row j = 1 (cells 6..11). Column 0 is ocean (-10 m), the receiver of column i is column i-1, z = 100 i
// (column 0: -10), upstream area of column i is (6 - i) * 1e8 m^2, distance 1e4 m. Every other cell is ocean and is skipped.
// ---------------------------------------------------------------------------------------------

const CHAIN_Z: [f64; 6] = [-10.0, 100.0, 200.0, 300.0, 400.0, 500.0];

fn chain_with(g: f64, strength: [f64; 6], z: [f64; 6]) -> (Domain, FieldStore) {
    let d = plane_g(6, 3, g);
    let n = d.n_cells();
    let (mut zz, mut fd, mut ocean, mut area, mut rs) = (vec![1000.0; n], vec![-1.0; n], vec![1.0; n], vec![0.0; n], vec![1.0; n]);
    for i in 0..6 {
        let c = 6 + i;
        zz[c] = z[i];
        rs[c] = strength[i];
        if i >= 1 {
            fd[c] = (c - 1) as f64;
            ocean[c] = 0.0;
            area[c] = (6 - i) as f64 * A;
        }
    }
    let order: Vec<f64> = (0..6).chain(12..18).chain((6..12).rev()).map(|c| c as f64).collect();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, zz.clone()).unwrap();
    f.put("elevation_filled", 1, zz).unwrap();
    f.put("flow_dir", 1, fd).unwrap();
    f.put("flow_order", 1, order).unwrap();
    f.put("drainage_area", 1, area).unwrap();
    f.put("depression_depth", 1, vec![0.0; n]).unwrap();
    f.put("ocean_mask", 1, ocean).unwrap();
    f.put("rock_strength", 1, rs).unwrap();
    seed_erosion_state(&mut f).unwrap();
    (d, f)
}
fn chain(g: f64) -> (Domain, FieldStore) { chain_with(g, [1.0; 6], CHAIN_Z) }

/// New elevations of the chain after one `erosion` run (column 0..5).
fn chain_after(f: &FieldStore) -> Vec<f64> {
    let z = f.data("elevation").unwrap();
    let st = f.data("erosion_step").unwrap();
    (6..12).map(|c| z[c] + st[c]).collect()
}

const WANT_1G: [f64; 6] = [-10.0, 95.29123291533112, 195.97273972751273, 296.5170431608372, 397.15356920010316, 497.98340331764905];
const WANT_2G: [f64; 6] = [-10.0, 90.96905332560577, 191.92363357967452, 292.99740378750016, 394.2710949006481, 495.93350365002493];

// ---------------------------------------------------------------------------------------------
// Pure function
// ---------------------------------------------------------------------------------------------

#[test]
fn incise_solves_the_implicit_equation() {
    // n = 1: closed form z_r + d / (1 + c)
    assert!(close(incise(100.0, 20.0, 1.0, 1.0), 60.0, 1e-15));
    assert!(close(incise(100.0, 20.0, 3.0, 1.0), 40.0, 1e-15));
    // nothing to cut: no coefficient, or the receiver is not lower
    assert_eq!(incise(100.0, 20.0, 0.0, 1.0), 100.0);
    assert_eq!(incise(100.0, 100.0, 5.0, 1.0), 100.0);
    assert_eq!(incise(100.0, 150.0, 5.0, 2.0), 100.0);
    // other exponents: h + c h^n = d must hold, and the root lies in (0, d]
    for &n in &[0.5, 1.5, 2.0, 3.0] {
        for &c in &[1e-6, 0.3, 5.0, 1e3] {
            let z = incise(100.0, 20.0, c, n);
            let h = z - 20.0;
            assert!(h > 0.0 && h <= 80.0, "n {n} c {c}: h {h}");
            assert!((h + c * h.powf(n) - 80.0).abs() <= 1e-9 * 80.0, "n {n} c {c}: residual");
        }
    }
    assert!(close(incise(100.0, 20.0, 0.01, 2.0), 72.46950765959603, 1e-9));
    // a huge coefficient pins the cell just above its receiver, never below
    for &n in &[0.5, 1.0, 2.0] {
        let z = incise(100.0, 20.0, 1e9, n);
        assert!(z > 20.0 && z < 20.001, "n {n}: {z}");
    }
}

// ---------------------------------------------------------------------------------------------
// The module on the hand-built chain
// ---------------------------------------------------------------------------------------------

#[test]
fn chain_hand_numbers_volume_rate_and_skips() {
    let (d, mut f) = chain(9.81);
    let mut e = Erosion::new();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let after = chain_after(&f);
    for i in 0..6 {
        assert!(close(after[i], WANT_1G[i], 1e-12), "column {i}: {} vs {}", after[i], WANT_1G[i]);
    }
    let st = f.data("erosion_step").unwrap();
    let delta = f.data("erosion_delta").unwrap();
    let rate = f.data("incision_rate").unwrap();
    let z0 = f.data("elevation").unwrap();
    for c in 0..d.n_cells() {
        assert!(st[c] <= 0.0 && delta[c] == st[c], "first step: delta equals the step (cell {c})");
        assert!(close(rate[c], -st[c] / 1.0e5, 1e-14) || (rate[c] == 0.0 && st[c] == 0.0));
        if !(7..12).contains(&c) { assert_eq!(st[c], 0.0, "cell {c} must not be incised"); }
    }
    // volume: 1.7082e9 m^3 (every cell area is 1e8 m^2)
    let vol: f64 = (0..d.n_cells()).map(|c| d.mesh.area[c] * (-st[c])).sum();
    assert!(close(vol, 1708201167.856673, 1e-10), "volume {vol}");
    assert!(close(e.params().get("eroded_volume_m3").unwrap(), vol, 1e-12));
    assert_eq!(e.params().get("n_incised_cells").unwrap(), 5.0);
    assert_eq!(e.params().get("n_capped_cells").unwrap(), 0.0);
    assert!(close(e.params().get("max_incision_m").unwrap(), 100.0 - WANT_1G[1], 1e-12));
    // rate of column 1: 4.7088e-5 m/yr
    assert!(close(rate[7], 4.708767084668878e-5, 1e-10));
    // the elevation field itself is untouched: erosion never writes `elevation`
    assert_eq!(z0[7], 100.0);
    // the ordering holds: every cut cell is still above its (new) receiver
    for i in 1..6 { assert!(after[i] > after[i - 1]); }
}

#[test]
fn state_is_committed_not_advanced_by_reruns() {
    let (d, mut f) = chain(9.81);
    let mut e = Erosion::new();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let first = f.data("erosion_delta").unwrap().to_vec();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert_eq!(f.data("erosion_delta").unwrap(), &first[..], "a rerun without commit must not advance time");
    commit_erosion_state(&mut f).unwrap();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    // elevation was not updated, so the second step cuts the same amount again: delta = 2 * step
    let (st, delta) = (f.data("erosion_step").unwrap(), f.data("erosion_delta").unwrap());
    for c in 0..d.n_cells() { assert!(close(delta[c], 2.0 * st[c], 1e-14) || (delta[c] == 0.0 && st[c] == 0.0)); }
}

#[test]
fn doubled_gravity_and_weaker_rock_erode_faster() {
    let mut e = Erosion::new();
    let (d1, mut f1) = chain(9.81);
    e.run(&d1, &mut f1, &DirtyRegion::All).unwrap();
    let one = chain_after(&f1);
    let (d2, mut f2) = chain(19.62);
    e.run(&d2, &mut f2, &DirtyRegion::All).unwrap();
    let two = chain_after(&f2);
    for i in 1..6 {
        assert!(two[i] < one[i], "column {i}: 2g must cut deeper");
        assert!(close(two[i], WANT_2G[i], 1e-12), "column {i}: {} vs {}", two[i], WANT_2G[i]);
    }
    // half the strength is the same as double K_eff, hence the same numbers as 2 g (gravity_exp 1)
    let (d3, mut f3) = chain_with(9.81, [0.5; 6], CHAIN_Z);
    e.run(&d3, &mut f3, &DirtyRegion::All).unwrap();
    let weak = chain_after(&f3);
    for i in 1..6 { assert!(close(weak[i], WANT_2G[i], 1e-12), "column {i}"); }
    // a weak region only: columns 1..=2 weak, the rest normal. Weak cells cut more than in the uniform case.
    let (d4, mut f4) = chain_with(9.81, [1.0, 0.25, 0.25, 1.0, 1.0, 1.0], CHAIN_Z);
    e.run(&d4, &mut f4, &DirtyRegion::All).unwrap();
    let mixed = chain_after(&f4);
    assert!(mixed[1] < one[1] && mixed[2] < one[2]);
    // in the small-step limit the cut is proportional to g (ratio 2 within the implicit nonlinearity, ~c)
    let mut small = Erosion::new();
    small.set_param("years_per_step", 1.0e3).unwrap();
    let (da, mut fa) = chain(9.81);
    let (db, mut fb) = chain(19.62);
    small.run(&da, &mut fa, &DirtyRegion::All).unwrap();
    let a = chain_after(&fa);
    small.run(&db, &mut fb, &DirtyRegion::All).unwrap();
    let b = chain_after(&fb);
    for i in 1..6 {
        let ratio = (CHAIN_Z[i] - b[i]) / (CHAIN_Z[i] - a[i]);
        assert!((ratio - 2.0).abs() < 2e-3, "column {i}: ratio {ratio}");
    }
    // gravity_exp = 0 switches the link off
    let mut nog = Erosion::new();
    nog.set_param("gravity_exp", 0.0).unwrap();
    let (dc, mut fc) = chain(19.62);
    nog.run(&dc, &mut fc, &DirtyRegion::All).unwrap();
    let c = chain_after(&fc);
    for i in 1..6 { assert!(close(c[i], WANT_1G[i], 1e-12), "column {i}"); }
}

#[test]
fn incision_cap_and_huge_steps() {
    // cap: 3 m per step. Column 1 and 2, 3 are capped (they would cut 4.7, 4.0, 3.5 m); columns 4, 5 are not.
    let (d, mut f) = chain(9.81);
    let mut e = Erosion::new();
    e.set_param("max_incision_m_per_step", 3.0).unwrap();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let a = chain_after(&f);
    let want = [-10.0, 97.0, 197.0, 297.0, 397.1668535443469, 497.9836637949872];
    for i in 0..6 { assert!(close(a[i], want[i], 1e-12), "column {i}: {} vs {}", a[i], want[i]); }
    assert_eq!(e.params().get("n_capped_cells").unwrap(), 3.0);
    assert!(close(e.params().get("max_incision_m").unwrap(), 3.0, 1e-12));
    // huge step: still finite, still ordered, every cell pinned just above its receiver
    let (d, mut f) = chain(9.81);
    let mut e = Erosion::new();
    e.set_param("years_per_step", 1.0e15).unwrap();
    e.set_param("max_incision_m_per_step", 1.0e9).unwrap();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let a = chain_after(&f);
    assert!(a.iter().all(|v| v.is_finite()));
    for i in 1..6 { assert!(a[i] >= a[i - 1] && a[i] < -9.99, "column {i}: {}", a[i]); }
    // exponent 2 (non-linear law, Newton path)
    let (d, mut f) = chain(9.81);
    let mut e = Erosion::new();
    e.set_param("n_exp", 2.0).unwrap();
    e.set_param("K", 1.0e-4).unwrap();
    e.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let a = chain_after(&f);
    for i in 1..6 {
        assert!(a[i] < CHAIN_Z[i] && a[i] >= a[i - 1], "column {i}: {}", a[i]);
    }
}

// ---------------------------------------------------------------------------------------------
// World-scale: routed random terrain, repeated steps
// ---------------------------------------------------------------------------------------------

struct World {
    d: Domain,
    f: FieldStore,
    routing: Scheduler,
}

fn eroding_world(level: u32, seed: u64) -> World {
    let d = sphere(level);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, terrain(&d, seed)).unwrap();
    f.put("rock_strength", 1, vec![1.0; n]).unwrap();
    seed_erosion_state(&mut f).unwrap();
    let mut routing = Scheduler::new();
    add_flow_routing_stage(&mut routing).unwrap();
    World { d, f, routing }
}

fn fast_erosion() -> Erosion {
    let mut e = Erosion::new();
    e.set_param("years_per_step", 1.0e6).unwrap();
    e
}

/// One more routing + erosion run WITHOUT changing the elevation; checks the single-step invariants on it.
fn check_single_step(w: &mut World, e: &mut Erosion) {
    w.routing.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
    e.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
    let n = w.d.n_cells();
    let z = w.f.data("elevation").unwrap().to_vec();
    let filled = w.f.data("elevation_filled").unwrap().to_vec();
    let dep = w.f.data("depression_depth").unwrap().to_vec();
    let ocean = w.f.data("ocean_mask").unwrap().to_vec();
    let fd: Vec<i64> = w.f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect();
    let st = w.f.data("erosion_step").unwrap().to_vec();
    let after: Vec<f64> = (0..n).map(|c| z[c] + st[c]).collect();
    let mut vol = 0.0;
    for c in 0..n {
        assert!(st[c].is_finite() && st[c] <= 0.0, "cell {c}");
        vol += w.d.mesh.area[c] * (-st[c]);
        if ocean[c] > 0.5 || dep[c] > 0.0 || fd[c] < 0 {
            assert_eq!(st[c], 0.0, "ocean, lake and outlet cells are not incised (cell {c})");
            continue;
        }
        let r = fd[c] as usize;
        let base = if dep[r] > 0.0 { filled[r] } else { after[r] };
        if z[c] > base {
            assert!(after[c] >= base - 1e-9, "cell {c} went below its receiver's base level: {} < {}", after[c], base);
        } else {
            assert_eq!(st[c], 0.0, "a cell with no drop is not cut");
        }
    }
    assert!(close(vol, e.params().get("eroded_volume_m3").unwrap(), 1e-12), "reported volume must equal the removed volume");
    // after huge steps everything is pinned just above its receiver and the cut can be below float resolution, so only >= 0 is general
    assert!(vol >= 0.0);
}

#[test]
fn single_step_invariants_on_a_routed_sphere() {
    let mut w = eroding_world(3, 3);
    let mut e = fast_erosion();
    check_single_step(&mut w, &mut e);
    assert!(e.params().get("eroded_volume_m3").unwrap() > 0.0);
}

#[test]
fn repeated_steps_lower_peaks_and_deepen_valleys_without_new_pits() {
    let mut w = eroding_world(3, 3);
    let n = w.d.n_cells();
    w.routing.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
    let z0 = w.f.data("elevation").unwrap().to_vec();
    let ocean0: Vec<bool> = w.f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let area0 = w.f.data("drainage_area").unwrap().to_vec();
    let dep0 = w.f.data("depression_depth").unwrap().to_vec();
    let fd0: Vec<i64> = w.f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect();
    let n_dep0 = dep0.iter().filter(|&&d| d > 0.0).count();
    let peak0 = (0..n).filter(|&c| !ocean0[c]).map(|c| z0[c]).fold(f64::MIN, f64::max);
    let mean0: f64 = (0..n).filter(|&c| !ocean0[c]).map(|c| z0[c]).sum::<f64>() / (0..n).filter(|&c| !ocean0[c]).count() as f64;
    // the trunk river: largest drainage area among land cells that are neither lake nor outlet
    let trunk = (0..n).filter(|&c| !ocean0[c] && dep0[c] == 0.0 && fd0[c] >= 0).max_by(|&a, &b| area0[a].total_cmp(&area0[b])).unwrap();

    let mut e = fast_erosion();
    let rep = run_erosion_steps(&w.d, &mut w.f, &mut w.routing, &mut e, 15).unwrap();
    assert_eq!(rep.len(), 15);
    assert!(rep.iter().all(|r| r.eroded_volume_m3 > 0.0 && r.max_incision_m.is_finite()));
    let z1 = w.f.data("elevation").unwrap().to_vec();
    assert!(z1.iter().all(|v| v.is_finite()));
    let peak1 = (0..n).filter(|&c| !ocean0[c]).map(|c| z1[c]).fold(f64::MIN, f64::max);
    let mean1: f64 = (0..n).filter(|&c| !ocean0[c]).map(|c| z1[c]).sum::<f64>() / (0..n).filter(|&c| !ocean0[c]).count() as f64;
    assert!(peak1 < peak0, "peak {peak0} -> {peak1}");
    assert!(mean1 < mean0, "mean land height {mean0} -> {mean1}");
    assert!(z1[trunk] < z0[trunk] - 1e-6, "the trunk river bed must deepen: {} -> {}", z0[trunk], z1[trunk]);
    for c in 0..n { assert!(z1[c] <= z0[c] + 1e-12, "4c-1 only cuts (cell {c})"); }
    // the cumulative field equals the total change, and the volume bookkeeping closes
    let delta = w.f.data("erosion_delta").unwrap();
    let area = &w.d.mesh.area;
    let mut vol_from_delta = 0.0;
    for c in 0..n {
        assert!(close(delta[c], z1[c] - z0[c], 1e-9) || (delta[c] - (z1[c] - z0[c])).abs() < 1e-9, "cell {c}");
        vol_from_delta += area[c] * (-delta[c]);
    }
    let vol_reported: f64 = rep.iter().map(|r| r.eroded_volume_m3).sum();
    assert!(close(vol_from_delta, vol_reported, 1e-10), "removed volume {vol_from_delta} vs reported {vol_reported}");
    // no new pits: route the eroded surface once more and count the depression cells
    w.routing.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
    let n_dep1 = w.f.data("depression_depth").unwrap().iter().filter(|&&d| d > 0.0).count();
    assert!(n_dep1 <= n_dep0, "depression cells {n_dep0} -> {n_dep1}: erosion must not create pits");
    // and the invariants still hold on the eroded world
    check_single_step(&mut w, &mut e);
}

#[test]
fn huge_steps_stay_finite_and_ordered_on_a_sphere() {
    let mut w = eroding_world(3, 5);
    let mut e = Erosion::new();
    e.set_param("years_per_step", 1.0e13).unwrap();
    e.set_param("max_incision_m_per_step", 1.0e9).unwrap();
    check_single_step(&mut w, &mut e);
    assert!(e.params().get("eroded_volume_m3").unwrap() > 0.0, "the first huge step still removes material");
    let rep = run_erosion_steps(&w.d, &mut w.f, &mut w.routing, &mut e, 3).unwrap();
    assert!(rep.iter().all(|r| r.eroded_volume_m3.is_finite()));
    assert!(w.f.data("elevation").unwrap().iter().all(|v| v.is_finite()));
    check_single_step(&mut w, &mut e);
}

#[test]
fn weak_region_and_heavy_gravity_on_a_sphere() {
    // total cut with a weak half-world is larger than with uniform strength; the weak half cuts more per cell than the strong half
    let cut = |strength: &dyn Fn(usize, &Domain) -> f64| -> (f64, Vec<f64>) {
        let mut w = eroding_world(3, 3);
        let n = w.d.n_cells();
        let rs: Vec<f64> = (0..n).map(|c| strength(c, &w.d)).collect();
        w.f.put("rock_strength", 1, rs).unwrap();
        w.routing.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
        let mut e = fast_erosion();
        e.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
        (e.params().get("eroded_volume_m3").unwrap(), w.f.data("erosion_step").unwrap().to_vec())
    };
    let (v_uniform, s_uniform) = cut(&|_, _| 1.0);
    let (v_weak, s_weak) = cut(&|c, d| if d.mesh.up[c][1] > 0.0 { 0.25 } else { 1.0 });
    assert!(v_weak > v_uniform, "{v_weak} vs {v_uniform}");
    let d = sphere(3);
    for c in 0..d.n_cells() {
        // a weak cell never cuts less than in the uniform world (same receivers, larger coefficient)
        if d.mesh.up[c][1] > 0.0 { assert!(s_weak[c] <= s_uniform[c] + 1e-12, "cell {c}"); }
    }
    // doubled gravity on the whole planet: more volume removed
    let g2 = generate_sphere(&SphereParams { radius: R, subdivision: 3, surface_gravity: 19.62, seed: 1 }).unwrap();
    let mut f = FieldStore::new(g2.n_cells());
    f.put("elevation", 1, terrain(&g2, 3)).unwrap();
    f.put("rock_strength", 1, vec![1.0; g2.n_cells()]).unwrap();
    seed_erosion_state(&mut f).unwrap();
    let mut routing = Scheduler::new();
    add_flow_routing_stage(&mut routing).unwrap();
    routing.run(&g2, &mut f, &DirtyRegion::All).unwrap();
    let mut e = fast_erosion();
    e.run(&g2, &mut f, &DirtyRegion::All).unwrap();
    assert!(e.params().get("eroded_volume_m3").unwrap() > v_uniform, "2 g must remove more than 1 g");
}

#[test]
fn erosion_is_deterministic() {
    let run = || -> Vec<f64> {
        let mut w = eroding_world(3, 9);
        let mut e = fast_erosion();
        run_erosion_steps(&w.d, &mut w.f, &mut w.routing, &mut e, 5).unwrap();
        w.f.data("elevation").unwrap().to_vec()
    };
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------------------------------------
// Interface, errors, terrain variant
// ---------------------------------------------------------------------------------------------

#[test]
fn erosion_interface_and_errors() {
    let s = Erosion::new().spec();
    assert_eq!(s.name, "erosion");
    for i in ["elevation", "elevation_filled", "flow_dir", "flow_order", "drainage_area", "depression_depth", "ocean_mask", "rock_strength", "erosion_delta_prev"] {
        assert!(s.inputs.iter().any(|x| x == i), "missing input {i}");
    }
    assert_eq!(s.outputs, vec!["erosion_delta".to_string(), "erosion_step".to_string(), "incision_rate".to_string()]);
    assert!(!s.outputs.iter().any(|x| x == "elevation"), "terrain stays the only producer of elevation");
    let p = Erosion::new();
    for (k, v) in EROSION_PARAMS.iter() { assert_eq!(p.params().get(k), Some(*v)); }
    assert_eq!((p.params().get("K"), p.params().get("m_exp"), p.params().get("n_exp")), (Some(2.0e-7), Some(0.5), Some(1.0)));
    let mut m = Erosion::new();
    assert!(m.set_param("K", -1.0).is_err());
    assert!(m.set_param("n_exp", 0.0).is_err());
    assert!(m.set_param("years_per_step", 0.0).is_err());
    assert!(m.set_param("max_incision_m_per_step", -1.0).is_err());
    assert!(m.set_param("K", f64::NAN).is_err());
    assert!(m.set_param("nonsense", 1.0).is_err());
    assert!(m.set_param("eroded_volume_m3", 1.0).is_err(), "output-only parameters cannot be set");
    assert!(m.set_param("m_exp", 0.4).is_ok());
    // missing inputs are errors, not defaults
    let (d, _) = chain(9.81);
    let mut empty = FieldStore::new(d.n_cells());
    assert!(m.run(&d, &mut empty, &DirtyRegion::All).is_err());
    // flow_order that is not uphill -> downhill is rejected
    let (d, mut f) = chain(9.81);
    // ascending cell numbers put every receiver BEFORE its donor on the chain (the valid order is descending)
    let bad: Vec<f64> = (0..18).map(|c| c as f64).collect();
    f.put("flow_order", 1, bad).unwrap();
    assert!(matches!(Erosion::new().run(&d, &mut f, &DirtyRegion::All), Err(Error::InvalidParam(_))));
    // non-finite input
    let (d, mut f) = chain(9.81);
    let mut z = f.data("elevation").unwrap().to_vec();
    z[8] = f64::NAN;
    f.put("elevation", 1, z).unwrap();
    assert!(matches!(Erosion::new().run(&d, &mut f, &DirtyRegion::All), Err(Error::NonFinite(_))));
    // wiring
    let mut sch = Scheduler::new();
    add_erosion_module(&mut sch).unwrap();
    sch.plan_order().unwrap();
    assert!(sch.external_inputs().contains("erosion_delta_prev"));
}

#[test]
fn terrain_variant_adds_the_cumulative_delta_and_old_terrain_is_unchanged() {
    let d = sphere(2);
    let n = d.n_cells();
    let mut rng = Rng::new(4);
    let tect: Vec<f64> = (0..n).map(|_| 2000.0 * rng.next_f64()).collect();
    let mk = |delta: Option<Vec<f64>>| -> FieldStore {
        let mut f = FieldStore::new(n);
        f.put("tectonic_elevation", 1, tect.clone()).unwrap();
        f.put("rock_strength", 1, vec![1.0; n]).unwrap();
        if let Some(dl) = delta { f.put("erosion_delta", 1, dl).unwrap(); }
        f
    };
    // spec: the variant has the extra input, the old module does not
    assert!(TerrainShaping::with_erosion().spec().inputs.iter().any(|s| s == "erosion_delta"));
    assert!(!TerrainShaping::new().spec().inputs.iter().any(|s| s == "erosion_delta"));
    // zero delta: bit-identical to the old terrain
    let mut a = mk(None);
    TerrainShaping::new().run(&d, &mut a, &DirtyRegion::All).unwrap();
    let mut b = mk(Some(vec![0.0; n]));
    TerrainShaping::with_erosion().run(&d, &mut b, &DirtyRegion::All).unwrap();
    assert_eq!(a.data("elevation").unwrap(), b.data("elevation").unwrap());
    assert_eq!(a.data("slope").unwrap(), b.data("slope").unwrap());
    // with noise, thermal erosion and caps off the result is exactly tectonic + delta
    let delta: Vec<f64> = (0..n).map(|_| -60.0 * rng.next_f64()).collect();
    let mut c = mk(Some(delta.clone()));
    let mut t = TerrainShaping::with_erosion();
    t.set_param("noise_amp_m", 0.0).unwrap();
    t.set_param("thermal_iters", 0.0).unwrap();
    t.set_param("peak_ref_m", 1.0e6).unwrap();
    t.set_param("depth_ref_m", 1.0e6).unwrap();
    t.run(&d, &mut c, &DirtyRegion::All).unwrap();
    let e = c.data("elevation").unwrap();
    for i in 0..n { assert!(close(e[i], tect[i] + delta[i], 1e-12) || (e[i] - (tect[i] + delta[i])).abs() < 1e-9, "cell {i}"); }
    // the variant needs the field
    let mut miss = mk(None);
    assert!(TerrainShaping::with_erosion().run(&d, &mut miss, &DirtyRegion::All).is_err());
    // wrong length or NaN is rejected
    let mut bad = mk(Some(vec![f64::NAN; n]));
    assert!(TerrainShaping::with_erosion().run(&d, &mut bad, &DirtyRegion::All).is_err());
    // terrain and erosion form a cycle: they cannot share a scheduler until the 4c-3 loop exists
    let mut s = Scheduler::new();
    s.register(Box::new(TerrainShaping::with_erosion())).unwrap();
    s.register(Box::new(Erosion::new())).unwrap();
    assert!(s.plan_order().is_err());
}
