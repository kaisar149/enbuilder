//! Part 4b-2 acceptance tests: runoff, discharge, rivers, fill-and-spill lakes, salt flats, water conservation.
//! On sphere, cube, walled / wrapped / void plane and disc. NOT compiled when written (no toolchain); see ARCHITECTURE.md.
//! Hand-built expectations were checked with `reference/mirror_4b2_cases.py`; random-world invariants with `reference/mirror_4b2.py`.
use enbuilder::generators::*;
use enbuilder::math::{length, sub};
use enbuilder::rivers_lakes::*;
use enbuilder::rng::Rng;
use enbuilder::*;
use std::collections::BTreeMap;

const R: f64 = 6.371e6;
const CELL: f64 = 10_000.0;
const RHO: f64 = 1000.0;

fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn rect(nx: usize, ny: usize, b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: CELL, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn disc(b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Disc { diameter_cells: 30 }, cell_size: CELL, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// Same terrain as `tests/part4b1.rs` / `terrain()` in `reference/mirror_4b1.py`.
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

/// Precipitation (kg/m^2/s) then evaporation, in this draw order: same as `fields_for` in `reference/mirror_4b2.py`.
fn fields_for(n: usize, seed: u64, p0: f64, e0: f64) -> (Vec<f64>, Vec<f64>) {
    let mut rng = Rng::new(seed);
    let p: Vec<f64> = (0..n).map(|_| p0 * (0.5 + rng.next_f64())).collect();
    let e: Vec<f64> = (0..n).map(|_| e0 * (0.5 + rng.next_f64())).collect();
    (p, e)
}

fn uni(n: usize, v: f64) -> Vec<f64> { vec![v; n] }
fn close(a: f64, b: f64, rel: f64) -> bool { (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-12) }

/// `params` are (module, name, value): "rivers_lakes" or "flow_routing".
fn setup(d: &Domain, z: Vec<f64>, p: Vec<f64>, e: Vec<f64>, params: &[(&str, &str, f64)]) -> (Scheduler, FieldStore) {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z).unwrap();
    f.put("precipitation", 1, p).unwrap();
    f.put("evaporation", 1, e).unwrap();
    let mut s = Scheduler::new();
    add_rivers_lakes_stage(&mut s).unwrap();
    for (m, k, v) in params {
        s.set_param(m, k, *v).unwrap();
    }
    (s, f)
}

fn go(d: &Domain, z: Vec<f64>, p: Vec<f64>, e: Vec<f64>, params: &[(&str, &str, f64)]) -> (Scheduler, FieldStore) {
    let (mut s, mut f) = setup(d, z, p, e, params);
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    (s, f)
}

fn par(s: &Scheduler, k: &str) -> f64 { s.module_param("rivers_lakes", k).unwrap() }

/// Hand-built plane: ocean column i = 0 at -10 m, land 100 m, listed cells overridden. Cell id = j * nx + i.
fn hand(nx: usize, ny: usize, cells: &[((usize, usize), f64)]) -> (Domain, Vec<f64>) {
    let d = rect(nx, ny, BoundaryKind::Wall);
    let mut z = vec![100.0; nx * ny];
    for j in 0..ny { z[j * nx] = -10.0; }
    for &((i, j), v) in cells { z[j * nx + i] = v; }
    (d, z)
}

/// Every structural property of a result. `precip` is the input precipitation.
fn check_hydro(name: &str, d: &Domain, z: &[f64], precip: &[f64], f: &FieldStore, s: &Scheduler, coeff: f64, thr: f64, min_depth: f64) {
    let n = d.n_cells();
    let area = &d.mesh.area;
    let ocean = f.data("ocean_mask").unwrap();
    let fd: Vec<i64> = f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect();
    let filled = f.data("elevation_filled").unwrap();
    let dep = f.data("depression_depth").unwrap();
    let runoff = f.data("runoff").unwrap();
    let disch = f.data("discharge").unwrap();
    let river = f.data("river_mask").unwrap();
    let lid = f.data("lake_id").unwrap();
    let ldep = f.data("lake_depth").unwrap();
    let lvol = f.data("lake_volume").unwrap();
    let lout = f.data("lake_outflow").unwrap();
    let salt = f.data("salt_flat").unwrap();

    // ---- water conservation, recomputed independently of the module's bookkeeping
    let mut pl = 0.0;
    for c in 0..n {
        if ocean[c] <= 0.5 { pl += precip[c].max(0.0) * area[c] / RHO; }
    }
    let tol = 1e-8 * pl.max(1e-12);
    assert!(close(par(s, "precip_land_m3s"), pl, 1e-9), "{name}: precip_land");
    assert!((par(s, "land_loss_m3s") - (1.0 - coeff) * pl).abs() <= tol, "{name}: land_loss");
    let (levap, oq, vq, sq) = (par(s, "lake_evap_m3s"), par(s, "ocean_discharge_m3s"), par(s, "void_outflow_m3s"), par(s, "sink_inflow_m3s"));
    assert!(levap >= 0.0 && oq >= 0.0 && vq >= 0.0 && sq >= 0.0, "{name}: negative flux");
    let resid = pl - (1.0 - coeff) * pl - levap - oq - vq - sq;
    assert!(resid.abs() <= tol, "{name}: land precipitation != loss + lake evap + ocean + void + sink (residual {resid} of {pl})");
    assert!(par(s, "balance_residual").abs() <= 1e-8, "{name}: reported residual {}", par(s, "balance_residual"));
    assert_eq!(par(s, "lake_storage_change_m3s"), 0.0);
    // everything that reaches a root = runoff - lake evaporation (independent of the ocean/void/sink split)
    let roots: f64 = (0..n).filter(|&c| fd[c] < 0).map(|c| disch[c]).sum();
    assert!((roots - (coeff * pl - levap)).abs() <= tol, "{name}: root discharge {roots} vs {}", coeff * pl - levap);

    // ---- runoff
    for c in 0..n {
        let want = if ocean[c] > 0.5 { 0.0 } else { coeff * precip[c].max(0.0) };
        assert!((runoff[c] - want).abs() <= 1e-15 + 1e-12 * want, "{name}: runoff at {c}");
        assert!(disch[c] >= 0.0 && disch[c].is_finite(), "{name}: discharge at {c}");
    }

    // ---- lakes appear exactly in the 4b-1 depressions; never above the spill height
    let mut units: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for c in 0..n {
        assert_eq!(lid[c] >= 0.0, dep[c] > 0.0, "{name}: lake_id must be set exactly on depression cells ({c})");
        assert!(ldep[c] >= 0.0 && ldep[c] <= dep[c] + 1e-9, "{name}: lake_depth {} beyond depression depth {} at {c}", ldep[c], dep[c]);
        assert!(z[c] + ldep[c] <= filled[c] + 1e-9, "{name}: lake surface above spill height at {c}");
        assert!((lvol[c] - area[c] * ldep[c]).abs() <= 1e-9 * (area[c] * ldep[c]).max(1.0), "{name}: lake_volume at {c}");
        if lid[c] >= 0.0 {
            units.entry(lid[c] as i64).or_default().push(c);
        } else {
            assert!(ldep[c] == 0.0 && lout[c] == 0.0 && salt[c] == 0.0, "{name}: lake fields set outside a lake at {c}");
        }
        if salt[c] > 0.5 {
            assert!(lid[c] >= 0.0 && ldep[c] == 0.0 && lout[c] == 0.0, "{name}: salt flat must be a dry cell of a non-spilling lake ({c})");
        }
    }
    assert_eq!(units.len() as f64, par(s, "n_lakes"), "{name}: n_lakes");
    let (mut n_spill, mut n_wet_endo) = (0usize, 0usize);
    for (id, cells) in &units {
        let level = filled[cells[0]];
        let out = lout[cells[0]];
        for &c in cells {
            assert_eq!(filled[c], level, "{name}: lake {id} spans two spill heights");
            assert_eq!(lout[c], out, "{name}: lake {id} has two outflows");
            // every neighbour outside the lake is at or above the spill height (the lake cannot leak below its rim)
            for &j in d.mesh.neighbors(c) {
                let j = j as usize;
                if lid[j] != lid[c] { assert!(filled[j] >= level, "{name}: lake {id} has a neighbour {j} below its spill height"); }
            }
        }
        // exits spill over rim cells that sit exactly at the spill height
        let exits: Vec<usize> = cells.iter().copied().filter(|&c| fd[c] >= 0 && lid[fd[c] as usize] != lid[c]).collect();
        assert!(!exits.is_empty(), "{name}: lake {id} has no exit");
        for &c in &exits {
            let r = fd[c] as usize;
            assert!(filled[r] == level && dep[r] == 0.0, "{name}: lake {id} spills at {r} which is not a rim cell at the spill height");
        }
        if out > 0.0 {
            n_spill += 1;
            for &c in cells {
                let want = if dep[c] < min_depth { 0.0 } else { dep[c] };
                assert!((ldep[c] - want).abs() <= 1e-9, "{name}: spilling lake {id} must be full (cell {c})");
            }
        } else {
            // endorheic or dry: wet cells share one water level at or below the spill height
            let wet: Vec<usize> = cells.iter().copied().filter(|&c| ldep[c] > 0.0).collect();
            if !wet.is_empty() {
                n_wet_endo += 1;
                let h = z[wet[0]] + ldep[wet[0]];
                for &c in &wet { assert!((z[c] + ldep[c] - h).abs() <= 1e-9 * h.abs().max(1.0), "{name}: lake {id} has a tilted surface"); }
                assert!(h <= level + 1e-9);
            }
        }
    }
    assert_eq!(par(s, "n_lakes_spilling") as usize, n_spill, "{name}: n_lakes_spilling");
    let counted = par(s, "n_lakes_spilling") + par(s, "n_lakes_endorheic") + par(s, "n_lakes_dry") + par(s, "n_unresolved_lakes");
    assert_eq!(counted, par(s, "n_lakes"), "{name}: lake classes must add up");
    assert!(n_wet_endo as f64 <= par(s, "n_lakes_endorheic") + par(s, "n_lakes_dry"));

    // ---- rivers: complete above the threshold, never on lakes or ocean, and gap-free downstream
    for c in 0..n {
        if river[c] > 0.5 {
            assert!(ocean[c] <= 0.5 && lid[c] < 0.0 && disch[c] > 0.0, "{name}: bad river cell {c}");
        } else if ocean[c] <= 0.5 && lid[c] < 0.0 {
            assert!(disch[c] <= thr, "{name}: cell {c} carries {} > threshold but is not a river", disch[c]);
        }
    }
    for c in 0..n {
        if river[c] <= 0.5 { continue; }
        let (mut x, mut steps) = (fd[c], 0usize);
        while x >= 0 {
            let xu = x as usize;
            steps += 1;
            assert!(steps <= n, "{name}: runaway river path from {c}");
            if lid[xu] >= 0.0 {
                if lout[xu] <= 0.0 { break; } // the river ends in a lake that does not spill
            } else if ocean[xu] > 0.5 {
                break;
            } else {
                assert!(river[xu] > 0.5 || disch[xu] == 0.0, "{name}: gap in the river from {c} at {xu}");
            }
            x = fd[xu];
        }
    }
    assert_eq!(par(s, "river_cells"), river.iter().filter(|&&r| r > 0.5).count() as f64, "{name}: river_cells");
}

#[test]
fn solve_lake_pure_numbers() {
    // three cells at 0, 10, 20 m below a 40 m spill height, 1 m^3/s evaporation each when wet (1e8 m^2 * 1e-8 m/s)
    let (z, a, ev) = ([0.0, 10.0, 20.0], [1e8; 3], [1e-8; 3]);
    let s = solve_lake(40.0, &z, &a, &ev, 0.0);
    assert_eq!(s.depth, vec![0.0; 3]);
    assert!(s.q_out == 0.0 && s.evap == 0.0 && !s.full);
    let s = solve_lake(40.0, &z, &a, &ev, 0.5);
    assert_eq!(s.depth, vec![0.0; 3]);
    assert!((s.evap - 0.5).abs() < 1e-12 && s.q_out == 0.0 && !s.full);
    let s = solve_lake(40.0, &z, &a, &ev, 1.5);
    assert_eq!(s.depth, vec![10.0, 0.0, 0.0], "one cell wet, level = floor of the next cell");
    assert!((s.evap - 1.5).abs() < 1e-12 && !s.full);
    let s = solve_lake(40.0, &z, &a, &ev, 2.5);
    assert_eq!(s.depth, vec![20.0, 10.0, 0.0]);
    assert!(!s.full);
    let s = solve_lake(40.0, &z, &a, &ev, 5.0);
    assert_eq!(s.depth, vec![40.0, 30.0, 20.0], "full to the spill height");
    assert!(s.full && (s.q_out - 2.0).abs() < 1e-12 && (s.evap - 3.0).abs() < 1e-12);
    // conservation for any inflow
    for q in [0.0, 0.3, 1.0, 1.7, 2.9, 3.1, 8.0, 1e3] {
        let s = solve_lake(40.0, &z, &a, &ev, q);
        assert!((s.q_out + s.evap - q).abs() < 1e-12, "q_in {q}");
        assert!(s.depth.iter().zip(z.iter()).all(|(d, zz)| *d >= 0.0 && zz + d <= 40.0 + 1e-12));
    }
    // unsorted floors: the lowest cell is wetted first whatever its index
    let s = solve_lake(40.0, &[20.0, 0.0, 10.0], &a, &ev, 1.5);
    assert_eq!(s.depth, vec![0.0, 10.0, 0.0]);
}

#[test]
fn lake_units_are_merged_basins_of_equal_spill_height() {
    let d = rect(6, 3, BoundaryKind::Wall);
    let n = d.n_cells();
    let (mut filled, mut depth) = (vec![0.0; n], vec![0.0; n]);
    for (c, f, dd) in [(7usize, 60.0, 5.0), (8, 60.0, 9.0), (9, 50.0, 3.0), (10, 50.0, 4.0), (15, 60.0, 2.0)] {
        filled[c] = f;
        depth[c] = dd;
    }
    let (id, units) = lake_units(&d, &filled, &depth);
    assert_eq!(units, vec![vec![7, 8], vec![9, 10], vec![15]], "cells 8 and 9 touch but have different spill heights; 15 touches only 9");
    for c in 0..n {
        let want = match c { 7 | 8 => 0, 9 | 10 => 1, 15 => 2, _ => -1 };
        assert_eq!(id[c], want, "cell {c}");
    }
    // equal height and touching merges them
    filled[9] = 60.0;
    filled[10] = 60.0;
    let (_, units) = lake_units(&d, &filled, &depth);
    assert_eq!(units, vec![vec![7, 8, 9, 10, 15]]);
}

#[test]
fn bowl_fills_to_its_lowest_rim_and_spills_there() {
    // 9 x 7 walled plane, ocean column 0; bowl (4,3) = 20 m and (5,3) = 30 m; channel (3,3) = 60, (2,3) = 40, (1,3) = 30. Spill height 60.
    let (nx, ny) = (9usize, 7usize);
    let id = |i: usize, j: usize| j * nx + i;
    let (d, z) = hand(nx, ny, &[((4, 3), 20.0), ((5, 3), 30.0), ((3, 3), 60.0), ((2, 3), 40.0), ((1, 3), 30.0)]);
    let n = d.n_cells();
    let p = uni(n, 1e-4); // runoff 4 m^3/s per land cell, precipitation 10 m^3/s per land cell
    let (s, f) = go(&d, z.clone(), p.clone(), uni(n, 1e-5), &[]);
    check_hydro("bowl_wet", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    let lid = f.data("lake_id").unwrap();
    let ldep = f.data("lake_depth").unwrap();
    let lout = f.data("lake_outflow").unwrap();
    let disch = f.data("discharge").unwrap();
    let river = f.data("river_mask").unwrap();
    let fd = f.data("flow_dir").unwrap();
    let lake_cells: Vec<usize> = (0..n).filter(|&c| lid[c] >= 0.0).collect();
    assert_eq!(lake_cells, vec![id(4, 3), id(5, 3)], "lakes appear exactly in the depression");
    assert!((ldep[id(4, 3)] - 40.0).abs() < 1e-9 && (ldep[id(5, 3)] - 30.0).abs() < 1e-9, "full to 60 m");
    for c in &lake_cells { assert!(z[*c] + ldep[*c] <= 60.0 + 1e-9); }
    // inflow from the independent drainage area of the lake outlet cell: 35 land cells * 4 m^3/s = 140; full-surface evaporation 2 * 3 = 6
    let acc = f.data("drainage_area").unwrap()[id(4, 3)];
    let qin = 0.4 * 1e-4 * acc / RHO;
    assert!((qin - 140.0).abs() < 1e-6);
    assert!((lout[id(4, 3)] - (qin - 6.0)).abs() < 1e-6, "outflow = inflow - evaporation, got {}", lout[id(4, 3)]);
    assert!((par(&s, "lake_evap_m3s") - 6.0).abs() < 1e-9);
    assert!((par(&s, "precip_land_m3s") - 560.0).abs() < 1e-6);
    assert!((par(&s, "land_loss_m3s") - 336.0).abs() < 1e-6);
    assert!((par(&s, "ocean_discharge_m3s") - 218.0).abs() < 1e-6);
    assert_eq!(par(&s, "n_lakes"), 1.0);
    assert_eq!(par(&s, "n_lakes_spilling"), 1.0);
    // it leaves over the lowest rim (3,3) and runs down the channel to the ocean column
    assert_eq!(fd[id(4, 3)], id(3, 3) as f64);
    assert_eq!(fd[id(3, 3)], id(2, 3) as f64);
    assert!(disch[id(3, 3)] >= lout[id(4, 3)] && (disch[id(3, 3)] - 158.0).abs() < 1e-6);
    assert!((disch[id(1, 3)] - 178.0).abs() < 1e-6);
    // rivers: exactly the channel, carried through the lake without a gap; none on the lake itself
    let rivers: Vec<usize> = (0..n).filter(|&c| river[c] > 0.5).collect();
    assert_eq!(rivers, vec![id(1, 3), id(2, 3), id(3, 3)]);
    assert_eq!(par(&s, "river_cells"), 3.0);
}

#[test]
fn dry_basin_stays_dry_and_a_starved_lake_is_partial() {
    let (nx, ny) = (9usize, 7usize);
    let id = |i: usize, j: usize| j * nx + i;
    let (d, z) = hand(nx, ny, &[((4, 3), 20.0), ((5, 3), 30.0), ((3, 3), 60.0), ((2, 3), 40.0), ((1, 3), 30.0)]);
    let n = d.n_cells();
    // no rain: dry lake, both cells are salt flats, nothing flows
    let p0 = uni(n, 0.0);
    let (s, f) = go(&d, z.clone(), p0.clone(), uni(n, 1e-5), &[]);
    check_hydro("bowl_norain", &d, &z, &p0, &f, &s, 0.4, 100.0, 0.5);
    assert!(f.data("lake_depth").unwrap().iter().all(|&v| v == 0.0));
    assert!(f.data("discharge").unwrap().iter().all(|&v| v == 0.0));
    let salt = f.data("salt_flat").unwrap();
    assert_eq!((0..n).filter(|&c| salt[c] > 0.5).collect::<Vec<_>>(), vec![id(4, 3), id(5, 3)]);
    assert_eq!(par(&s, "n_lakes_dry"), 1.0);
    assert_eq!(par(&s, "river_cells"), 0.0);
    // weak rain: inflow 4.2 m^3/s against 3 m^3/s evaporation per wet cell: only the 20 m cell is wet, level = the 30 m floor
    let p1 = uni(n, 3e-6);
    let (s, f) = go(&d, z.clone(), p1.clone(), uni(n, 1e-5), &[]);
    check_hydro("bowl_weak", &d, &z, &p1, &f, &s, 0.4, 100.0, 0.5);
    let ldep = f.data("lake_depth").unwrap();
    let salt = f.data("salt_flat").unwrap();
    assert!((ldep[id(4, 3)] - 10.0).abs() < 1e-9, "wet cell depth {}", ldep[id(4, 3)]);
    assert_eq!(ldep[id(5, 3)], 0.0);
    assert_eq!(salt[id(4, 3)], 0.0);
    assert_eq!(salt[id(5, 3)], 1.0, "the dry shore of an endorheic lake is a salt flat");
    assert_eq!(f.data("lake_outflow").unwrap()[id(4, 3)], 0.0);
    assert!((par(&s, "lake_evap_m3s") - 4.2).abs() < 1e-9, "all inflow evaporates");
    assert!((par(&s, "ocean_discharge_m3s") - 2.52).abs() < 1e-9);
    assert_eq!(par(&s, "n_lakes_endorheic"), 1.0);
    assert_eq!(par(&s, "n_lakes_spilling"), 0.0);
    // a wet climate fills and overflows the very same basin
    let p2 = uni(n, 1e-4);
    let (s, f) = go(&d, z.clone(), p2.clone(), uni(n, 1e-5), &[]);
    assert_eq!(par(&s, "n_lakes_spilling"), 1.0);
    assert!(f.data("salt_flat").unwrap().iter().all(|&v| v == 0.0));
    // min_lake_depth_m hides shallow water: with 11 m minimum the 10 m puddle disappears and becomes salt flat too
    let (s, f) = go(&d, z.clone(), p1, uni(n, 1e-5), &[("rivers_lakes", "min_lake_depth_m", 11.0)]);
    assert_eq!(f.data("lake_depth").unwrap()[id(4, 3)], 0.0);
    assert_eq!(f.data("salt_flat").unwrap()[id(4, 3)], 1.0);
    assert_eq!(par(&s, "lake_area_m2"), 0.0);
}

#[test]
fn adjacent_basins_cascade_or_merge_by_spill_height() {
    let (nx, ny) = (9usize, 7usize);
    let id = |i: usize, j: usize| j * nx + i;
    // Cascade: A (2,3) = 20 spills west over (1,3) = 30; B (4,3) = 20 is separated by a ridge (3,3) = 50 and fills to 50, spilling into A.
    let (d, z) = hand(nx, ny, &[((1, 3), 30.0), ((2, 3), 20.0), ((3, 3), 50.0), ((4, 3), 20.0)]);
    let n = d.n_cells();
    let p = uni(n, 1e-4);
    let (s, f) = go(&d, z.clone(), p.clone(), uni(n, 1e-5), &[]);
    check_hydro("cascade", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    let (lid, ldep, lout, fd) = (f.data("lake_id").unwrap(), f.data("lake_depth").unwrap(), f.data("lake_outflow").unwrap(), f.data("flow_dir").unwrap());
    assert_eq!(par(&s, "n_lakes"), 2.0);
    assert_eq!(par(&s, "n_lakes_spilling"), 2.0);
    assert_ne!(lid[id(2, 3)], lid[id(4, 3)]);
    assert_eq!(lid[id(3, 3)], -1.0, "the ridge is the spill rim of B, not a lake cell");
    assert!((ldep[id(2, 3)] - 10.0).abs() < 1e-9 && (ldep[id(4, 3)] - 30.0).abs() < 1e-9);
    assert_eq!(fd[id(4, 3)], id(3, 3) as f64);
    assert_eq!(fd[id(3, 3)], id(2, 3) as f64);
    assert!(lout[id(2, 3)] > lout[id(4, 3)], "A receives all of B's outflow plus its own catchment");
    // Merge: ridge lowered to 25 m, below A's 30 m rim: both basins fill to 30 m and are ONE lake of three cells
    let (d, z) = hand(nx, ny, &[((1, 3), 30.0), ((2, 3), 20.0), ((3, 3), 25.0), ((4, 3), 20.0)]);
    let (s, f) = go(&d, z.clone(), p.clone(), uni(n, 1e-5), &[]);
    check_hydro("merge", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    let (lid, ldep) = (f.data("lake_id").unwrap(), f.data("lake_depth").unwrap());
    assert_eq!(par(&s, "n_lakes"), 1.0);
    assert!(lid[id(2, 3)] == lid[id(3, 3)] && lid[id(3, 3)] == lid[id(4, 3)] && lid[id(2, 3)] >= 0.0);
    assert!((ldep[id(2, 3)] - 10.0).abs() < 1e-9 && (ldep[id(3, 3)] - 5.0).abs() < 1e-9 && (ldep[id(4, 3)] - 10.0).abs() < 1e-9);
    for c in [id(2, 3), id(3, 3), id(4, 3)] { assert!((z[c] + ldep[c] - 30.0).abs() < 1e-9, "one common water level"); }
}

#[test]
fn rivers_follow_the_threshold_on_a_tilted_plane() {
    // 20 x 10, ocean column 0, every land row drains west: discharge(i) = 4 * (20 - i) with 4 m^3/s of runoff per cell
    let (nx, ny) = (20usize, 10usize);
    let d = rect(nx, ny, BoundaryKind::Wall);
    let z: Vec<f64> = (0..nx * ny).map(|c| if c % nx == 0 { -40.0 } else { 10.0 + 50.0 * (c % nx) as f64 }).collect();
    let n = nx * ny;
    let p = uni(n, 1e-4);
    let (s, f) = go(&d, z.clone(), p.clone(), uni(n, 1e-5), &[("rivers_lakes", "river_threshold_m3s", 42.0)]);
    check_hydro("tilted", &d, &z, &p, &f, &s, 0.4, 42.0, 0.5);
    let (disch, river) = (f.data("discharge").unwrap(), f.data("river_mask").unwrap());
    for j in 0..ny {
        for i in 1..nx {
            let c = j * nx + i;
            assert!((disch[c] - 4.0 * (nx - i) as f64).abs() < 1e-9, "discharge at ({i},{j})");
            assert_eq!(river[c] > 0.5, i <= 9, "river at ({i},{j}): 4 * (20 - i) > 42 exactly for i <= 9");
        }
        assert_eq!(river[j * nx], 0.0, "ocean is never river");
    }
    assert_eq!(par(&s, "river_cells"), 90.0);
    assert_eq!(par(&s, "n_lakes"), 0.0);
    assert!((par(&s, "ocean_discharge_m3s") - 0.4 * par(&s, "precip_land_m3s")).abs() < 1e-9, "no lakes: all runoff reaches the ocean");
    // a lower threshold only adds river cells
    let (s2, _) = go(&d, z, p, uni(n, 1e-5), &[("rivers_lakes", "river_threshold_m3s", 10.0)]);
    assert!(par(&s2, "river_cells") > 90.0);
}

#[test]
fn water_is_conserved_and_rules_hold_on_all_topologies() {
    // (name, domain, terrain seed, field seed, +offset that turns the whole world into land)
    let worlds: Vec<(&str, Domain, u64, u64, f64)> = vec![
        ("sphere3", sphere(3), 11, 31, 0.0),
        ("sphere4", sphere(4), 11, 31, 0.0),
        ("cube", cube(), 12, 32, 0.0),
        ("plane_wall", rect(40, 24, BoundaryKind::Wall), 13, 33, 0.0),
        ("plane_wrap", rect(40, 24, BoundaryKind::Wrap), 13, 33, 0.0),
        ("plane_void", rect(40, 24, BoundaryKind::Void), 14, 34, 1000.0),
        ("disc_wall", disc(BoundaryKind::Wall), 15, 35, 0.0),
    ];
    for (name, d, tseed, fseed, off) in worlds {
        let n = d.n_cells();
        let z: Vec<f64> = terrain(&d, tseed).iter().map(|v| v + off).collect();
        let (p, e) = fields_for(n, fseed, 3e-5, 1e-5);
        let (s, f) = go(&d, z.clone(), p.clone(), e, &[]);
        check_hydro(name, &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
        assert!(par(&s, "n_lakes") > 0.0, "{name}: the jittered terrain must hold lakes");
        assert!(par(&s, "lake_evap_m3s") > 0.0, "{name}");
        if name != "disc_wall" { assert_eq!(par(&s, "n_unresolved_lakes"), 0.0, "{name}"); }
        if name == "plane_void" {
            assert_eq!(par(&s, "ocean_discharge_m3s"), 0.0);
            assert!(par(&s, "void_outflow_m3s") > 0.0, "{name}: water leaves over the Void edge");
            assert_eq!(par(&s, "sink_inflow_m3s"), 0.0);
        } else {
            assert!(par(&s, "ocean_discharge_m3s") > 0.0, "{name}");
            assert_eq!(par(&s, "void_outflow_m3s"), 0.0, "{name}");
        }
    }
}

#[test]
fn closed_world_collects_everything_in_its_terminal_sink() {
    let d = rect(40, 24, BoundaryKind::Wall);
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|i| 1000.0 + ((i * 37) % 101) as f64).collect();
    let (p, e) = fields_for(n, 35, 3e-5, 1e-5);
    let (s, f) = go(&d, z.clone(), p.clone(), e, &[]);
    check_hydro("closed", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    assert_eq!(par(&s, "ocean_discharge_m3s"), 0.0);
    assert!(par(&s, "sink_inflow_m3s") > 0.0, "the one terminal sink receives the surplus");
}

#[test]
fn wet_dry_and_parameter_extremes() {
    let d = rect(40, 24, BoundaryKind::Wrap);
    let n = d.n_cells();
    let z = terrain(&d, 13);
    // wet climate: even the smallest catchment out-supplies the evaporation, so every lake spills
    let (p, e) = fields_for(n, 33, 3e-4, 1e-5);
    let (s, f) = go(&d, z.clone(), p.clone(), e, &[]);
    check_hydro("wet", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    assert!(par(&s, "n_lakes") > 0.0);
    assert_eq!(par(&s, "n_lakes_spilling"), par(&s, "n_lakes"));
    assert!(f.data("salt_flat").unwrap().iter().all(|&v| v == 0.0));
    // dry climate: no lake overflows, dry shores are salt flats
    let (p, e) = fields_for(n, 33, 3e-7, 1e-5);
    let (s, f) = go(&d, z.clone(), p.clone(), e, &[]);
    check_hydro("dry", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    assert_eq!(par(&s, "n_lakes_spilling"), 0.0);
    assert!(f.data("salt_flat").unwrap().iter().any(|&v| v > 0.5));
    // no open-water evaporation: every drop of runoff reaches the ocean
    let (p, e) = fields_for(n, 33, 3e-5, 1e-5);
    let (s, f) = go(&d, z.clone(), p.clone(), e.clone(), &[("rivers_lakes", "lake_evap_factor", 0.0)]);
    check_hydro("no_evap", &d, &z, &p, &f, &s, 0.4, 100.0, 0.5);
    assert_eq!(par(&s, "lake_evap_m3s"), 0.0);
    assert_eq!(par(&s, "n_lakes_spilling"), par(&s, "n_lakes"));
    assert!(close(par(&s, "ocean_discharge_m3s"), 0.4 * par(&s, "precip_land_m3s"), 1e-9));
    // runoff_coeff = 0: no rivers, no lakes with inflow; = 1: nothing is lost on land
    let (s, f) = go(&d, z.clone(), p.clone(), e.clone(), &[("rivers_lakes", "runoff_coeff", 0.0)]);
    check_hydro("coeff0", &d, &z, &p, &f, &s, 0.0, 100.0, 0.5);
    assert!(f.data("discharge").unwrap().iter().all(|&v| v == 0.0));
    assert_eq!(par(&s, "river_cells"), 0.0);
    assert_eq!(par(&s, "ocean_discharge_m3s"), 0.0);
    let (s, f) = go(&d, z.clone(), p.clone(), e.clone(), &[("rivers_lakes", "runoff_coeff", 1.0)]);
    check_hydro("coeff1", &d, &z, &p, &f, &s, 1.0, 100.0, 0.5);
    assert_eq!(par(&s, "land_loss_m3s"), 0.0);
    // a bigger runoff coefficient never lowers ocean discharge
    let (s_lo, _) = go(&d, z.clone(), p.clone(), e.clone(), &[("rivers_lakes", "runoff_coeff", 0.2)]);
    let (s_hi, _) = go(&d, z, p, e, &[("rivers_lakes", "runoff_coeff", 0.6)]);
    assert!(par(&s_hi, "ocean_discharge_m3s") > par(&s_lo, "ocean_discharge_m3s"));
}

#[test]
fn results_are_deterministic_and_negative_rain_is_ignored() {
    let d = sphere(4);
    let n = d.n_cells();
    let z = terrain(&d, 21);
    let (p, e) = fields_for(n, 41, 3e-5, 1e-5);
    let (_, a) = go(&d, z.clone(), p.clone(), e.clone(), &[]);
    let (_, b) = go(&d, z.clone(), p.clone(), e.clone(), &[]);
    for name in ["runoff", "discharge", "river_mask", "lake_id", "lake_depth", "lake_volume", "lake_outflow", "salt_flat"] {
        assert_eq!(a.data(name).unwrap(), b.data(name).unwrap(), "{name} differs between runs");
    }
    // negative precipitation (a numerical undershoot upstream) is treated as zero on land
    let mut pn = p.clone();
    for c in (0..n).step_by(5) { pn[c] = -1.0; }
    let (s, f) = go(&d, z.clone(), pn.clone(), e, &[]);
    check_hydro("negative_rain", &d, &z, &pn, &f, &s, 0.4, 100.0, 0.5);
    assert!(f.data("runoff").unwrap().iter().all(|&v| v >= 0.0));
}

#[test]
fn bad_inputs_and_interface() {
    let mut m = RiversLakes::new();
    assert_eq!(m.params().get("runoff_coeff"), Some(0.4));
    assert_eq!(m.params().get("river_threshold_m3s"), Some(100.0));
    assert_eq!(m.params().get("lake_evap_factor"), Some(3.0));
    assert_eq!(m.params().get("min_lake_depth_m"), Some(0.5));
    assert_eq!(m.params().get("rho_water"), Some(1000.0));
    assert!(m.set_param("runoff_coeff", 1.5).is_err());
    assert!(m.set_param("runoff_coeff", -0.1).is_err());
    assert!(m.set_param("river_threshold_m3s", -1.0).is_err());
    assert!(m.set_param("lake_evap_factor", f64::NAN).is_err());
    assert!(m.set_param("min_lake_depth_m", -1.0).is_err());
    assert!(m.set_param("rho_water", 0.0).is_err());
    assert!(m.set_param("n_lakes", 3.0).is_err(), "output-only parameters are not settable");
    assert!(m.set_param("nonsense", 1.0).is_err());
    assert!(m.set_param("runoff_coeff", 0.25).is_ok());
    assert_eq!(m.params().get("runoff_coeff"), Some(0.25));
    let spec = m.spec();
    assert_eq!(spec.name, "rivers_lakes");
    assert_eq!(spec.inputs.len(), 8);
    for need in ["elevation", "ocean_mask", "elevation_filled", "depression_depth", "flow_dir", "flow_order", "precipitation", "evaporation"] {
        assert!(spec.inputs.iter().any(|s| s == need), "input {need}");
    }
    assert_eq!(spec.outputs.len(), 8);
    for need in ["runoff", "discharge", "river_mask", "lake_id", "lake_depth", "lake_volume", "lake_outflow", "salt_flat"] {
        assert!(spec.outputs.iter().any(|s| s == need), "output {need}");
    }
    // scheduler order: sea_level -> ocean_mask -> flow_routing -> rivers_lakes
    let mut s = Scheduler::new();
    add_rivers_lakes_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |n: &str| order.iter().position(|m| m == n).unwrap();
    assert!(pos("sea_level") < pos("ocean_mask") && pos("ocean_mask") < pos("flow_routing") && pos("flow_routing") < pos("rivers_lakes"));
    // the module alone needs the 4b-1 fields from somewhere else
    let d = rect(10, 6, BoundaryKind::Wall);
    let mut s2 = Scheduler::new();
    add_rivers_lakes_module(&mut s2).unwrap();
    let mut f = FieldStore::new(60);
    f.put("elevation", 1, vec![1.0; 60]).unwrap();
    f.put("precipitation", 1, vec![1e-5; 60]).unwrap();
    f.put("evaporation", 1, vec![1e-5; 60]).unwrap();
    assert!(s2.run(&d, &mut f, &DirtyRegion::All).is_err());
    // NaN rain or evaporation is an error, not silent garbage
    let z: Vec<f64> = (0..60).map(|c| if c % 10 == 0 { -10.0 } else { 100.0 + c as f64 }).collect();
    let mut p = vec![1e-5; 60];
    p[33] = f64::NAN;
    let (mut s3, mut f3) = setup(&d, z.clone(), p, vec![1e-5; 60], &[]);
    assert!(s3.run(&d, &mut f3, &DirtyRegion::All).is_err());
    let mut e = vec![1e-5; 60];
    e[12] = f64::INFINITY;
    let (mut s4, mut f4) = setup(&d, z, vec![1e-5; 60], e, &[]);
    assert!(s4.run(&d, &mut f4, &DirtyRegion::All).is_err());
}
