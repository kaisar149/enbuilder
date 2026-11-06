use enbuilder::generators::*;
use enbuilder::*;

const YEAR: f64 = 3.15576e7;
const DAY: f64 = 86400.0;

// ------------------------------------------------------------ helpers

fn land_mask(up: [f64; 3]) -> bool {
    // ~30% land: a polar-cap-like region around an off-axis direction
    let a = [0.3_f64, 0.5, 0.8];
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    (up[0] * a[0] + up[1] * a[1] + up[2] * a[2]) / l > 0.4
}

fn elevation_for(d: &Domain) -> Vec<f64> {
    d.mesh.up.iter().map(|&u| if land_mask(u) { 100.0 } else { -3000.0 }).collect()
}

fn sphere(sub: u32, g: f64) -> Domain {
    generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: g, seed: 7 }).unwrap()
}

/// Solve the energy stage on `d` (lights must already be set). Returns fields and run report.
fn climate(d: &Domain, season: f64) -> (FieldStore, RunReport) {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, elevation_for(d)).unwrap();
    seed_temperature(&mut f, 288.0).unwrap();
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    s.set_param("insolation", "season", season).unwrap();
    let r = s.run(d, &mut f, &DirtyRegion::All).unwrap();
    (f, r)
}

fn earth_like(tilt_deg: f64) -> Domain {
    let mut d = sphere(3, 9.81);
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, tilt_deg.to_radians()));
    d
}

fn mean_where(d: &Domain, data: &[f64], cond: impl Fn([f64; 3]) -> bool) -> f64 {
    let (mut s, mut c) = (0.0, 0usize);
    for i in 0..d.n_cells() {
        if cond(d.mesh.up[i]) {
            s += data[i];
            c += 1;
        }
    }
    assert!(c > 0, "empty region");
    s / c as f64
}

fn area_mean(d: &Domain, data: &[f64]) -> f64 {
    data.iter().zip(d.mesh.area.iter()).map(|(t, a)| t * a).sum::<f64>() / d.mesh.total_area()
}

// ------------------------------------------------------------ B: energy (acceptance)

#[test]
fn earth_like_sphere_equator_warmer_than_poles() {
    let d = earth_like(0.0);
    let (f, r) = climate(&d, 0.0);
    let t = f.data("temperature").unwrap();
    let eq = mean_where(&d, t, |u| u[2].abs() < 0.2);
    let pole = mean_where(&d, t, |u| u[2].abs() > 0.9);
    assert!(eq - pole > 25.0, "equator {eq} pole {pole}");
    assert!((285.0..310.0).contains(&eq), "equator {eq}");
    let g = area_mean(&d, t);
    assert!((275.0..295.0).contains(&g), "global mean {g}");
    assert!(r.loops[0].converged);
}

#[test]
fn tilted_planet_has_seasons_and_untilted_does_not() {
    let d = earth_like(23.44);
    let (fa, ra) = climate(&d, 0.25);
    let (fb, rb) = climate(&d, 0.75);
    let (ta, tb) = (fa.data("temperature").unwrap(), fb.data("temperature").unwrap());
    let north = |t: &[f64]| mean_where(&d, t, |u| u[2] > 0.7);
    let south = |t: &[f64]| mean_where(&d, t, |u| u[2] < -0.7);
    assert!(north(ta) - north(tb) > 30.0, "north {} vs {}", north(ta), north(tb));
    assert!(south(tb) - south(ta) > 30.0, "south {} vs {}", south(tb), south(ta));
    assert!(ra.loops[0].iterations < 20 && rb.loops[0].iterations < 20);

    let d0 = earth_like(0.0);
    let (f0a, _) = climate(&d0, 0.25);
    let (f0b, _) = climate(&d0, 0.75);
    let diff = f0a.data("temperature").unwrap().iter().zip(f0b.data("temperature").unwrap()).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(diff < 0.5, "untilted planet should not change with season, got {diff} K");
}

#[test]
fn flat_plane_with_overhead_sun_cools_with_distance() {
    let mut d = generate_plane(&PlaneParams {
        shape: PlaneShape::Disc { diameter_cells: 40 },
        cell_size: 1.0e5,
        boundary: BoundaryKind::Wall,
        surface_gravity: 9.81,
        seed: 3,
    })
    .unwrap();
    let h = 2.0e6;
    d.lights.push(Light::point([0.0, 0.0, h], 1361.0, h));
    let (f, r) = climate(&d, 0.0);
    let t = f.data("temperature").unwrap();
    let mut sums = [0.0; 5];
    let mut cnt = [0usize; 5];
    for i in 0..d.n_cells() {
        let c = d.mesh.centroid[i];
        let rad = (c[0] * c[0] + c[1] * c[1]).sqrt();
        let b = ((rad / (h / 5.0)) as usize).min(4);
        sums[b] += t[i];
        cnt[b] += 1;
    }
    let m: Vec<f64> = (0..5).map(|b| sums[b] / cnt[b] as f64).collect();
    for b in 0..4 {
        assert!(m[b] > m[b + 1], "temperature must fall with distance: {m:?}");
    }
    assert!(m[0] - m[4] > 30.0, "{m:?}");
    assert!(r.loops[0].converged && r.loops[0].iterations < 20);
}

#[test]
fn cube_world_varies_sensibly_across_faces_and_edges() {
    let mut d = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 8, surface_gravity: 9.81, seed: 5 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 0.0));
    let (f, r) = climate(&d, 0.0);
    let t = f.data("temperature").unwrap();
    assert!(t.iter().all(|&x| (240.0..320.0).contains(&x)), "unphysical temperatures");
    let cap = mean_where(&d, t, |u| u[2].abs() > 0.9);
    let belt = mean_where(&d, t, |u| u[2].abs() < 0.3);
    assert!(belt - cap > 15.0, "belt {belt} cap {cap}");
    let n = 8 * 8;
    let (mut same, mut cross, mut ns, mut nc) = (0.0, 0.0, 0usize, 0usize);
    for i in 0..d.n_cells() {
        for &j in d.neighbors(i) {
            let dt = (t[i] - t[j as usize]).abs();
            if i / n == j as usize / n { same += dt; ns += 1 } else { cross += dt; nc += 1 }
        }
    }
    assert!(nc > 0);
    assert!(cross / (nc as f64) < 2.0 * same / (ns as f64), "temperature jumps across cube edges");
    assert!(r.loops[0].converged);
}

#[test]
fn ice_albedo_loop_converges_in_under_20_passes() {
    for season in [0.0, 0.125, 0.25, 0.5, 0.75] {
        let d = earth_like(23.44);
        let (f, r) = climate(&d, season);
        let lr = &r.loops[0];
        assert!(lr.converged && lr.iterations < 20, "season {season}: {lr:?}");
        // ice appears only where it is cold
        let t = f.data("temperature").unwrap();
        let ice = f.data("ice_fraction").unwrap();
        for i in 0..d.n_cells() {
            if t[i] > 275.0 { assert!(ice[i] < 0.2, "ice at {} K", t[i]); }
        }
    }
}

#[test]
fn energy_stage_plan_and_external_inputs() {
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    assert_eq!(s.plan_order().unwrap(), vec!["insolation", "albedo", "energy_balance"]);
    assert!(s.external_inputs().contains("elevation"));
    assert!(!s.external_inputs().contains("temperature"));
}

#[test]
fn terrain_shadowing_darkens_cells_behind_a_ridge() {
    let d = {
        let mut d = generate_plane(&PlaneParams {
            shape: PlaneShape::Rect { nx: 20, ny: 20 },
            cell_size: 1000.0,
            boundary: BoundaryKind::Wall,
            surface_gravity: 9.81,
            seed: 1,
        })
        .unwrap();
        let a = 5.0f64.to_radians();
        d.lights.push(Light::directional([a.cos(), 0.0, a.sin()], 1361.0));
        d
    };
    let idx = |i: usize, j: usize| j * 20 + i;
    let mut f = FieldStore::new(d.n_cells());
    let mut elev = vec![0.0; d.n_cells()];
    for j in 0..20 {
        elev[idx(10, j)] = 500.0;
    }
    f.put("elevation", 1, elev).unwrap();
    let mut m = Insolation::new().with_shadowing(true);
    m.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let q = f.data("insolation").unwrap();
    let full = 1361.0 * 5.0f64.to_radians().sin();
    assert!(q[idx(8, 5)] == 0.0, "cell behind ridge must be dark");
    assert!(q[idx(13, 5)] > 0.99 * full && q[idx(2, 5)] > 0.99 * full && q[idx(10, 5)] > 0.99 * full);

    f.put("elevation", 1, vec![0.0; d.n_cells()]).unwrap();
    m.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("insolation").unwrap().iter().all(|&v| (v - full).abs() < 1e-9));
}

#[test]
fn insolation_requires_lights_and_validates_params() {
    let d = sphere(1, 9.81);
    let mut f = FieldStore::new(d.n_cells());
    assert!(Insolation::new().run(&d, &mut f, &DirtyRegion::All).is_err());
    assert!(Insolation::new().set_param("nope", 1.0).is_err());
    assert!(EnergyBalance::new().set_param("greenhouse", 1.5).is_err());
}

// ------------------------------------------------------------ A: tectonics and terrain

fn painted(d: &Domain, va: [f64; 3], vb: [f64; 3], ca: bool, cb: bool) -> PaintedPlates {
    PaintedPlates {
        plate_of_cell: d.mesh.up.iter().map(|u| if u[0] < 0.0 { 1 } else { 0 }).collect(),
        plates: vec![PlateDef { velocity: va, continental: ca }, PlateDef { velocity: vb, continental: cb }],
    }
}

fn run_tectonics(d: &Domain, p: PaintedPlates) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    let mut s = Scheduler::new();
    s.register(Box::new(Tectonics::new().with_painted(p))).unwrap();
    s.set_param("tectonics", "hotspots", 0.0).unwrap();
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    f
}

#[test]
fn collision_builds_mountains_rift_digs_valleys_transform_does_nothing() {
    let d = sphere(4, 9.81);
    let near = |u: [f64; 3]| u[0].abs() < 0.1;
    let far = |u: [f64; 3]| u[0].abs() > 0.8;

    let f = run_tectonics(&d, painted(&d, [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true, true));
    let e = f.data("tectonic_elevation").unwrap();
    assert!(mean_where(&d, e, near) > mean_where(&d, e, far) + 4000.0, "collision");
    assert!(f.data("volcanism").unwrap().iter().all(|&v| v < 0.01));

    let f = run_tectonics(&d, painted(&d, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], true, true));
    let e = f.data("tectonic_elevation").unwrap();
    assert!(mean_where(&d, e, near) < mean_where(&d, e, far) - 1000.0, "continental rift");

    let f = run_tectonics(&d, painted(&d, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], true, true));
    let e = f.data("tectonic_elevation").unwrap();
    assert!(e.iter().all(|&v| (v - 400.0).abs() < 1.0), "transform boundary must not change elevation");
}

#[test]
fn subduction_makes_arc_on_overriding_side_and_trench_on_oceanic_side() {
    let d = sphere(4, 9.81);
    // plate 0 (x>0) continental moving toward plate 1 (x<0) oceanic
    let f = run_tectonics(&d, painted(&d, [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true, false));
    let e = f.data("tectonic_elevation").unwrap();
    let a_edge = mean_where(&d, e, |u| u[0] > 0.0 && u[0] < 0.1);
    let a_far = mean_where(&d, e, |u| u[0] > 0.8);
    let b_edge = mean_where(&d, e, |u| u[0] < 0.0 && u[0] > -0.1);
    let b_far = mean_where(&d, e, |u| u[0] < -0.8);
    assert!(a_edge > a_far + 2000.0, "volcanic arc: {a_edge} vs {a_far}");
    assert!(b_edge < b_far - 3000.0, "trench: {b_edge} vs {b_far}");
    assert!(f.data("volcanism").unwrap().iter().cloned().fold(0.0, f64::max) > 0.9);
}

#[test]
fn random_tectonics_is_deterministic_and_well_formed() {
    let d = sphere(4, 9.81);
    let run = |d: &Domain| {
        let mut f = FieldStore::new(d.n_cells());
        let mut s = Scheduler::new();
        s.register(Box::new(Tectonics::new())).unwrap();
        s.run(d, &mut f, &DirtyRegion::All).unwrap();
        f
    };
    let (a, b) = (run(&d), run(&d));
    assert_eq!(a.data("tectonic_elevation").unwrap(), b.data("tectonic_elevation").unwrap());
    let d2 = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: 4, surface_gravity: 9.81, seed: 99 }).unwrap();
    assert_ne!(a.data("tectonic_elevation").unwrap(), run(&d2).data("tectonic_elevation").unwrap());

    let mut ids: Vec<u64> = a.data("plate_id").unwrap().iter().map(|&v| v as u64).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 12);
    assert!(a.data("volcanism").unwrap().iter().all(|&v| (0.0..=1.0).contains(&v)));
    assert!(a.data("rock_strength").unwrap().iter().all(|&v| (0.3..=2.0).contains(&v)));
    assert!(a.data("tectonic_elevation").unwrap().iter().all(|v| v.is_finite()));
    assert!(a.data("boundary_type").unwrap().iter().any(|&v| v > 0.0));
}

fn terrain_world(g: f64, setup: impl Fn(&mut Scheduler)) -> (Domain, FieldStore) {
    let d = sphere(4, g);
    let mut f = FieldStore::new(d.n_cells());
    let mut s = Scheduler::new();
    s.register(Box::new(Tectonics::new().with_painted(painted(&d, [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true, true)))).unwrap();
    s.register(Box::new(TerrainShaping::new())).unwrap();
    s.set_param("tectonics", "hotspots", 0.0).unwrap();
    setup(&mut s);
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    (d, f)
}

fn max_of(v: &[f64]) -> f64 { v.iter().cloned().fold(f64::MIN, f64::max) }

#[test]
fn higher_gravity_gives_lower_flatter_relief() {
    let run = |g: f64| {
        let (_, f) = terrain_world(g, |_| {});
        (max_of(f.data("elevation").unwrap()), max_of(f.data("slope").unwrap()))
    };
    let (lo_e, _) = run(3.0);
    let (mid_e, mid_s) = run(9.81);
    let (hi_e, hi_s) = run(25.0);
    assert!(hi_e < mid_e - 1500.0, "g=25 peak {hi_e} vs g=9.81 peak {mid_e}");
    assert!(lo_e > mid_e, "g=3 peak {lo_e} vs g=9.81 peak {mid_e}");
    assert!(hi_s < mid_s, "slope {hi_s} vs {mid_s}");
    assert!(mid_e < 9000.0 * 1.3 + 1.0);
}

#[test]
fn thermal_erosion_limits_slope_and_conserves_volume() {
    let setup = |iters: f64| {
        move |s: &mut Scheduler| {
            s.set_param("terrain", "noise_amp_m", 0.0).unwrap();
            s.set_param("terrain", "peak_ref_m", 1e9).unwrap();
            s.set_param("terrain", "depth_ref_m", 1e9).unwrap();
            s.set_param("terrain", "max_slope", 0.002).unwrap();
            s.set_param("terrain", "thermal_iters", iters).unwrap();
        }
    };
    let (d, before) = terrain_world(9.81, setup(0.0));
    let (_, after) = terrain_world(9.81, setup(8.0));
    let (eb, ea) = (before.data("elevation").unwrap(), after.data("elevation").unwrap());
    assert!(max_of(after.data("slope").unwrap()) < 0.9 * max_of(before.data("slope").unwrap()));
    let vol = |e: &[f64]| e.iter().zip(d.mesh.area.iter()).map(|(h, a)| h * a).sum::<f64>();
    let scale: f64 = eb.iter().zip(d.mesh.area.iter()).map(|(h, a)| h.abs() * a).sum();
    assert!((vol(ea) - vol(eb)).abs() / scale < 1e-9, "thermal erosion must conserve volume");
}

#[test]
fn soft_cap_is_identity_below_knee_and_bounded_above() {
    use enbuilder::terrain::soft_cap;
    assert_eq!(soft_cap(1000.0, 9000.0, 0.6), 1000.0);
    let v = soft_cap(1e9, 9000.0, 0.6);
    assert!(v <= 9000.0 && v > 8999.0);
    assert!(soft_cap(7000.0, 9000.0, 0.6) < 7000.0);
}

#[test]
fn rng_and_noise_are_deterministic() {
    use enbuilder::{noise::fbm3, rng::Rng};
    let (mut a, mut b) = (Rng::new(5), Rng::new(5));
    for _ in 0..10 { assert_eq!(a.next_u64(), b.next_u64()); }
    let v = Rng::new(1).choose_distinct(100, 10);
    let mut s = v.clone();
    s.sort();
    s.dedup();
    assert_eq!(s.len(), 10);
    assert_eq!(fbm3([1.3, 2.2, -0.4], 5, 9), fbm3([1.3, 2.2, -0.4], 5, 9));
    assert!(fbm3([1.3, 2.2, -0.4], 5, 9).abs() <= 1.0);
}
