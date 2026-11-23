//! Part 6a: dirty regions, incremental re-solve, locks. (Diff and explainer are in tests/part6b.rs.)
use enbuilder::dirty::{extend_downwind, grow_region, plan_dirty};
use enbuilder::generators::{generate_cube, generate_plane, generate_sphere, CubeParams, PlaneParams, PlaneShape, SphereParams};
use enbuilder::toyclimate::{toy_fields, toy_scheduler};
use enbuilder::*;

const NX: usize = 40;
const NY: usize = 30;
const DX: f64 = 1000.0;

fn plane(boundary: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: NX, ny: NY }, cell_size: DX, boundary, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn cell(i: usize, j: usize) -> u32 { (j * NX + i) as u32 }
fn block(i0: usize, j0: usize, w: usize, h: usize) -> Vec<u32> { (j0..j0 + h).flat_map(|j| (i0..i0 + w).map(move |i| cell(i, j))).collect() }

fn solved_toy() -> (Domain, FieldStore, Scheduler) {
    let d = plane(BoundaryKind::Wall);
    let mut f = toy_fields(d.n_cells()).unwrap();
    let mut s = toy_scheduler(NX, false).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    (d, f, s)
}
fn bits_equal(a: &[f64], b: &[f64], c: usize) -> bool { a[c].to_bits() == b[c].to_bits() }

// ------------------------------------------------------------------ regions

#[test]
fn grow_region_on_a_plane_is_the_manhattan_ball_and_zero_radius_is_the_seeds() {
    let d = plane(BoundaryKind::Wall);
    let seed = cell(20, 15);
    assert_eq!(grow_region(&d, &[seed], 0.0).cells, vec![seed]);
    for k in 1..=5usize {
        let r = grow_region(&d, &[seed], k as f64 * DX + 1.0);
        for j in 0..NY {
            for i in 0..NX {
                let man = (i as i64 - 20).unsigned_abs() as usize + (j as i64 - 15).unsigned_abs() as usize;
                let c = cell(i, j) as usize;
                assert_eq!(r.contains(c), man <= k, "cell ({i},{j}) k={k}");
                if man <= k {
                    assert!((r.dist[c] - man as f64 * DX).abs() < 1e-9);
                }
            }
        }
    }
}

#[test]
fn regions_are_nested_sorted_and_work_on_sphere_and_cube() {
    let s = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: 3, surface_gravity: 9.81, seed: 1 }).unwrap();
    let c = generate_cube(&CubeParams { half_side: 1.0e6, cells_per_edge: 8, surface_gravity: 9.81, seed: 1 }).unwrap();
    for d in [s, c] {
        let seeds = [3u32, 11];
        let mut prev = grow_region(&d, &seeds, 0.0);
        let h = d.mesh.edge_dists(0).iter().cloned().fold(0.0, f64::max);
        for k in 1..6 {
            let r = grow_region(&d, &seeds, k as f64 * h);
            assert!(r.cells.windows(2).all(|w| w[0] < w[1]));
            assert!(prev.cells.iter().all(|&c| r.contains(c as usize)), "nested");
            assert!(r.len() >= prev.len());
            assert!(seeds.iter().all(|&s| r.dist[s as usize] == 0.0));
            prev = r;
        }
        assert!(prev.len() > seeds.len());
        let all = grow_region(&d, &seeds, 1e12);
        assert!(all.is_all());
        assert!(all.border(&d).is_empty(), "the whole mesh has no border");
    }
}

#[test]
fn border_cells_have_an_outside_neighbour() {
    let d = plane(BoundaryKind::Wall);
    let r = grow_region(&d, &[cell(20, 15)], 3.0 * DX + 1.0);
    let b = r.border(&d);
    assert!(!b.is_empty());
    for &c in &b {
        assert!(d.mesh.neighbors(c as usize).iter().any(|&j| !r.contains(j as usize)));
    }
    // cells strictly inside (Manhattan <= 2) are not border
    assert!(!b.contains(&cell(20, 15)) && !b.contains(&cell(21, 15)));
}

#[test]
fn downstream_closure_follows_the_module_graph() {
    let s = toy_scheduler(NX, false).unwrap();
    let up = downstream_fields(s.specs(), &["surface"]);
    assert_eq!(up.iter().cloned().collect::<Vec<_>>(), vec!["albedo", "humidity", "surface", "temperature"]);
    let up = downstream_fields(s.specs(), &["lake_source"]);
    assert_eq!(up.iter().cloned().collect::<Vec<_>>(), vec!["lake_source", "moisture"]);
    assert!(!downstream_fields(s.specs(), &["surface"]).contains("rock_hardness"));
}

#[test]
fn the_downwind_extension_follows_the_wind_cone_only() {
    let d = plane(BoundaryKind::Wall);
    let n = d.n_cells();
    let base = grow_region(&d, &block(10, 14, 2, 2), 0.0);
    let mut east = vec![0.0; 3 * n];
    for c in 0..n {
        east[3 * c] = 5.0;
    }
    let r = extend_downwind(&d, &base, &east, 12.0 * DX, 0.5, 0.1);
    assert!(r.contains(cell(20, 14) as usize), "east of the patch within 12 cells");
    assert!(r.contains(cell(20, 15) as usize));
    assert!(!r.contains(cell(30, 14) as usize), "beyond the carry length");
    assert!(!r.contains(cell(5, 14) as usize), "nothing upwind");
    assert!(!r.contains(cell(11, 25) as usize), "nothing sideways far away (cone 60 degrees)");
    // no wind, tiny wind, wrong length -> unchanged
    let calm = vec![0.0; 3 * n];
    assert_eq!(extend_downwind(&d, &base, &calm, 12.0 * DX, 0.5, 0.1), base);
    assert_eq!(extend_downwind(&d, &base, &east[..3], 12.0 * DX, 0.5, 0.1), base);
    assert_eq!(extend_downwind(&d, &base, &east, 0.0, 0.5, 0.1), base);
}

#[test]
fn plan_dirty_gives_each_field_its_own_radius_and_the_edited_field_only_the_seeds() {
    let (d, f, s) = solved_toy();
    let spec = InfluenceSpec::uniform(2.0 * DX + 1.0).with_field("humidity", 5.0 * DX + 1.0);
    let seeds = block(20, 15, 1, 1);
    let p = plan_dirty(&d, &f, s.specs(), &["surface"], &seeds, &spec, 1.0);
    assert_eq!(p.per_field["surface"].cells, seeds);
    assert!(p.per_field["albedo"].len() < p.per_field["humidity"].len());
    assert_eq!(p.per_field["albedo"].len(), 13, "Manhattan ball of radius 2 has 13 cells");
    assert!(!p.per_field.contains_key("rock_hardness"));
    let wide = plan_dirty(&d, &f, s.specs(), &["surface"], &seeds, &spec, 2.0);
    assert!(wide.per_field["albedo"].len() > p.per_field["albedo"].len());
    assert!(p.union.len() >= p.per_field["humidity"].len());
}

// ------------------------------------------------------------------ the acceptance tests

#[test]
fn a_patch_in_a_desert_changes_nearby_fields_and_leaves_far_fields_bit_identical() {
    let (d, mut f, mut s) = solved_toy();
    let before: Vec<(String, Vec<f64>)> = f.names().into_iter().map(|n| (n.clone(), f.data(&n).unwrap().to_vec())).collect();
    let patch = block(17, 12, 7, 7);
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(6.0 * DX + 1.0));
    let rep = resolve_edit(&mut s, &d, &mut f, &Edit::set("ice patch", "surface", &patch, 1.0), &cfg).unwrap();
    assert!(rep.commit.is_some());
    // near: temperature drops (albedo 0.7 -> T_eq = 272 vs 292)
    let (t0, t1) = (&before.iter().find(|(n, _)| n == "temperature").unwrap().1, f.data("temperature").unwrap());
    let centre = cell(20, 15) as usize;
    assert!(t1[centre] < t0[centre] - 5.0, "{} -> {}", t0[centre], t1[centre]);
    assert!(t1[cell(27, 15) as usize] < t0[cell(27, 15) as usize] - 0.5, "4 cells beyond the patch edge still cooled");
    let h1 = f.data("humidity").unwrap();
    let h0 = &before.iter().find(|(n, _)| n == "humidity").unwrap().1;
    assert!(h1[centre] < h0[centre]);
    // far: everything outside the regions is bit-identical
    for (name, old) in &before {
        let new = f.data(name).unwrap();
        match rep.regions.get(name) {
            Some(r) => {
                for c in 0..d.n_cells() {
                    if !r.contains(c) {
                        assert!(bits_equal(old, new, c), "{name} cell {c} changed outside its region");
                    }
                }
            }
            None => assert_eq!(old, &new.to_vec(), "{name} is not downstream of the edit and must not change"),
        }
    }
    assert!(rep.region_cells < d.n_cells(), "the region is a proper subset of the mesh");
    assert!(rep.field("temperature").unwrap().changed_cells > 20);
    // far corner cell, explicitly
    let far = cell(2, 2) as usize;
    assert!(bits_equal(&t0, t1, far));
}

#[test]
fn the_incremental_result_matches_a_full_solve_within_the_stated_tolerance() {
    let (d, f0, mut s) = solved_toy();
    let patch = block(17, 12, 7, 7);
    let edit = Edit::set("ice patch", "surface", &patch, 1.0);
    let mut full = f0.fork();
    solve_full(&mut s, &d, &mut full, &edit).unwrap();
    let mut inc = f0.fork();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(4.0 * DX + 1.0)); // deliberately small: the solver must widen
    let rep = resolve_edit(&mut s, &d, &mut inc, &edit, &cfg).unwrap();
    for name in ["temperature", "humidity", "albedo"] {
        let (a, b) = (full.data(name).unwrap(), inc.data(name).unwrap());
        let err = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
        let fr = rep.field(name).unwrap();
        assert!(err <= cfg.border_tol * fr.peak_change * 1.0001 + 1e-9, "{name}: error {err} vs tol*peak {}", cfg.border_tol * fr.peak_change);
        // the reported truncation error is the true error (the solve is global, so it is known)
        assert!((err - fr.truncation_error).abs() < 1e-9, "{name}: reported {} true {}", fr.truncation_error, err);
    }
    assert!(rep.field("temperature").unwrap().widenings >= 1, "radius 4 cells is too small for a decay length of ~3 cells and 1% border tolerance");
    assert!(rep.field("temperature").unwrap().peak_change > 5.0);
    assert!(rep.max_truncation_error() < 0.01 * 30.0);
}

#[test]
fn a_zero_tolerance_widens_to_the_whole_mesh_and_equals_the_full_solve_bit_for_bit() {
    let (d, f0, mut s) = solved_toy();
    let edit = Edit::set("ice patch", "surface", &block(17, 12, 7, 7), 1.0);
    let mut full = f0.fork();
    solve_full(&mut s, &d, &mut full, &edit).unwrap();
    let mut inc = f0.fork();
    let mut cfg = IncrementalConfig::new(InfluenceSpec::uniform(3.0 * DX));
    cfg.border_tol = 0.0;
    cfg.max_widen = 30;
    let rep = resolve_edit(&mut s, &d, &mut inc, &edit, &cfg).unwrap();
    assert!(rep.field("temperature").unwrap().whole_mesh);
    for name in ["temperature", "humidity", "albedo", "surface"] {
        assert_eq!(full.data(name).unwrap(), inc.data(name).unwrap(), "{name}");
    }
}

#[test]
fn widening_reduces_the_truncation_error() {
    let (d, f0, mut s) = solved_toy();
    let edit = Edit::set("ice patch", "surface", &block(17, 12, 7, 7), 1.0);
    let mut errs = Vec::new();
    for max_widen in [0usize, 3, 12] {
        let mut f = f0.fork();
        let mut cfg = IncrementalConfig::new(InfluenceSpec::uniform(2.0 * DX));
        cfg.max_widen = max_widen;
        cfg.far_field_guard = false; // study the widening alone
        let rep = resolve_edit(&mut s, &d, &mut f, &edit, &cfg).unwrap();
        errs.push(rep.field("temperature").unwrap().truncation_error);
    }
    assert!(errs[0] > errs[1] && errs[1] > errs[2], "{errs:?}");
    assert!(errs[0] > 1.0, "radius 2 cells cuts a ~3-cell-decay signal hard: {}", errs[0]);
}

#[test]
fn an_edit_far_downwind_of_nothing_changes_nothing_that_is_unrelated() {
    // the unrelated chain (bedrock -> rock_hardness) is not even in the affected set
    let (d, mut f, mut s) = solved_toy();
    let rh = f.data("rock_hardness").unwrap().to_vec();
    let v = f.get("rock_hardness").unwrap().version;
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(6.0 * DX));
    let rep = resolve_edit(&mut s, &d, &mut f, &Edit::set("x", "surface", &[cell(5, 5)], 1.0), &cfg).unwrap();
    assert!(!rep.affected_fields.contains(&"rock_hardness".to_string()));
    assert_eq!(f.data("rock_hardness").unwrap(), rh.as_slice());
    assert_eq!(f.get("rock_hardness").unwrap().version, v, "not even a commit entry");
}

#[test]
fn wind_carries_the_effect_downwind_so_the_region_follows_it() {
    let (d, f0, mut s) = solved_toy();
    s.set_param("plume", "decay", 0.7).unwrap();
    let mut f0 = f0;
    s.run(&d, &mut f0, &DirtyRegion::All).unwrap();
    let src = block(8, 14, 2, 2);
    let edit = Edit::set("spring", "lake_source", &src, 1.0);
    let mut full = f0.fork();
    solve_full(&mut s, &d, &mut full, &edit).unwrap();
    let plume_far = cell(14, 15) as usize;
    assert!(full.data("moisture").unwrap()[plume_far] > 0.1, "the plume reaches 5 cells downwind");

    let mut run = |downwind: f64, max_widen: usize| {
        let mut f = f0.fork();
        let mut cfg = IncrementalConfig::new(InfluenceSpec::uniform(3.0 * DX).with_downwind("moisture", downwind));
        cfg.max_widen = max_widen;
        cfg.far_field_guard = false; // study the downwind carry alone
        let rep = resolve_edit(&mut s, &d, &mut f, &edit, &cfg).unwrap();
        (f, rep)
    };
    // without the wind carry and without widening the plume is cut off
    let (f_cut, r_cut) = run(0.0, 0);
    assert!(f_cut.data("moisture").unwrap()[plume_far] == 0.0);
    assert!(r_cut.field("moisture").unwrap().truncation_error > 0.1);
    // with a 20-cell downwind carry the plume is inside the region from the start
    let (f_w, r_w) = run(20.0 * DX, 0);
    assert!(r_w.regions["moisture"].contains(plume_far));
    assert!(!r_w.regions["moisture"].contains(cell(1, 15) as usize), "nothing upwind");
    assert!(f_w.data("moisture").unwrap()[plume_far] > 0.1);
    assert!(r_w.field("moisture").unwrap().truncation_error < 0.05, "{}", r_w.field("moisture").unwrap().truncation_error);
}

// ------------------------------------------------------------------ locks

#[test]
fn locked_cells_never_change_through_a_resolve_and_effects_go_around_them() {
    let (d, f0, mut s) = solved_toy();
    let edit = Edit::set("ice patch", "surface", &block(17, 12, 7, 7), 1.0);
    // lock a block between the patch and a probe cell, at a value the physics would not give
    let wall = block(24, 12, 2, 7);
    let mut f = f0.fork();
    f.write("temperature", |t| {
        for &c in &wall {
            t[c as usize] = 250.0;
        }
    })
    .unwrap();
    f.lock_cells("temperature", &wall).unwrap();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(10.0 * DX));
    resolve_edit(&mut s, &d, &mut f, &edit, &cfg).unwrap();
    for &c in &wall {
        assert_eq!(f.data("temperature").unwrap()[c as usize], 250.0, "locked cell {c}");
    }
    // effects propagate AROUND the lock: a probe beyond/above the block still cools relative to the unedited state
    let probe = cell(25, 21) as usize; // above the 7-cell-tall block, beside the patch's row range
    let (t_before, t_after) = (f0.data("temperature").unwrap()[probe], f.data("temperature").unwrap()[probe]);
    assert!((t_before - t_after).abs() > 1e-3, "probe moved: {t_before} -> {t_after}");
    // and the lock acts as a boundary condition: a cell beside the locked block is pulled toward 250 relative to an unlocked run
    let mut g = f0.fork();
    resolve_edit(&mut s, &d, &mut g, &edit, &cfg).unwrap();
    let beside = cell(26, 15) as usize;
    assert!(f.data("temperature").unwrap()[beside] < g.data("temperature").unwrap()[beside] - 0.5, "the cold lock must cool its neighbour");
}

#[test]
fn a_pinned_edit_survives_the_physics_that_would_overwrite_it() {
    let (d, mut f, mut s) = solved_toy();
    let patch = block(17, 12, 7, 7);
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(8.0 * DX));
    // albedo is PRODUCED by `surface_albedo` (surface is 0 everywhere). Painting it unpinned: the producer wins.
    resolve_edit(&mut s, &d, &mut f, &Edit::set("paint", "albedo", &patch, 0.9), &cfg).unwrap();
    assert!((f.data("albedo").unwrap()[cell(20, 15) as usize] - 0.2).abs() < 1e-12, "unpinned paint is recomputed by the producer");
    // pinned: the override sticks and the temperature below responds
    let t_before = f.data("temperature").unwrap()[cell(20, 15) as usize];
    resolve_edit(&mut s, &d, &mut f, &Edit::set("paint", "albedo", &patch, 0.9).pinned(), &cfg).unwrap();
    assert_eq!(f.data("albedo").unwrap()[cell(20, 15) as usize], 0.9);
    assert!(f.is_locked("albedo", cell(20, 15) as usize));
    assert!(f.data("temperature").unwrap()[cell(20, 15) as usize] < t_before - 8.0);
    // a later full run cannot change it either
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert_eq!(f.data("albedo").unwrap()[cell(20, 15) as usize], 0.9);
    // unlock and re-run: the producer takes over again
    f.unlock_field("albedo");
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!((f.data("albedo").unwrap()[cell(20, 15) as usize] - 0.2).abs() < 1e-12);
}

#[test]
fn set_cells_bypasses_locks_and_moves_the_pin() {
    let d = plane(BoundaryKind::Wall);
    let mut f = FieldStore::new(d.n_cells());
    f.put("x", 1, vec![1.0; d.n_cells()]).unwrap();
    f.lock_cells("x", &[3, 4]).unwrap();
    f.write("x", |v| v.iter_mut().for_each(|a| *a = 9.0)).unwrap();
    assert_eq!(&f.data("x").unwrap()[3..5], &[1.0, 1.0], "module writes cannot touch locked cells");
    assert_eq!(f.data("x").unwrap()[5], 9.0);
    f.put("x", 1, vec![7.0; d.n_cells()]).unwrap();
    assert_eq!(&f.data("x").unwrap()[3..5], &[1.0, 1.0], "put is also restored");
    f.set_cells("x", &[3], &[2.5], false).unwrap();
    assert_eq!(f.data("x").unwrap()[3], 2.5);
    f.write("x", |v| v.iter_mut().for_each(|a| *a = 0.0)).unwrap();
    assert_eq!(f.data("x").unwrap()[3], 2.5, "the pin moved to the new value");
    assert_eq!(f.locked_cells("x"), vec![3, 4]);
    f.unlock_cells("x", &[3]);
    assert_eq!(f.locked_cells("x"), vec![4]);
    assert!(f.set_cells("x", &[0], &[1.0, 2.0], false).is_err());
    assert!(f.set_cells("x", &[d.n_cells() as u32], &[1.0], false).is_err());
    assert!(f.set_cells("x", &[0], &[f64::NAN], false).is_err());
    assert!(f.lock_cells("nope", &[0]).is_err());
    // layered fields
    f.put("v", 3, vec![0.0; 3 * d.n_cells()]).unwrap();
    f.set_cells("v", &[2], &[1.0, 2.0, 3.0], true).unwrap();
    f.write("v", |a| a.iter_mut().for_each(|x| *x = 5.0)).unwrap();
    assert_eq!(&f.data("v").unwrap()[6..9], &[1.0, 2.0, 3.0]);
}

#[test]
fn fork_copies_fields_and_locks_but_not_history() {
    let d = plane(BoundaryKind::Wall);
    let mut f = FieldStore::new(d.n_cells());
    f.put("x", 1, vec![1.0; d.n_cells()]).unwrap();
    f.lock_cells("x", &[0]).unwrap();
    let mut g = f.fork();
    assert!(!g.can_undo());
    assert_eq!(g.locked_cells("x"), vec![0]);
    g.write("x", |v| v[1] = 5.0).unwrap();
    assert_eq!(f.data("x").unwrap()[1], 1.0, "independent storage");
    assert!(g.current_version() >= f.current_version());
}

// ------------------------------------------------------------------ undo, errors, validation

#[test]
fn a_resolved_edit_is_one_undo_step_that_restores_every_field_exactly() {
    let (d, mut f, mut s) = solved_toy();
    let snap: Vec<(String, Vec<f64>)> = f.names().into_iter().map(|n| (n.clone(), f.data(&n).unwrap().to_vec())).collect();
    let v0 = f.current_version();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(6.0 * DX));
    let rep = resolve_edit(&mut s, &d, &mut f, &Edit::set("ice patch", "surface", &block(17, 12, 7, 7), 1.0), &cfg).unwrap();
    assert_eq!(rep.commit, Some(v0 + 1), "one commit for the edit and its consequences");
    assert_ne!(f.data("temperature").unwrap(), snap.iter().find(|(n, _)| n == "temperature").unwrap().1.as_slice());
    f.undo().unwrap();
    for (n, v) in &snap {
        assert_eq!(f.data(n).unwrap(), v.as_slice(), "{n}");
    }
}

#[test]
fn a_failing_solve_rolls_the_edit_back() {
    let d = plane(BoundaryKind::Wall);
    let mut f = toy_fields(d.n_cells()).unwrap();
    let mut s = toy_scheduler(NX, true).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    s.set_param("bomb", "armed", 1.0).unwrap();
    let snap: Vec<(String, Vec<f64>)> = f.names().into_iter().map(|n| (n.clone(), f.data(&n).unwrap().to_vec())).collect();
    let v0 = f.current_version();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(6.0 * DX));
    assert!(resolve_edit(&mut s, &d, &mut f, &Edit::set("x", "surface", &[cell(5, 5)], 1.0), &cfg).is_err());
    assert_eq!(f.current_version(), v0);
    for (n, v) in &snap {
        assert_eq!(f.data(n).unwrap(), v.as_slice(), "{n} restored");
    }
}

#[test]
fn bad_edits_and_configs_are_rejected_before_anything_changes() {
    let (d, mut f, mut s) = solved_toy();
    let v0 = f.current_version();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(3.0 * DX));
    let bad = [
        Edit::set("no cells", "surface", &[], 1.0),
        Edit::set("no field", "nonexistent", &[1], 1.0),
        Edit::set("out of range", "surface", &[d.n_cells() as u32], 1.0),
        Edit::per_cell("wrong length", "surface", &[1, 2], vec![1.0, 2.0, 3.0]),
        Edit::set("nan", "surface", &[1], f64::NAN),
    ];
    for e in &bad {
        assert!(resolve_edit(&mut s, &d, &mut f, e, &cfg).is_err(), "{}", e.label);
        assert_eq!(f.current_version(), v0);
    }
    let mut c = cfg.clone();
    c.widen_factor = 1.0;
    assert!(resolve_edit(&mut s, &d, &mut f, &Edit::set("ok", "surface", &[1], 1.0), &c).is_err());
    let mut c = cfg.clone();
    c.border_tol = -1.0;
    assert!(c.validate().is_err());
    // per-cell values in the user's order
    let e = Edit::per_cell("two", "surface", &[cell(30, 5), cell(10, 5)], vec![0.25, 0.75]);
    resolve_edit(&mut s, &d, &mut f, &e, &cfg).unwrap();
    assert_eq!(f.data("surface").unwrap()[cell(30, 5) as usize], 0.25);
    assert_eq!(f.data("surface").unwrap()[cell(10, 5) as usize], 0.75);
}

#[test]
fn the_solver_is_deterministic() {
    let run = || {
        let (d, mut f, mut s) = solved_toy();
        let cfg = IncrementalConfig::new(InfluenceSpec::uniform(4.0 * DX));
        let rep = resolve_edit(&mut s, &d, &mut f, &Edit::set("p", "surface", &block(17, 12, 7, 7), 1.0), &cfg).unwrap();
        (f.data("temperature").unwrap().to_vec(), rep.region_cells, rep.field("temperature").unwrap().widenings)
    };
    assert_eq!(run(), run());
}

#[test]
fn works_on_wrapped_planes_across_the_seam() {
    let d = plane(BoundaryKind::Wrap);
    let mut f = toy_fields(d.n_cells()).unwrap();
    let mut s = toy_scheduler(NX, false).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let t0 = f.data("temperature").unwrap().to_vec();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(6.0 * DX));
    // patch on the left edge; the effect must show on the right edge through the wrap
    resolve_edit(&mut s, &d, &mut f, &Edit::set("seam", "surface", &block(0, 14, 2, 3), 1.0), &cfg).unwrap();
    let t1 = f.data("temperature").unwrap();
    assert!(t1[cell(NX - 1, 15) as usize] < t0[cell(NX - 1, 15) as usize] - 1.0, "the wrap carries the cooling to the other side");
    assert!((t1[cell(NX / 2, 15) as usize] - t0[cell(NX / 2, 15) as usize]).abs() < 0.05, "far side of the torus is (nearly) untouched");
}

// ------------------------------------------------------------------ real physics: energy stage with the ice-albedo loop

fn energy_world() -> (Domain, FieldStore, Scheduler, RunReport) {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    s.set_param("insolation", "season", 0.25).unwrap();
    let r = s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    (d, f, s, r)
}

#[test]
fn a_lake_in_the_real_energy_stage_matches_a_full_solve_and_warm_starts_the_loop() {
    let (d, f0, mut s, cold) = energy_world();
    let cold_passes: usize = cold.loops.iter().map(|l| l.iterations).sum();
    // the lake: land cells near the ridge axis dropped to 300 m below sea level (albedo_water, lapse change)
    let z = f0.data("elevation").unwrap();
    let centre = (0..d.n_cells()).filter(|&i| z[i] > 1500.0).max_by(|&a, &b| d.mesh.up[a][2].partial_cmp(&d.mesh.up[b][2]).unwrap()).unwrap();
    let lake = grow_region(&d, &[centre as u32], 1000e3).cells; // the cell and its ring of neighbours
    let edit = Edit::set("lake", "elevation", &lake, -300.0).pinned();
    let mut full = f0.fork();
    solve_full(&mut s, &d, &mut full, &edit).unwrap();
    let mut inc = f0.fork();
    let cfg = IncrementalConfig::new(InfluenceSpec::uniform(1500e3));
    let rep = resolve_edit(&mut s, &d, &mut inc, &edit, &cfg).unwrap();
    let warm_passes = rep.loop_passes();
    println!("cold loop passes {cold_passes}, warm {warm_passes}; region {} of {} cells", rep.region_cells, d.n_cells());
    for fr in &rep.fields { println!("{fr:?}"); }
    let t = rep.field("temperature").unwrap();
    assert!(t.peak_change > 0.5, "the lake changes the local temperature: {}", t.peak_change);
    // the global energy balance moves every cell by ~2 K; the far-field guard must have caught it (radius alone cannot)
    assert!(rep.field("temperature_sl").unwrap().far_field_added > 0, "{:?}", rep.field("temperature_sl"));
    for name in ["temperature", "temperature_sl", "albedo"] {
        let (a, b) = (full.data(name).unwrap(), inc.data(name).unwrap());
        let err = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
        let pk = rep.field(name).unwrap().peak_change;
        let allowed = (cfg.border_tol * pk).max(rep.field(name).unwrap().noise_abs);
        assert!(err <= allowed * 1.001 + 1e-6, "{name}: err {err}, allowed {allowed}");
    }
    // pinned: the lake stays a lake
    assert!(lake.iter().all(|&c| inc.data("elevation").unwrap()[c as usize] == -300.0));
}

#[test]
fn the_far_field_guard_catches_a_plume_the_radius_missed_and_keeps_the_tolerance() {
    let (d, mut f0, mut s) = solved_toy();
    s.set_param("plume", "decay", 0.7).unwrap();
    s.run(&d, &mut f0, &DirtyRegion::All).unwrap();
    let edit = Edit::set("spring", "lake_source", &block(8, 14, 2, 2), 1.0);
    let mut f = f0.fork();
    let mut cfg = IncrementalConfig::new(InfluenceSpec::uniform(3.0 * DX)); // no downwind carry, no widening: the radius misses the plume
    cfg.max_widen = 0;
    let rep = resolve_edit(&mut s, &d, &mut f, &edit, &cfg).unwrap();
    let m = rep.field("moisture").unwrap();
    assert!(m.far_field_added > 0, "{m:?}");
    assert!(m.truncation_error <= cfg.border_tol * m.peak_change + 1e-12, "guard bound: {m:?}");
    assert!(f.data("moisture").unwrap()[cell(14, 15) as usize] > 0.1, "the plume is committed");
    assert!(bits_equal(f0.data("moisture").unwrap(), f.data("moisture").unwrap(), cell(1, 1) as usize), "cells with no signal stay bit-identical");
}
