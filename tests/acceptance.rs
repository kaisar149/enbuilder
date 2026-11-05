use enbuilder::generators::*;
use enbuilder::testmods::*;
use enbuilder::*;

const PI: f64 = std::f64::consts::PI;

fn all_domains(big_disc: bool) -> Vec<(&'static str, Domain, f64)> {
    let dd: usize = if big_disc { 100 } else { 30 };
    let sphere = generate_sphere(&SphereParams { radius: 1.0, subdivision: 3, surface_gravity: 9.81, seed: 1 }).unwrap();
    let cube = generate_cube(&CubeParams { half_side: 1.0, cells_per_edge: 8, surface_gravity: 9.81, seed: 1 }).unwrap();
    let plane = |shape, boundary| {
        generate_plane(&PlaneParams { shape, cell_size: 1.0, boundary, surface_gravity: 9.81, seed: 1 }).unwrap()
    };
    vec![
        ("sphere", sphere, 4.0 * PI),
        ("cube", cube, 24.0),
        ("rect_wrap", plane(PlaneShape::Rect { nx: 20, ny: 20 }, BoundaryKind::Wrap), 400.0),
        ("rect_wall", plane(PlaneShape::Rect { nx: 30, ny: 20 }, BoundaryKind::Wall), 600.0),
        ("rect_void", plane(PlaneShape::Rect { nx: 16, ny: 12 }, BoundaryKind::Void), 192.0),
        ("disc_wall", plane(PlaneShape::Disc { diameter_cells: dd }, BoundaryKind::Wall), PI * (dd as f64 / 2.0) * (dd as f64 / 2.0)),
    ]
}

#[test]
fn neighbor_lists_are_symmetric() {
    for (name, d, _) in all_domains(true) {
        d.mesh.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn areas_match_analytic_within_1_percent() {
    for (name, d, analytic) in all_domains(true) {
        let err = (d.mesh.total_area() - analytic).abs() / analytic;
        assert!(err < 0.01, "{name}: area error {err}");
    }
}

#[test]
fn cell_counts_and_degrees() {
    let s = generate_sphere(&SphereParams { radius: 1.0, subdivision: 3, ..Default::default() }).unwrap();
    assert_eq!(s.n_cells(), 10 * 64 + 2);
    let pents = (0..s.n_cells()).filter(|&i| s.mesh.degree(i) == 5).count();
    assert_eq!(pents, 12);
    assert!((0..s.n_cells()).all(|i| matches!(s.mesh.degree(i), 5 | 6)));
    let c = generate_cube(&CubeParams { cells_per_edge: 5, ..Default::default() }).unwrap();
    assert!((0..c.n_cells()).all(|i| c.mesh.degree(i) == 4));
}

#[test]
fn generators_are_deterministic() {
    let a = generate_sphere(&SphereParams { radius: 1.0, subdivision: 2, ..Default::default() }).unwrap();
    let b = generate_sphere(&SphereParams { radius: 1.0, subdivision: 2, ..Default::default() }).unwrap();
    assert_eq!(a.mesh, b.mesh);
}

#[test]
fn frames_are_orthonormal_and_gravity_points_down() {
    for (name, d, _) in all_domains(false) {
        for i in 0..d.n_cells() {
            let f = d.frames[i];
            let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            assert!(dot(f.up, f.north).abs() < 1e-9, "{name}: north not tangent");
            assert!(dot(f.up, f.east).abs() < 1e-9, "{name}: east not tangent");
            assert!(dot(d.gravity[i], f.up) < 0.0, "{name}: gravity not downward");
        }
    }
}

fn pseudo_random(n: usize) -> Vec<f64> {
    let mut x: u64 = 0x9E3779B97F4A7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

#[test]
fn diffusion_conserves_and_converges_to_uniform_on_every_shape() {
    for (name, d, _) in all_domains(false) {
        let mut fields = FieldStore::new(d.n_cells());
        fields.put("temperature", 1, pseudo_random(d.n_cells())).unwrap();
        let weighted = |f: &FieldStore| -> f64 {
            f.data("temperature").unwrap().iter().zip(d.mesh.area.iter()).map(|(t, a)| t * a).sum()
        };
        let total0 = weighted(&fields);

        let mut s = Scheduler::new();
        s.register(Box::new(Diffusion::new("temperature"))).unwrap();
        s.set_param("diffusion", "steps", 25.0).unwrap();
        s.add_feedback_loop(
            FeedbackLoop::new("relax", &["diffusion"]).watch(&["temperature"]).relaxation(1.0).tolerance(1e-12).max_iters(50_000),
        )
        .unwrap();
        let report = s.run(&d, &mut fields, &DirtyRegion::All).unwrap();
        assert!(report.loops[0].converged, "{name}: did not converge ({:?})", report.loops[0]);

        let total1 = weighted(&fields);
        assert!((total1 - total0).abs() / total0 < 1e-10, "{name}: conservation error");
        let t = fields.data("temperature").unwrap();
        let spread = t.iter().cloned().fold(f64::MIN, f64::max) - t.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread < 1e-5, "{name}: not uniform, spread {spread}");
        let uniform = total0 / d.mesh.total_area();
        assert!((t[0] - uniform).abs() < 1e-5, "{name}: wrong uniform value");
    }
}

#[test]
fn scheduler_rejects_undeclared_cycles_but_accepts_declared_loops() {
    let build = |declare: bool| {
        let mut s = Scheduler::new();
        s.register(Box::new(Noop::new("a", &["x"], &["y"]))).unwrap();
        s.register(Box::new(Noop::new("b", &["y"], &["x"]))).unwrap();
        if declare {
            s.add_feedback_loop(FeedbackLoop::new("ab", &["a", "b"])).unwrap();
        }
        s.build()
    };
    match build(false) {
        Err(Error::Cycle(m)) => assert!(m.contains(&"a".to_string()) && m.contains(&"b".to_string())),
        other => panic!("expected cycle error, got {other:?}"),
    }
    assert!(build(true).is_ok());
}

#[test]
fn scheduler_orders_by_dependency_and_rejects_duplicate_producers() {
    let mut s = Scheduler::new();
    // registered in reverse dependency order on purpose
    s.register(Box::new(Noop::new("c", &["b_out"], &["c_out"]))).unwrap();
    s.register(Box::new(Noop::new("b", &["a_out"], &["b_out"]))).unwrap();
    s.register(Box::new(Noop::new("a", &["seed_field"], &["a_out"]))).unwrap();
    assert_eq!(s.plan_order().unwrap(), vec!["a", "b", "c"]);
    assert!(s.external_inputs().contains("seed_field"));

    let mut s2 = Scheduler::new();
    s2.register(Box::new(Noop::new("p", &[], &["f"]))).unwrap();
    s2.register(Box::new(Noop::new("q", &[], &["f"]))).unwrap();
    assert!(matches!(s2.build(), Err(Error::MultipleProducers { .. })));
}

#[test]
fn constant_source_then_diffusion_runs_in_dependency_order() {
    let d = all_domains(false).remove(2).1; // rect_wrap
    let mut fields = FieldStore::new(d.n_cells());
    fields.create("temperature", 1, 0.0).unwrap();
    let mut s = Scheduler::new();
    s.register(Box::new(Diffusion::new("temperature").with_source("heat_source"))).unwrap();
    s.register(Box::new(ConstantSource::new("heat_source", 1.0))).unwrap();
    let r = s.run(&d, &mut fields, &DirtyRegion::All).unwrap();
    assert_eq!(r.order, vec!["constant_source", "diffusion"]);
    assert!(fields.data("temperature").unwrap().iter().all(|&t| t > 0.0));
}

#[test]
fn feedback_loop_with_relaxation_converges_to_thermostat_target() {
    let d = all_domains(false).remove(2).1;
    let mut fields = FieldStore::new(d.n_cells());
    fields.put("temperature", 1, pseudo_random(d.n_cells())).unwrap();
    let mut s = Scheduler::new();
    s.register(Box::new(Thermostat::new("temperature", "heat_source", 5.0, 1.0))).unwrap();
    s.register(Box::new(Diffusion::new("temperature").with_source("heat_source"))).unwrap();
    s.add_feedback_loop(
        FeedbackLoop::new("thermo", &["thermostat", "diffusion"]).watch(&["temperature"]).relaxation(0.5).tolerance(1e-11).max_iters(20_000),
    )
    .unwrap();
    let r = s.run(&d, &mut fields, &DirtyRegion::All).unwrap();
    assert!(r.loops[0].converged, "{:?}", r.loops[0]);
    assert!(fields.data("temperature").unwrap().iter().all(|&t| (t - 5.0).abs() < 1e-5));
}

#[test]
fn failed_run_rolls_back() {
    let d = all_domains(false).remove(2).1;
    let mut fields = FieldStore::new(d.n_cells());
    let mut s = Scheduler::new();
    s.register(Box::new(Noop::new("a", &[], &["x"]))).unwrap();
    s.register(Box::new(Noop::new("b", &["x", "missing"], &["y"]))).unwrap();
    assert!(matches!(s.run(&d, &mut fields, &DirtyRegion::All), Err(Error::MissingInput { .. })));
    assert!(!fields.has("x"), "partial run must be rolled back");
}

#[test]
fn fieldstore_undo_redo_and_diff() {
    let mut f = FieldStore::new(4);
    f.create("h", 1, 0.0).unwrap();
    let v0 = f.current_version();
    f.write("h", |d| d[1] = 2.0).unwrap();
    let v1 = f.current_version();
    f.write("h", |d| {
        d[1] = 3.0;
        d[2] = 7.0;
    })
    .unwrap();
    let diff = f.diff("h", v1).unwrap();
    assert_eq!(diff.entries, vec![(1, 2.0, 3.0), (2, 0.0, 7.0)]);
    let diff0 = f.diff("h", v0).unwrap();
    assert_eq!(diff0.entries, vec![(1, 0.0, 3.0), (2, 0.0, 7.0)]);

    f.undo().unwrap();
    assert_eq!(f.data("h").unwrap(), &[0.0, 2.0, 0.0, 0.0]);
    f.redo().unwrap();
    assert_eq!(f.data("h").unwrap(), &[0.0, 3.0, 7.0, 0.0]);
    f.undo().unwrap();
    f.undo().unwrap();
    f.undo().unwrap(); // undoes the creation itself
    assert!(!f.has("h"));
    f.redo().unwrap();
    assert!(f.has("h"));

    // a group collapses many writes into one undo step
    f.begin_group("batch").unwrap();
    f.write("h", |d| d[0] = 1.0).unwrap();
    f.write("h", |d| d[3] = 1.0).unwrap();
    f.end_group().unwrap();
    f.undo().unwrap();
    assert_eq!(f.data("h").unwrap(), &[0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn fieldstore_history_truncation_is_reported() {
    let mut f = FieldStore::new(2).with_max_history(2);
    f.create("a", 1, 0.0).unwrap();
    for k in 1..=5 {
        f.write("a", |d| d[0] = k as f64).unwrap();
    }
    assert!(matches!(f.diff("a", 0), Err(Error::HistoryTruncated { .. })));
    assert!(f.diff("a", f.current_version() - 1).is_ok());
}
