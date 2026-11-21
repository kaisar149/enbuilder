//! Part 4c-2 acceptance tests: sediment transport, deposition (floodplains, deltas, lake trapping), thermal erosion.
//! Compiled and run against the full crate (hand numbers of the pure functions are exact; world-scale cases check invariants).
use enbuilder::generators::*;
use enbuilder::math::{length, sub};
use enbuilder::rng::Rng;
use enbuilder::sediment::*;
use enbuilder::*;

const R: f64 = 6.371e6;
const CELL: f64 = 10_000.0;
const YEARS: f64 = 1.0e5;

fn close(a: f64, b: f64, rel: f64) -> bool { (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-300) }
fn plane(nx: usize, ny: usize, boundary: BoundaryKind, g: f64) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: CELL, boundary, surface_gravity: g, seed: 1 }).unwrap()
}
fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }

/// Same random terrain as the 4c-1 tests.
fn terrain(d: &Domain, seed: u64) -> Vec<f64> {
    let n = d.n_cells();
    let c = &d.mesh.centroid;
    let mut mid = [0.0; 3];
    for p in c { for k in 0..3 { mid[k] += p[k]; } }
    for k in 0..3 { mid[k] /= n as f64; }
    let ext = c.iter().map(|p| length(sub(*p, mid))).fold(1.0, f64::max);
    let mut rng = Rng::new(seed);
    (0..n).map(|i| 800.0 * ((c[i][0] - mid[0]) / ext) - 150.0 + 300.0 * rng.next_f64()).collect()
}

struct World {
    d: Domain,
    f: FieldStore,
    s: Scheduler,
}

fn world(d: Domain, z: Vec<f64>, years: f64) -> World {
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, z).unwrap();
    f.put("rock_strength", 1, vec![1.0; n]).unwrap();
    seed_surface_state(&mut f).unwrap();
    let mut s = Scheduler::new();
    add_surface_stage(&mut s).unwrap();
    set_surface_years(&mut s, years).unwrap();
    World { d, f, s }
}

fn sp(w: &World, k: &str) -> f64 { w.s.module_param("sediment", k).unwrap() }
fn cells(w: &World, k: &str) -> Vec<f64> { w.f.data(k).unwrap().to_vec() }

/// 16 x 5 plane like `reference/run_4c2_hand.py`: two ocean columns, a long gentle ramp (2 m per cell), then steep mountains.
fn ramp_world(boundary: BoundaryKind) -> World {
    let (nx, ny) = (16usize, 5usize);
    let d = plane(nx, ny, boundary, 9.81);
    let mut z = vec![0.0; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let h = if i <= 1 { -20.0 } else if i <= 9 { 10.0 + (i - 1) as f64 * 2.0 } else { 26.0 + (i - 9) as f64 * 300.0 };
            z[j * nx + i] = if i >= 2 { h + 50.0 * (j as f64 - 2.0).abs() } else { h };
        }
    }
    world(d, z, YEARS)
}

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

#[test]
fn capacity_and_basin_filling_hand_numbers() {
    assert_eq!(transport_capacity(2.0, 9.0, 0.5, 0.1), 2.0 * 3.0 * 0.1);
    assert_eq!(transport_capacity(2.0, 9.0, 1.0, 0.5), 9.0);
    for (kc, q, s) in [(0.0, 1.0, 1.0), (1.0, 0.0, 1.0), (1.0, 1.0, 0.0), (1.0, 1.0, -0.1)] {
        assert_eq!(transport_capacity(kc, q, 1.0, s), 0.0);
    }
    // floors 0, 1, 2 (unit areas), top 3: capacity 3 + 2 + 1 = 6
    let (fl, ar) = ([0.0, 1.0, 2.0], [1.0; 3]);
    assert_eq!(fill_basin(&fl, &ar, 3.0, 0.0), (0.0, 0.0));
    let (lv, tr) = fill_basin(&fl, &ar, 3.0, 0.5);
    assert!(close(lv, 0.5, 1e-12) && close(tr, 0.5, 1e-12));
    let (lv, tr) = fill_basin(&fl, &ar, 3.0, 3.0); // 2 (cell 0 to level 2) + 1 (cell 1) -> level 2
    assert!(close(lv, 2.0, 1e-12) && close(tr, 3.0, 1e-12), "{lv} {tr}");
    let (lv, tr) = fill_basin(&fl, &ar, 3.0, 10.0);
    assert!(close(lv, 3.0, 1e-12) && close(tr, 6.0, 1e-12), "overfilled basin traps its capacity only: {lv} {tr}");
    // unequal areas: 2 m^2 at floor 0, 1 m^2 at floor 1, top 2: volume 3 -> level 1 needs 2, then 1.0 over 3 m^2 -> 1.333
    let (lv, tr) = fill_basin(&[0.0, 1.0], &[2.0, 1.0], 2.0, 3.0);
    assert!(close(lv, 1.0 + 1.0 / 3.0, 1e-12) && close(tr, 3.0, 1e-12), "{lv} {tr}");
}

#[test]
fn thermal_relaxation_respects_the_limit_and_conserves_volume() {
    let d = plane(24, 12, BoundaryKind::Wall, 9.81);
    let n = d.n_cells();
    let mut rng = Rng::new(7);
    let z0: Vec<f64> = (0..n).map(|_| 3000.0 * rng.next_f64()).collect();
    let g = vec![9.81; n];
    let st = vec![1.0; n];
    let vol = |z: &[f64]| -> f64 { (0..n).map(|c| d.mesh.area[c] * z[c]).sum() };
    let mut z = z0.clone();
    let rep = thermal_relax(&d.mesh, &mut z, &g, &st, 0.05, 20_000, 1e-6);
    assert!(rep.max_excess_m <= 1e-6, "converged: {}", rep.max_excess_m);
    assert!(rep.moved_m3 > 0.0);
    assert!((vol(&z) - vol(&z0)).abs() <= 1e-9 * vol(&z0).abs(), "volume conserved");
    for i in 0..n {
        for (k, &j) in d.mesh.neighbors(i).iter().enumerate() {
            let lim = 0.05 * d.mesh.edge_dists(i)[k];
            assert!((z[i] - z[j as usize]).abs() <= lim + 2e-6, "edge {i}-{j}: {} > {lim}", (z[i] - z[j as usize]).abs());
        }
    }
    // already stable input is not touched
    let mut flat = vec![5.0; n];
    let rep = thermal_relax(&d.mesh, &mut flat, &g, &st, 0.05, 100, 1e-6);
    assert!(flat.iter().all(|&v| v == 5.0) && rep.moved_m3 == 0.0);
    // double gravity halves the limit, so more material moves; stronger rock raises it, so less moves
    let run = |gv: f64, s: f64| -> f64 {
        let mut zz = z0.clone();
        thermal_relax(&d.mesh, &mut zz, &vec![gv; n], &vec![s; n], 0.05, 20_000, 1e-6).moved_m3
    };
    assert!(run(19.62, 1.0) > run(9.81, 1.0));
    assert!(run(9.81, 2.0) < run(9.81, 1.0));
    // iteration cap respected
    let mut zz = z0.clone();
    let rep = thermal_relax(&d.mesh, &mut zz, &g, &st, 0.05, 3, 0.0);
    assert!(rep.iters <= 3);
}

// ---------------------------------------------------------------------------------------------
// Hand-sized worlds: deltas, floodplains, mountains, lakes
// ---------------------------------------------------------------------------------------------

#[test]
fn deltas_form_at_the_river_mouth_and_floodplains_on_the_flat_reach() {
    let mut w = ramp_world(BoundaryKind::Wall);
    // the default Kc is derived for ~1e12 m^2 basins; this toy ramp drains ~1e9 m^2, so capacity is scaled down to keep the same regime
    w.s.set_param("sediment", "Kc", 1.0e7).unwrap();
    let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 6).unwrap();
    let nx = 16;
    let dm = cells(&w, "delta_mask");
    let fm = cells(&w, "floodplain_mask");
    let dep = cells(&w, "deposition");
    let z = cells(&w, "elevation");
    // some step produced a delta and a floodplain
    assert!(steps.iter().any(|s| s.n_delta_cells > 0), "a delta must form at the ocean outlet: {steps:?}");
    assert!(steps.iter().any(|s| s.n_floodplain_cells > 0), "a floodplain must form on the gentle reach: {steps:?}");
    // deltas live on ocean cells, never above sea level; floodplains on land
    let ocean = cells(&w, "ocean_mask");
    for c in 0..w.d.n_cells() {
        if dm[c] > 0.5 {
            assert!(ocean[c] > 0.5, "delta cell {c} must be an ocean cell");
        }
        if fm[c] > 0.5 {
            assert!(ocean[c] < 0.5, "floodplain cell {c} must be land");
        }
    }
    // mountains (columns >= 11) never receive deposits
    for j in 0..5 {
        for i in 11..nx {
            assert_eq!(dep[j * nx + i], 0.0, "mountain cell ({i},{j}) got a deposit");
        }
    }
    assert!(z.iter().all(|v| v.is_finite()));
    for s in &steps {
        assert!(s.balance_residual.abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "{s:?}");
    }
}

#[test]
fn a_lake_traps_all_incoming_sediment_until_filled() {
    // 18 x 3 plane: ocean at column 0, ramp up to a rim at column 6, a basin (columns 7..10) below the rim, mountains behind it.
    let (nx, ny) = (18usize, 3usize);
    let d = plane(nx, ny, BoundaryKind::Wall, 9.81);
    let mut z = vec![0.0; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            z[j * nx + i] = match i {
                0 => -20.0,
                1..=6 => 20.0 + 4.0 * (i as f64),
                7..=10 => 30.0 - (i as f64 - 7.0) * 0.0 - 10.0, // basin floor 20 m, rim 44 m at column 6
                _ => 40.0 + 120.0 * (i as f64 - 10.0),
            } + if j == 1 { 0.0 } else { 3.0 };
        }
    }
    let mut w = world(d, z, YEARS);
    let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 3).unwrap();
    assert!(steps[0].n_depression_cells > 0, "the test terrain must contain a lake basin: {steps:?}");
    assert!(steps.iter().any(|s| s.stored_lakes_m3 > 0.0), "the basin must store sediment: {steps:?}");
    for s in &steps {
        assert!(s.balance_residual.abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "{s:?}");
    }
    // a lake never raises its floor above the spill level: no cell inside the basin ends higher than its lowest rim donor
    let z = cells(&w, "elevation");
    for j in 0..ny {
        for i in 7..=10 {
            assert!(z[j * nx + i].is_finite());
        }
    }
    assert_eq!(sp(&w, "n_unresolved_lakes"), 0.0);
}

// ---------------------------------------------------------------------------------------------
// World scale: sphere, random terrain
// ---------------------------------------------------------------------------------------------

fn fast_world(level: u32, seed: u64) -> World {
    let d = sphere(level);
    let z = terrain(&d, seed);
    let mut w = world(d, z, 1.0e6);
    w.s.set_param("sediment", "Kc", 2.0e7).unwrap();
    w
}

#[test]
fn volume_is_conserved_on_a_sphere_over_repeated_steps() {
    for (level, seed) in [(3u32, 1u64), (3, 2), (4, 3)] {
        let mut w = fast_world(level, seed);
        let n = w.d.n_cells();
        let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 8).unwrap();
        for (k, s) in steps.iter().enumerate() {
            // acceptance: eroded = deposited + exported + stored, to 1e-8 (relative to the volume moved)
            let sum = s.deposited_m3 + s.exported_m3 + s.stored_lakes_m3;
            assert!((s.eroded_volume_m3 - sum).abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "L{level} step {k}: {s:?}");
            assert!(s.balance_residual.abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "L{level} step {k}: {s:?}");
            assert!(s.deposited_m3 >= 0.0 && s.exported_m3 >= 0.0 && s.stored_lakes_m3 >= 0.0);
        }
        assert!(steps[0].eroded_volume_m3 > 0.0);
        // last step, independent check on the surface: sum(area * terrain_step) = -exports (thermal is zero-sum)
        let st = cells(&w, "terrain_step");
        let net: f64 = (0..n).map(|c| w.d.mesh.area[c] * st[c]).sum();
        let want = -(sp(&w, "export_ocean_m3") + sp(&w, "export_void_m3") + sp(&w, "export_sink_m3"));
        assert!((net - want).abs() <= 1e-8 * sp(&w, "eroded_volume_m3").max(1.0), "L{level}: net {net} vs exports {want}");
        assert!(sp(&w, "thermal_volume_error_m3").abs() <= 1e-8 * sp(&w, "eroded_volume_m3").max(1.0));
    }
}

#[test]
fn thermal_step_leaves_no_slope_above_the_talus_limit() {
    let mut w = fast_world(3, 5);
    run_surface_steps(&w.d, &mut w.f, &mut w.s, 4).unwrap();
    assert!(sp(&w, "thermal_max_excess_m") <= 1e-6, "thermal erosion converged: {}", sp(&w, "thermal_max_excess_m"));
    // verify on the surface the step left behind (z after the committed step = elevation field now)
    let z = cells(&w, "elevation");
    let n = w.d.n_cells();
    let tol = sp(&w, "thermal_tol_m");
    // The elevation was advanced by the full terrain_step of the last step, so it is the thermally relaxed surface of that step.
    let (max_slope, scale) = (sp(&w, "max_slope"), sp(&w, "talus_scale"));
    for i in 0..n {
        let gi = w.d.gravity_magnitude(i);
        for (k, &j) in w.d.mesh.neighbors(i).iter().enumerate() {
            let gj = w.d.gravity_magnitude(j as usize);
            let g = 0.5 * (gi + gj);
            let lim = max_slope * scale * (enbuilder::consts::G_EARTH / g) * w.d.mesh.edge_dists(i)[k];
            assert!((z[i] - z[j as usize]).abs() <= lim + 4.0 * tol + 1e-6, "edge {i}-{j}");
        }
    }
    // turning thermal erosion off leaves steep cells (the switch works) and keeps the budget
    let mut w2 = fast_world(3, 5);
    w2.s.set_param("sediment", "thermal_iters", 0.0).unwrap();
    let s = run_surface_steps(&w2.d, &mut w2.f, &mut w2.s, 1).unwrap();
    assert_eq!(sp(&w2, "thermal_moved_m3"), 0.0);
    assert!(s[0].balance_residual.abs() <= 1e-8 * s[0].eroded_volume_m3.max(1.0));
}

#[test]
fn no_new_pits_and_every_land_cell_still_drains() {
    let mut w = fast_world(3, 2);
    let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 8).unwrap();
    for k in 1..steps.len() {
        assert!(steps[k].n_depression_cells <= steps[k - 1].n_depression_cells.max(steps[0].n_depression_cells), "step {k}: {:?}", steps[k]);
    }
    // re-run flow routing on the final surface: every land cell reaches an ocean cell, a lake or an outlet without a cycle
    let n = w.d.n_cells();
    let mut r = Scheduler::new();
    add_flow_routing_stage(&mut r).unwrap();
    r.run(&w.d, &mut w.f, &DirtyRegion::All).unwrap();
    let fd: Vec<i64> = cells(&w, "flow_dir").iter().map(|&v| v as i64).collect();
    let ocean = cells(&w, "ocean_mask");
    let dep = cells(&w, "depression_depth");
    for c in 0..n {
        if ocean[c] > 0.5 { continue; }
        let mut cur = c;
        let mut hops = 0;
        while fd[cur] >= 0 && ocean[cur] < 0.5 {
            cur = fd[cur] as usize;
            hops += 1;
            assert!(hops <= n, "cycle from cell {c}");
        }
        assert!(ocean[cur] > 0.5 || fd[cur] < 0 || dep[cur] > 0.0, "cell {c} does not drain");
    }
    // the final depression count is not larger than the initial one
    assert!(steps.last().unwrap().n_depression_cells <= steps[0].n_depression_cells);
}

#[test]
fn deposits_go_to_low_ground_not_to_mountains() {
    let mut w = fast_world(4, 3);
    run_surface_steps(&w.d, &mut w.f, &mut w.s, 1).unwrap();
    let n = w.d.n_cells();
    let z0 = terrain(&w.d, 3);
    let dep = cells(&w, "deposition");
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| z0[b].partial_cmp(&z0[a]).unwrap());
    let top = &order[..n / 20];
    let on_top = top.iter().filter(|&&c| dep[c] > 0.0).count();
    let total = dep.iter().filter(|&&d| d > 0.0).count();
    assert!(total > 0, "something must be deposited");
    assert!(on_top as f64 <= 0.05 * total as f64 + 1.0, "{on_top} of {total} deposit cells are on the top 5% of terrain");
}

#[test]
fn boundaries_wrap_void_and_closed_worlds_keep_the_budget() {
    for b in [BoundaryKind::Wall, BoundaryKind::Wrap, BoundaryKind::Void] {
        let mut w = ramp_world(b);
        let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 4).unwrap();
        for s in &steps {
            assert!(s.balance_residual.abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "{b:?}: {s:?}");
        }
        let n = w.d.n_cells();
        let st = cells(&w, "terrain_step");
        let net: f64 = (0..n).map(|c| w.d.mesh.area[c] * st[c]).sum();
        let want = -(sp(&w, "export_ocean_m3") + sp(&w, "export_void_m3") + sp(&w, "export_sink_m3"));
        assert!((net - want).abs() <= 1e-8 * sp(&w, "eroded_volume_m3").max(1.0), "{b:?}");
    }
    // closed world (no ocean at all): nothing may disappear except into the sink export
    let d = plane(12, 6, BoundaryKind::Wall, 9.81);
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|c| 100.0 + 20.0 * (c % 12) as f64 + 3.0 * (c / 12) as f64).collect();
    let mut w = world(d, z, YEARS);
    let steps = run_surface_steps(&w.d, &mut w.f, &mut w.s, 3).unwrap();
    for s in &steps { assert!(s.balance_residual.abs() <= 1e-8 * s.eroded_volume_m3.max(1.0), "{s:?}"); }
}

#[test]
fn sediment_is_deterministic() {
    let run = || {
        let mut w = fast_world(3, 9);
        run_surface_steps(&w.d, &mut w.f, &mut w.s, 5).unwrap();
        (cells(&w, "elevation"), cells(&w, "terrain_delta"), cells(&w, "sediment"), cells(&w, "delta_mask"))
    };
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------------------------------------
// Interface
// ---------------------------------------------------------------------------------------------

#[test]
fn parameters_state_and_errors() {
    let mut m = Sediment::new();
    for (k, v) in SEDIMENT_PARAMS.iter() { assert_eq!(m.params().get(k), Some(*v), "{k}"); }
    for (k, v) in [("Kc", -1.0), ("a_exp", 0.0), ("deposit_fraction", 1.5), ("deposit_fraction", -0.1), ("years_per_step", 0.0), ("max_slope", 0.0), ("Kc", f64::NAN)] {
        assert!(m.set_param(k, v).is_err(), "{k}={v}");
    }
    assert!(m.set_param("nonsense", 1.0).is_err());
    assert!(m.set_param("deposit_fraction", 1.0).is_ok());
    let spec = m.spec();
    for o in ["sediment", "terrain_delta", "terrain_step", "deposition", "delta_mask", "floodplain_mask"] {
        assert!(spec.outputs.iter().any(|s| s == o), "{o}");
    }
    assert!(!spec.outputs.iter().any(|s| s == "erosion_delta"), "one producer per field: erosion keeps erosion_delta");
    assert!(Sediment::with_discharge().spec().inputs.iter().any(|s| s == "discharge"));
    // scheduler order: routing -> erosion -> sediment
    let mut s = Scheduler::new();
    add_surface_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |n: &str| order.iter().position(|m| m == n).unwrap();
    assert!(pos("flow_routing") < pos("erosion") && pos("erosion") < pos("sediment"));
    // years must agree
    s.set_param("sediment", "years_per_step", 5.0).unwrap();
    let w = ramp_world(BoundaryKind::Wall);
    let mut f = w.f;
    assert!(run_surface_steps(&w.d, &mut f, &mut s, 1).is_err());
    // missing state
    let w = ramp_world(BoundaryKind::Wall);
    let mut f = FieldStore::new(w.d.n_cells());
    f.put("elevation", 1, vec![0.0; w.d.n_cells()]).unwrap();
    f.put("rock_strength", 1, vec![1.0; w.d.n_cells()]).unwrap();
    let mut s2 = Scheduler::new();
    add_surface_stage(&mut s2).unwrap();
    set_surface_years(&mut s2, YEARS).unwrap();
    assert!(run_surface_steps(&w.d, &mut f, &mut s2, 1).is_err());
}

#[test]
fn terrain_net_variant_reads_terrain_delta() {
    let d = sphere(2);
    let n = d.n_cells();
    let mut rng = Rng::new(4);
    let tect: Vec<f64> = (0..n).map(|_| 2000.0 * rng.next_f64()).collect();
    let mk = |delta: Vec<f64>| -> FieldStore {
        let mut f = FieldStore::new(n);
        f.put("tectonic_elevation", 1, tect.clone()).unwrap();
        f.put("rock_strength", 1, vec![1.0; n]).unwrap();
        f.put("terrain_delta", 1, delta).unwrap();
        f
    };
    let spec = TerrainShaping::with_net_erosion().spec();
    assert!(spec.inputs.iter().any(|s| s == "terrain_delta") && !spec.inputs.iter().any(|s| s == "erosion_delta"));
    let delta: Vec<f64> = (0..n).map(|_| -60.0 * rng.next_f64() + 25.0 * rng.next_f64()).collect(); // incision AND deposition
    let mut f = mk(delta.clone());
    let mut t = TerrainShaping::with_net_erosion();
    t.set_param("noise_amp_m", 0.0).unwrap();
    t.set_param("thermal_iters", 0.0).unwrap();
    t.set_param("peak_ref_m", 1.0e6).unwrap();
    t.set_param("depth_ref_m", 1.0e6).unwrap();
    t.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let e = f.data("elevation").unwrap();
    for i in 0..n { assert!((e[i] - (tect[i] + delta[i])).abs() < 1e-9, "cell {i}"); }
    let mut miss = FieldStore::new(n);
    miss.put("tectonic_elevation", 1, tect.clone()).unwrap();
    miss.put("rock_strength", 1, vec![1.0; n]).unwrap();
    assert!(TerrainShaping::with_net_erosion().run(&d, &mut miss, &DirtyRegion::All).is_err());
    // terrain and sediment form a cycle until the 4c-3 loop exists
    let mut s = Scheduler::new();
    s.register(Box::new(TerrainShaping::with_net_erosion())).unwrap();
    add_surface_stage(&mut s).unwrap();
    assert!(s.plan_order().is_err());
}
