//! Part 4b-1 acceptance tests: priority-flood depression handling, flow direction, flow order, drainage area.
//! On sphere, cube, walled / wrapped / void plane and disc. NOT compiled when written (no toolchain); see ARCHITECTURE.md.
//! Hand-built expectations were checked with `reference/mirror_4b1_cases.py`; invariants with `reference/mirror_4b1.py`.
use enbuilder::flow_routing::*;
use enbuilder::generators::*;
use enbuilder::math::{length, sub};
use enbuilder::rng::Rng;
use enbuilder::*;

const R: f64 = 6.371e6;
const CELL: f64 = 10_000.0;
const CELL_AREA: f64 = CELL * CELL;

fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn rect(nx: usize, ny: usize, b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: CELL, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn disc(b: BoundaryKind) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Disc { diameter_cells: 30 }, cell_size: CELL, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// East-west ramp plus 300 m of per-cell jitter: about half ocean, many small depressions on land and near the coast.
/// Same formula as `terrain()` in `reference/mirror_4b1.py`.
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

fn build(d: &Domain, z: Vec<f64>, params: &[(&str, f64)]) -> (Scheduler, FieldStore) {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z).unwrap();
    let mut s = Scheduler::new();
    add_flow_routing_stage(&mut s).unwrap();
    for (k, v) in params {
        s.set_param("flow_routing", k, *v).unwrap();
    }
    (s, f)
}

fn route(d: &Domain, z: Vec<f64>, params: &[(&str, f64)]) -> (Scheduler, FieldStore) {
    let (mut s, mut f) = build(d, z, params);
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    (s, f)
}

fn fd_of(f: &FieldStore) -> Vec<i64> { f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect() }

/// Independent Planchon-Darboux fill (no epsilon) with the given roots; must equal `elevation_filled` exactly.
fn planchon_darboux(d: &Domain, z: &[f64], root: &[bool]) -> Vec<f64> {
    let n = d.n_cells();
    let mut w: Vec<f64> = (0..n).map(|i| if root[i] { z[i] } else { f64::INFINITY }).collect();
    let mut changed = true;
    let mut sweep = 0;
    while changed {
        changed = false;
        sweep += 1;
        for t in 0..n {
            let c = if sweep % 2 == 1 { t } else { n - 1 - t };
            if root[c] || w[c] <= z[c] {
                continue;
            }
            for &j in d.mesh.neighbors(c) {
                let j = j as usize;
                if z[c] >= w[j] {
                    w[c] = z[c];
                    changed = true;
                    break;
                }
                if w[c] > w[j] {
                    w[c] = w[j];
                    changed = true;
                }
            }
        }
    }
    w
}

/// Every structural property of a routing result; returns (number of roots, number of ocean cells).
fn check_routing(name: &str, d: &Domain, z: &[f64], f: &FieldStore, s: &Scheduler, void_outflow: bool) -> (usize, usize) {
    let n = d.n_cells();
    let fd = fd_of(f);
    let filled = f.data("elevation_filled").unwrap();
    let depth = f.data("depression_depth").unwrap();
    let order = f.data("flow_order").unwrap();
    let acc = f.data("drainage_area").unwrap();
    let mask = f.data("ocean_mask").unwrap();
    let void = void_outflow && matches!(d.boundary, BoundaryKind::Void);
    let n_sinks = s.module_param("flow_routing", "n_sinks").unwrap() as usize;

    // roots = ocean, Void edge, or terminal sinks; nothing else has flow_dir -1
    let mut roots = 0;
    let mut non_ocean_roots = 0;
    for c in 0..n {
        let is_outlet = mask[c] > 0.5 || (void && d.mesh.boundary_len[c] > 0.0);
        if fd[c] < 0 {
            roots += 1;
            if !is_outlet { non_ocean_roots += 1; }
        } else {
            assert!(!is_outlet, "{name}: outlet cell {c} has a receiver");
            assert!(d.mesh.neighbors(c).contains(&(fd[c] as u32)), "{name}: receiver of {c} is not a neighbour");
            assert!(filled[fd[c] as usize] <= filled[c], "{name}: cell {c} drains uphill on the filled surface");
        }
    }
    assert_eq!(non_ocean_roots, n_sinks, "{name}: roots that are not outlets must be exactly the reported sinks");

    // every cell reaches a root in at most n steps (no cycles)
    for c in 0..n {
        let mut x = c;
        let mut steps = 0;
        while fd[x] >= 0 {
            x = fd[x] as usize;
            steps += 1;
            assert!(steps <= n, "{name}: cycle or runaway path from cell {c}");
        }
    }

    // filled surface: >= elevation, equal on roots, depth = filled - z exactly, equals an independent fill
    for c in 0..n {
        assert!(filled[c] >= z[c], "{name}: filled below elevation at {c}");
        assert_eq!(depth[c], filled[c] - z[c], "{name}: depression_depth mismatch at {c}");
        assert!(depth[c] >= 0.0);
        if fd[c] < 0 { assert_eq!(filled[c], z[c], "{name}: root {c} was filled"); }
    }
    let root: Vec<bool> = (0..n).map(|c| fd[c] < 0).collect();
    let pd = planchon_darboux(d, z, &root);
    for c in 0..n {
        assert_eq!(pd[c], filled[c], "{name}: filled surface differs from Planchon-Darboux at cell {c}");
    }

    // flow_order: a permutation of all cells in which every cell precedes its receiver
    assert_eq!(order.len(), n);
    let mut pos = vec![usize::MAX; n];
    for (k, &c) in order.iter().enumerate() {
        let c = c as usize;
        assert!(c < n && pos[c] == usize::MAX, "{name}: flow_order is not a permutation");
        pos[c] = k;
    }
    for c in 0..n {
        if fd[c] >= 0 { assert!(pos[c] < pos[fd[c] as usize], "{name}: cell {c} comes after its receiver in flow_order"); }
    }

    // drainage_area: independent accumulation (walk every land cell to its root, add its area to every cell on the way)
    let mut want = vec![0.0f64; n];
    let mut total_land = 0.0;
    for c in 0..n {
        if mask[c] > 0.5 { continue; }
        total_land += d.mesh.area[c];
        let mut x = c;
        loop {
            want[x] += d.mesh.area[c];
            if fd[x] < 0 { break; }
            x = fd[x] as usize;
        }
    }
    let mut sum_roots = 0.0;
    for c in 0..n {
        assert!((acc[c] - want[c]).abs() <= 1e-9 * want[c].max(d.mesh.area[c]), "{name}: drainage_area at {c}: {} vs {}", acc[c], want[c]);
        if fd[c] < 0 { sum_roots += acc[c]; }
    }
    assert!((sum_roots - total_land).abs() <= 1e-9 * total_land.max(1.0), "{name}: roots hold {sum_roots}, land is {total_land}");
    let n_ocean = (0..n).filter(|&c| mask[c] > 0.5).count();
    (roots, n_ocean)
}

#[test]
fn every_land_cell_drains_to_an_outlet_on_all_topologies() {
    let worlds: Vec<(&str, Domain, u64)> = vec![
        ("sphere", sphere(4), 11),
        ("cube", cube(), 12),
        ("plane_wall", rect(40, 24, BoundaryKind::Wall), 13),
        ("plane_wrap", rect(40, 24, BoundaryKind::Wrap), 13),
        ("disc_wall", disc(BoundaryKind::Wall), 15),
    ];
    for (name, d, seed) in worlds {
        let z = terrain(&d, seed);
        let (s, f) = route(&d, z.clone(), &[]);
        let (roots, n_ocean) = check_routing(name, &d, &z, &f, &s, true);
        let n = d.n_cells();
        assert!(n_ocean > n / 5 && n_ocean < 4 * n / 5, "{name}: degenerate test world ({n_ocean} ocean cells of {n})");
        assert_eq!(roots, n_ocean, "{name}: with an ocean present the only roots are ocean cells");
        assert_eq!(s.module_param("flow_routing", "n_sinks"), Some(0.0), "{name}");
        let dep = s.module_param("flow_routing", "n_depression_cells").unwrap();
        assert!(dep > 0.0, "{name}: the jittered terrain must contain depressions");
        assert!(s.module_param("flow_routing", "max_depression_depth_m").unwrap() > 0.0);
    }
}

#[test]
fn hand_built_bowl_fills_to_its_lowest_rim_and_spills_there() {
    // 9 x 7 walled plane, ocean column i = 0 (-10 m), land 100 m, a two-cell bowl (20 m) at (4,3), (5,3)
    // and a west channel (3,3) = 60, (2,3) = 40, (1,3) = 30. Lowest rim = 60 m.
    let (nx, ny) = (9usize, 7usize);
    let id = |i: usize, j: usize| j * nx + i;
    let d = rect(nx, ny, BoundaryKind::Wall);
    let mut z = vec![100.0; nx * ny];
    for j in 0..ny { z[id(0, j)] = -10.0; }
    z[id(4, 3)] = 20.0;
    z[id(5, 3)] = 20.0;
    z[id(3, 3)] = 60.0;
    z[id(2, 3)] = 40.0;
    z[id(1, 3)] = 30.0;
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("bowl", &d, &z, &f, &s, true);
    let (filled, depth, acc) = (f.data("elevation_filled").unwrap(), f.data("depression_depth").unwrap(), f.data("drainage_area").unwrap());
    let fd = fd_of(&f);
    assert_eq!(filled[id(4, 3)], 60.0);
    assert_eq!(filled[id(5, 3)], 60.0);
    assert_eq!(depth[id(4, 3)], 40.0);
    assert_eq!(depth[id(5, 3)], 40.0);
    for c in 0..nx * ny {
        if c != id(4, 3) && c != id(5, 3) { assert_eq!(depth[c], 0.0, "only the bowl is a depression (cell {c})"); }
    }
    // the bowl spills west through the channel into the ocean
    for i in 1..=5 { assert_eq!(fd[id(i, 3)], id(i - 1, 3) as i64, "channel cell ({i},3)"); }
    // cells next to the bowl drain into it
    assert_eq!(fd[id(4, 2)], id(4, 3) as i64);
    assert_eq!(fd[id(4, 4)], id(4, 3) as i64);
    assert_eq!(fd[id(5, 2)], id(5, 3) as i64);
    assert_eq!(fd[id(6, 3)], id(5, 3) as i64);
    // all 8 x 7 land cells drain to the ocean column; the ocean cell cells hold only land area
    let ocean_sum: f64 = (0..ny).map(|j| acc[id(0, j)]).sum();
    assert!((ocean_sum - 56.0 * CELL_AREA).abs() < 1.0);
    assert_eq!(s.module_param("flow_routing", "n_depression_cells"), Some(2.0));
    assert_eq!(s.module_param("flow_routing", "max_depression_depth_m"), Some(40.0));
}

#[test]
fn flats_are_resolved_breadth_first_from_the_coast() {
    // 20 x 10 plateau at 100 m, ocean column i = 0: nothing is a depression and every cell at column i reaches the ocean in exactly i steps.
    let (nx, ny) = (20usize, 10usize);
    let d = rect(nx, ny, BoundaryKind::Wall);
    let mut z = vec![100.0; nx * ny];
    for j in 0..ny { z[j * nx] = -10.0; }
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("plateau", &d, &z, &f, &s, true);
    assert_eq!(s.module_param("flow_routing", "n_depression_cells"), Some(0.0));
    let fd = fd_of(&f);
    for j in 0..ny {
        for i in 1..nx {
            let (mut x, mut k) = (j * nx + i, 0usize);
            while fd[x] >= 0 {
                x = fd[x] as usize;
                k += 1;
            }
            assert_eq!(k, i, "path length of ({i},{j})");
        }
    }
}

#[test]
fn sub_sea_level_pit_that_is_not_ocean_is_filled_like_any_depression() {
    // Same plateau; one cell at -50 m inside the land is below sea level but is a lake candidate (1 cell < 1% of the area), not ocean.
    let (nx, ny) = (20usize, 10usize);
    let d = rect(nx, ny, BoundaryKind::Wall);
    let mut z = vec![100.0; nx * ny];
    for j in 0..ny { z[j * nx] = -10.0; }
    let pit = 5 * nx + 10;
    z[pit] = -50.0;
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("pit", &d, &z, &f, &s, true);
    assert_eq!(f.data("ocean_mask").unwrap()[pit], 0.0);
    assert_eq!(f.data("elevation_filled").unwrap()[pit], 100.0);
    assert_eq!(f.data("depression_depth").unwrap()[pit], 150.0);
    // the lowest-hop neighbour (west) discovered it, so the pit drains west
    assert_eq!(fd_of(&f)[pit], (5 * nx + 9) as i64);
}

#[test]
fn steepest_descent_on_a_tilted_plane() {
    let (nx, ny) = (20usize, 10usize);
    let d = rect(nx, ny, BoundaryKind::Wall);
    let z: Vec<f64> = (0..nx * ny).map(|c| if c % nx == 0 { -40.0 } else { 10.0 + 50.0 * (c % nx) as f64 }).collect();
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("slope", &d, &z, &f, &s, true);
    let fd = fd_of(&f);
    for c in 0..nx * ny {
        if c % nx >= 1 { assert_eq!(fd[c], c as i64 - 1, "cell {c} must drain west"); }
    }
    assert_eq!(s.module_param("flow_routing", "n_depression_cells"), Some(0.0));
}

#[test]
fn void_boundary_is_an_outflow_and_wall_is_closed() {
    let (nx, ny) = (30usize, 10usize);
    let z: Vec<f64> = (0..nx * ny).map(|c| 100.0 + 5.0 * (c % nx) as f64 + 0.3 * (c / nx) as f64).collect();
    // Void: all 2*(30+10)-4 = 76 edge cells are outlets, every cell drains to one of them, roots hold all the area.
    let d = rect(nx, ny, BoundaryKind::Void);
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("void", &d, &z, &f, &s, true);
    assert_eq!(s.module_param("flow_routing", "n_void_outlets"), Some(76.0));
    assert_eq!(s.module_param("flow_routing", "n_outlets"), Some(76.0));
    assert_eq!(s.module_param("flow_routing", "n_sinks"), Some(0.0));
    let fd = fd_of(&f);
    for c in 0..nx * ny {
        assert_eq!(fd[c] < 0, d.mesh.boundary_len[c] > 0.0, "root iff on the Void edge (cell {c})");
    }
    // switch off: Void behaves like a wall, so the lowest cell becomes the one terminal sink
    let (s2, f2) = route(&d, z.clone(), &[("void_outflow", 0.0)]);
    check_routing("void_off", &d, &z, &f2, &s2, false);
    assert_eq!(s2.module_param("flow_routing", "n_outlets"), Some(0.0));
    assert_eq!(s2.module_param("flow_routing", "n_sinks"), Some(1.0));
    // Wall with the same terrain: closed, one sink at the lowest cell (index 0)
    let w = rect(nx, ny, BoundaryKind::Wall);
    let (s3, f3) = route(&w, z.clone(), &[]);
    check_routing("wall", &w, &z, &f3, &s3, true);
    assert_eq!(s3.module_param("flow_routing", "n_sinks"), Some(1.0));
    assert_eq!(fd_of(&f3)[0], -1);
}

#[test]
fn closed_world_without_outlet_gets_one_terminal_sink_or_an_error() {
    let d = rect(20, 10, BoundaryKind::Wall);
    let z: Vec<f64> = (0..200).map(|i| 1000.0 + ((i * 37) % 101) as f64).collect();
    let lowest = (0..200).fold(0usize, |b, i| if z[i] < z[b] { i } else { b });
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("closed", &d, &z, &f, &s, true);
    assert_eq!(s.module_param("flow_routing", "n_sinks"), Some(1.0));
    assert_eq!(s.module_param("flow_routing", "n_outlets"), Some(0.0));
    let fd = fd_of(&f);
    let roots: Vec<usize> = (0..200).filter(|&c| fd[c] < 0).collect();
    assert_eq!(roots, vec![lowest]);
    assert!((f.data("drainage_area").unwrap()[lowest] - 200.0 * CELL_AREA).abs() < 1.0);
    // require_outlet = 1 turns it into an error, not a hang
    let (mut s2, mut f2) = build(&d, z, &[("require_outlet", 1.0)]);
    assert!(s2.run(&d, &mut f2, &DirtyRegion::All).is_err());
}

#[test]
fn all_land_sphere_drains_to_a_single_sink() {
    let d = sphere(3);
    let mut rng = Rng::new(5);
    let z: Vec<f64> = (0..d.n_cells()).map(|_| 1000.0 + 300.0 * rng.next_f64()).collect();
    let lowest = (0..z.len()).fold(0usize, |b, i| if z[i] < z[b] { i } else { b });
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("land_sphere", &d, &z, &f, &s, true);
    assert_eq!(s.module_param("flow_routing", "n_sinks"), Some(1.0));
    assert_eq!(fd_of(&f)[lowest], -1);
    assert!((f.data("drainage_area").unwrap()[lowest] - d.mesh.total_area()).abs() < 1e-6 * d.mesh.total_area());
}

#[test]
fn all_ocean_world_has_no_routing_work() {
    let d = sphere(3);
    let z = vec![-4000.0; d.n_cells()];
    let (s, f) = route(&d, z.clone(), &[]);
    check_routing("ocean_sphere", &d, &z, &f, &s, true);
    assert!(fd_of(&f).iter().all(|&v| v == -1));
    assert!(f.data("drainage_area").unwrap().iter().all(|&a| a == 0.0));
    assert_eq!(s.module_param("flow_routing", "n_depression_cells"), Some(0.0));
}

#[test]
fn results_are_deterministic_and_pure_functions_match_the_module() {
    let d = sphere(4);
    let z = terrain(&d, 21);
    let (_, a) = route(&d, z.clone(), &[]);
    let (_, b) = route(&d, z.clone(), &[]);
    for name in ["elevation_filled", "depression_depth", "flow_dir", "flow_order", "drainage_area"] {
        assert_eq!(a.data(name).unwrap(), b.data(name).unwrap(), "{name} differs between runs");
    }
    // pure functions, called directly
    let ocean: Vec<bool> = a.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let outlet = outlet_mask(&d, &ocean, true);
    let fl = priority_flood(&d.mesh, &z, &outlet).unwrap();
    let fd = flow_directions(&d.mesh, &fl);
    let order = flow_order(&fl);
    assert_eq!(fl.filled, a.data("elevation_filled").unwrap());
    assert_eq!(fd.iter().map(|&v| v as f64).collect::<Vec<_>>(), a.data("flow_dir").unwrap());
    assert_eq!(order.iter().map(|&v| v as f64).collect::<Vec<_>>(), a.data("flow_order").unwrap());
    let land: Vec<bool> = ocean.iter().map(|&o| !o).collect();
    assert_eq!(accumulate_area(&d.mesh, &fd, &order, &land), a.data("drainage_area").unwrap());
    // root_of agrees with walking the receivers
    let root = root_of(&fd, &order);
    for c in (0..d.n_cells()).step_by(7) {
        let mut x = c;
        while fd[x] >= 0 { x = fd[x] as usize; }
        assert_eq!(root[c], x as i32);
    }
}

#[test]
fn bad_inputs_and_interface() {
    let d = rect(10, 6, BoundaryKind::Wall);
    // priority_flood rejects wrong lengths and non-finite elevations
    assert!(priority_flood(&d.mesh, &vec![0.0; 59], &vec![false; 60]).is_err());
    let mut z = vec![0.0; 60];
    z[7] = f64::NAN;
    assert!(priority_flood(&d.mesh, &z, &vec![false; 60]).is_err());
    // parameters
    let mut m = FlowRouting::new();
    assert_eq!(m.params().get("void_outflow"), Some(1.0));
    assert_eq!(m.params().get("require_outlet"), Some(0.0));
    assert!(m.set_param("void_outflow", 2.0).is_err());
    assert!(m.set_param("require_outlet", -1.0).is_err());
    assert!(m.set_param("n_sinks", 1.0).is_err(), "output-only parameters are not settable");
    assert!(m.set_param("nonsense", 1.0).is_err());
    assert!(m.set_param("require_outlet", 1.0).is_ok());
    let spec = m.spec();
    assert_eq!(spec.name, "flow_routing");
    assert_eq!(spec.inputs, vec!["elevation".to_string(), "ocean_mask".to_string()]);
    assert_eq!(spec.outputs.len(), 5);
    // scheduler order: sea_level -> ocean_mask -> flow_routing
    let mut s = Scheduler::new();
    add_flow_routing_stage(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |n: &str| order.iter().position(|m| m == n).unwrap();
    assert!(pos("sea_level") < pos("ocean_mask") && pos("ocean_mask") < pos("flow_routing"));
    // the module alone needs `ocean_mask` from somewhere else
    let mut s2 = Scheduler::new();
    add_flow_routing_module(&mut s2).unwrap();
    let mut f = FieldStore::new(60);
    f.put("elevation", 1, vec![1.0; 60]).unwrap();
    assert!(s2.run(&d, &mut f, &DirtyRegion::All).is_err());
}
