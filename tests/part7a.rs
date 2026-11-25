//! Part 7a: the anomaly system (model, compilation, hooks, mute = baseline, inspector).
use enbuilder::anomaly::*;
use enbuilder::budget::{add_climate_pipeline, run_season};
use enbuilder::generators::{generate_plane, PlaneParams, PlaneShape};
use enbuilder::testworld::earth_like_world;
use enbuilder::{BoundaryKind, Domain, FieldStore, Scheduler};

fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// Centre of cell (x, y) of the plane (row-major, 10 km cells); read from the mesh so the test does not assume the origin.
fn cell_at(d: &Domain, x: usize, y: usize, nx: usize) -> usize {
    let _ = d;
    y * nx + x
}

// ------------------------------------------------------------------ shapes

#[test]
fn shape_distances_on_a_plane() {
    let d = plane(20, 10);
    let c = cell_at(&d, 5, 5, 20);
    let p = d.mesh.centroid[c];
    // Sphere: distance = |centroid - centre| - radius, zero inside
    let sh = Shape::Sphere { center: p, radius_m: 25_000.0 };
    let dist = sh.distances(&d).unwrap();
    assert_eq!(dist[c], 0.0);
    let j = cell_at(&d, 10, 5, 20); // 50 km away
    assert!((dist[j] - 25_000.0).abs() < 1e-6);
    // Column along z through p: horizontal distance minus radius
    let col = Shape::Column { center: p, axis: [0.0, 0.0, 1.0], radius_m: 0.0 };
    assert!((col.distances(&d).unwrap()[j] - 50_000.0).abs() < 1e-6);
    // Line from p to p + 100 km in x, half width 0: a cell 30 km along and 20 km off is 20 km away
    let b = [p[0] + 100_000.0, p[1], p[2]];
    let line = Shape::Line { a: p, b, half_width_m: 0.0 };
    let k = cell_at(&d, 8, 7, 20);
    assert!((line.distances(&d).unwrap()[k] - 20_000.0).abs() < 1e-6);
    // Volume: box covering x in [p.x - 10km, p.x + 10km] (all y,z): distance in x only
    let vol = Shape::Volume { min: [p[0] - 10_000.0, -1e9, -1e9], max: [p[0] + 10_000.0, 1e9, 1e9] };
    assert!((vol.distances(&d).unwrap()[j] - 40_000.0).abs() < 1e-6);
    // Region: surface distance = hops * 10 km on a 4-neighbour grid (Manhattan)
    let reg = Shape::Region { cells: vec![c as u32] };
    let r = reg.distances(&d).unwrap();
    assert!((r[j] - 50_000.0).abs() < 1e-6);
    assert!((r[k] - 50_000.0).abs() < 1e-6, "3 right + 2 up = 5 hops (Manhattan)");
    // Global
    assert!(Shape::Global.distances(&d).unwrap().iter().all(|&v| v == 0.0));
    // Invalid shapes are rejected
    assert!(Shape::Region { cells: vec![] }.distances(&d).is_err());
    assert!(Shape::Region { cells: vec![9999] }.distances(&d).is_err());
    assert!(Shape::Sphere { center: p, radius_m: -1.0 }.distances(&d).is_err());
}

// ------------------------------------------------------------------ falloff

#[test]
fn falloff_weights_follow_the_chosen_curve() {
    let l = 100.0;
    assert_eq!(Falloff::Hard.weight(0.0, l), 1.0);
    assert_eq!(Falloff::Hard.weight(1e-9, l), 0.0);
    assert!((Falloff::Linear.weight(50.0, l) - 0.5).abs() < 1e-12);
    assert_eq!(Falloff::Linear.weight(100.0, l), 0.0);
    assert!((Falloff::Gaussian.weight(100.0, l) - (-1.0f64).exp()).abs() < 1e-12);
    assert_eq!(Falloff::Gaussian.weight(301.0, l), 0.0, "finite support at 3 L");
    assert!((Falloff::InverseSquare.weight(100.0, l) - 0.5).abs() < 1e-12);
    assert!((Falloff::InverseSquare.weight(300.0, l) - 0.1).abs() < 1e-12);
    assert_eq!(Falloff::InverseSquare.weight(1001.0, l), 0.0, "finite support at 10 L");
    let c = Falloff::Custom(vec![(0.0, 1.0), (1.0, 0.25), (2.0, 0.0)]);
    assert!((c.weight(50.0, l) - 0.625).abs() < 1e-12);
    assert!((c.weight(150.0, l) - 0.125).abs() < 1e-12);
    assert_eq!(c.weight(250.0, l), 0.0);
    assert!(Falloff::Custom(vec![(1.0, 0.5), (0.5, 0.2)]).validate().is_err(), "x must increase");
    assert!(Falloff::Custom(vec![(0.0, 1.5)]).validate().is_err(), "w in [0,1]");
    // monotone non-increasing for the built-in smooth curves
    for f in [Falloff::Linear, Falloff::Gaussian, Falloff::InverseSquare] {
        let mut prev = 2.0;
        for k in 0..200 {
            let w = f.weight(k as f64 * 5.0, l);
            assert!(w <= prev + 1e-15, "{} not monotone", f.name());
            prev = w;
        }
    }
}

#[test]
fn compiled_fields_fall_off_exactly_like_the_curve() {
    let d = plane(30, 5);
    let c = cell_at(&d, 2, 2, 30);
    for (fo, name) in [(Falloff::Linear, "lin"), (Falloff::Gaussian, "gauss"), (Falloff::InverseSquare, "inv"), (Falloff::Custom(vec![(0.0, 1.0), (2.0, 0.2)]), "cust")] {
        let l = 60_000.0;
        let a = Anomaly::new(name, Shape::Region { cells: vec![c as u32] }, fo.clone(), l).with_effect(Effect::add("temperature_source", 80.0));
        let mut set = AnomalySet::new();
        set.add(a).unwrap();
        let comp = set.compile(&d, None).unwrap();
        let b = &comp.hooks["temperature_source"].b;
        for x in 0..30 {
            let i = cell_at(&d, x, 2, 30);
            let dist = (x as f64 - 2.0).abs() * 10_000.0;
            let expect = 80.0 * fo.weight(dist, l);
            assert!((b[i] - expect).abs() < 1e-9, "{name} at x={x}: {} vs {expect}", b[i]);
        }
    }
}

// ------------------------------------------------------------------ operations and composition

#[test]
fn add_multiply_override_and_composition() {
    let d = plane(8, 3);
    let g = Shape::Global;
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("mul", g.clone(), Falloff::Hard, 0.0).with_effect(Effect::multiply("albedo", 2.0))).unwrap();
    set.add(Anomaly::new("add", g.clone(), Falloff::Hard, 0.0).with_effect(Effect::add("albedo", 0.1))).unwrap();
    let c = set.compile(&d, None).unwrap();
    let m = &c.hooks["albedo"];
    // (v * 2) + 0.1, applied in insertion order
    assert!((m.a[0] * 0.3 + m.b[0] - 0.7).abs() < 1e-12);
    // override halfway: v' = 0.5 v + 0.5 * 1.0
    let mut s2 = AnomalySet::new();
    s2.add(Anomaly::new("o", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::new("albedo", Op::Override, 1.0))).unwrap();
    let c2 = s2.compile(&d, None).unwrap();
    assert!((c2.hooks["albedo"].a[0] * 0.3 + c2.hooks["albedo"].b[0] - 1.0).abs() < 1e-12, "weight 1 overrides fully");
    // source hooks keep only b
    assert_eq!(hook_kind("temperature_source"), Some(HookKind::Source));
    assert_eq!(hook_kind("custom:mana"), Some(HookKind::Source));
    assert_eq!(hook_kind("nonsense"), None);
    // unknown hook / duplicate name / bad magnitude are rejected
    let mut s3 = AnomalySet::new();
    s3.add(Anomaly::new("bad", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::add("nonsense", 1.0))).unwrap();
    assert!(s3.compile(&d, None).is_err());
    assert!(s3.add(Anomaly::new("bad", Shape::Global, Falloff::Hard, 0.0)).is_err());
    let mut s4 = AnomalySet::new();
    s4.add(Anomaly::new("nan", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::add("albedo", f64::NAN))).unwrap();
    assert!(s4.compile(&d, None).is_err());
}

#[test]
fn muted_anomalies_compile_to_identity() {
    let d = plane(8, 3);
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("x", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::multiply("albedo", 3.0)).with_effect(Effect::add("temperature_source", 50.0)).muted(true)).unwrap();
    let c = set.compile(&d, None).unwrap();
    assert!(c.hooks.values().all(|m| m.is_identity()));
    set.set_muted("x", false).unwrap();
    assert!(set.compile(&d, None).unwrap().hooks.values().any(|m| !m.is_identity()));
    assert!(set.set_muted("nope", true).is_err());
}

// ------------------------------------------------------------------ rules

#[test]
fn rules_scale_the_weight() {
    let d = plane(6, 3);
    let mk = |rule: Rule, t: f64| {
        let mut s = AnomalySet::new();
        s.time_s = t;
        s.add(Anomaly::new("r", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::add("temperature_source", 10.0)).with_rule(rule)).unwrap();
        s.compile(&d, None).unwrap().hooks["temperature_source"].b[0]
    };
    let per = Rule::Periodic { period_s: 100.0, duty: 0.25, phase: 0.0, low: 0.1 };
    assert_eq!(mk(per.clone(), 10.0), 10.0);
    assert!((mk(per.clone(), 60.0) - 1.0).abs() < 1e-12);
    assert_eq!(mk(per, 110.0), 10.0, "periodic");
    let script = Rule::Script(ScriptRule(std::sync::Arc::new(|ctx: &RuleContext| if ctx.time_s > 5.0 { 0.5 } else { 0.0 })));
    assert_eq!(mk(script.clone(), 1.0), 0.0);
    assert_eq!(mk(script, 9.0), 5.0);
    // conditional on a field mean
    let mut f = FieldStore::new(d.n_cells());
    f.put("temperature", 1, vec![300.0; d.n_cells()]).unwrap();
    let mut s = AnomalySet::new();
    s.add(Anomaly::new("w", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::add("temperature_source", 10.0)).with_rule(Rule::When { field: "temperature".into(), at_least: 290.0, factor_on: 1.0, factor_off: 0.0 })).unwrap();
    assert_eq!(s.compile(&d, Some(&f)).unwrap().hooks["temperature_source"].b[0], 10.0);
    f.put("temperature", 1, vec![250.0; d.n_cells()]).unwrap();
    assert_eq!(s.compile(&d, Some(&f)).unwrap().hooks["temperature_source"].b[0], 0.0);
    assert_eq!(s.compile(&d, None).unwrap().hooks["temperature_source"].b[0], 0.0, "no field: factor_off");
    assert!(Rule::Periodic { period_s: 0.0, duty: 0.5, phase: 0.0, low: 0.0 }.validate().is_err());
}

// ------------------------------------------------------------------ gravity through the Domain

#[test]
fn gravity_anomaly_changes_the_domain_not_the_modules() {
    let d = plane(10, 4);
    let c = cell_at(&d, 5, 2, 10);
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("heavy", Shape::Region { cells: vec![c as u32] }, Falloff::Linear, 30_000.0).with_effect(Effect::multiply("gravity", 3.0))).unwrap();
    let e = set.effective_domain(&d).unwrap();
    assert!((e.gravity_magnitude(c) - 3.0 * 9.81).abs() < 1e-9);
    assert!(e.gravity_magnitude(cell_at(&d, 6, 2, 10)) > 9.81 && e.gravity_magnitude(cell_at(&d, 6, 2, 10)) < 3.0 * 9.81);
    assert!((e.gravity_magnitude(cell_at(&d, 9, 2, 10)) - 9.81).abs() < 1e-12, "outside the reach");
    assert!(e.gravity[c][2] < 0.0, "direction kept (down)");
    set.set_muted("heavy", true).unwrap();
    let m = set.effective_domain(&d).unwrap();
    assert!(m.gravity == d.gravity, "muted: exact clone");
    // non-positive gravity is refused
    let mut s2 = AnomalySet::new();
    s2.add(Anomaly::new("zero", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::set("gravity", 0.0))).unwrap();
    assert!(s2.effective_domain(&d).is_err());
}

// ------------------------------------------------------------------ mute restores the baseline EXACTLY (full pipeline)

const ALL_SOURCE_AND_MODIFIER_HOOKS: [&str; 6] = ["temperature_source", "albedo", "evaporation", "wind_forcing", "pressure", "roughness"];

fn run_pipeline(handle: Option<&Anomalies>) -> FieldStore {
    let (base, mut f) = earth_like_world(3).unwrap();
    let dom = match handle {
        Some(h) => h.effective_domain(&base).unwrap(),
        None => base.clone(),
    };
    let mut s = Scheduler::new();
    if let Some(h) = handle {
        s.register(Box::new(AnomalyStage::new(h.clone()).with_hooks(&ALL_SOURCE_AND_MODIFIER_HOOKS))).unwrap();
    }
    add_climate_pipeline(&mut s).unwrap();
    run_season(&dom, &mut f, &mut s, 0.0).unwrap();
    f
}

fn assert_same_world(a: &FieldStore, b: &FieldStore) {
    let mut n = 0;
    for name in a.names() {
        let (x, y) = (a.data(&name).unwrap(), b.data(&name).unwrap());
        assert_eq!(x.len(), y.len(), "{name}");
        for i in 0..x.len() {
            assert!(x[i].to_bits() == y[i].to_bits(), "field {name}[{i}]: {} vs {}", x[i], y[i]);
        }
        n += 1;
    }
    assert!(n > 20, "compared {n} fields");
}

fn demo_set(base: &Domain) -> AnomalySet {
    let c = base.mesh.centroid[base.n_cells() / 3];
    let mut set = AnomalySet::new();
    set.add(
        Anomaly::new("hot_spot", Shape::Sphere { center: c, radius_m: 600_000.0 }, Falloff::Gaussian, 800_000.0)
            .with_effect(Effect::add("temperature_source", 120.0))
            .with_effect(Effect::multiply("evaporation", 2.0))
            .with_effect(Effect::add("albedo", 0.2).with_reach(0.5))
            .with_effect(Effect::add("wind_forcing", -300.0))
            .with_effect(Effect::multiply("gravity", 2.0))
            .with_effect(Effect::multiply("pressure", 1.05)),
    )
    .unwrap();
    set
}

#[test]
fn muting_restores_the_baseline_world_exactly() {
    let (base, _) = earth_like_world(3).unwrap();
    let baseline = run_pipeline(None);

    // an anomaly stage with NO anomalies is the baseline, bit for bit (apart from the identity hook fields)
    let empty = Anomalies::new();
    let with_empty = run_pipeline(Some(&empty));
    for name in baseline.names() {
        assert_eq!(baseline.data(&name).unwrap().iter().map(|v| v.to_bits()).collect::<Vec<_>>(), with_empty.data(&name).unwrap().iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{name}");
    }

    // with the anomaly: the world changes
    let h = Anomalies::from_set(demo_set(&base));
    let active = run_pipeline(Some(&h));
    let (t0, t1) = (baseline.data("temperature").unwrap(), active.data("temperature").unwrap());
    let dmax = (0..t0.len()).map(|i| (t1[i] - t0[i]).abs()).fold(0.0, f64::max);
    assert!(dmax > 1.0, "the anomaly must change temperature measurably (max {dmax} K)");

    // mute it: the baseline comes back exactly
    h.set_muted("hot_spot", true).unwrap();
    let muted = run_pipeline(Some(&h));
    for name in baseline.names() {
        let (x, y) = (baseline.data(&name).unwrap(), muted.data(&name).unwrap());
        assert!(x.iter().zip(y).all(|(a, b)| a.to_bits() == b.to_bits()), "field '{name}' differs after muting");
    }
    // un-mute again: same as before
    h.set_muted("hot_spot", false).unwrap();
    let again = run_pipeline(Some(&h));
    assert_same_world(&active, &again);
}

// ------------------------------------------------------------------ inspector

#[test]
fn inspector_lists_mutes_and_maps_impact() {
    let (base, _) = earth_like_world(3).unwrap();
    let mut set = demo_set(&base);
    set.add(Anomaly::new("cold_line", Shape::Line { a: base.mesh.centroid[0], b: base.mesh.centroid[50], half_width_m: 100_000.0 }, Falloff::Linear, 300_000.0).with_effect(Effect::add("temperature_source", -40.0)).muted(true)).unwrap();
    let list = inspect(&base, &set).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "hot_spot");
    assert!(!list[0].muted && list[1].muted);
    assert_eq!(list[0].shape, "sphere");
    assert_eq!(list[0].falloff, "gaussian");
    assert!(list[0].hooks.contains(&"gravity".to_string()));
    assert!(list[0].cells_influenced > 0 && list[0].cells_influenced < base.n_cells());
    assert!(list[0].area_influenced_m2 > 0.0 && list[0].reach_m > 0.0);
    // per-hook impact map: Add -> w*m, Multiply -> w*(m-1)
    let im = impact_map(&base, set.get("hot_spot").unwrap()).unwrap();
    let ts = &im["temperature_source"];
    let ev = &im["evaporation"];
    let i = (0..ts.len()).max_by(|&a, &b| ts[a].partial_cmp(&ts[b]).unwrap()).unwrap();
    assert!((ts[i] - 120.0).abs() < 1e-9, "core carries the full magnitude");
    assert!((ev[i] - 1.0).abs() < 1e-9, "multiply by 2 = +100% relative");
    assert!(ts.iter().all(|&v| v >= 0.0));
    // the impact map ignores the mute state
    assert!(impact_map(&base, set.get("cold_line").unwrap()).unwrap()["temperature_source"].iter().any(|&v| v < 0.0));
}

#[test]
fn field_impact_resolves_with_and_without_and_restores_mute() {
    let (base, _) = earth_like_world(3).unwrap();
    let h = Anomalies::from_set(demo_set(&base));
    h.set_muted("hot_spot", true).unwrap();
    let impact = field_impact(&h, "hot_spot", "temperature", |hh| Ok(run_pipeline(Some(hh)))).unwrap();
    assert!(impact.iter().cloned().fold(0.0, f64::max) > 1.0, "heating shows up in temperature");
    assert!(h.snapshot().get("hot_spot").unwrap().muted, "mute state restored");
    assert!(field_impact(&h, "nope", "temperature", |hh| Ok(run_pipeline(Some(hh)))).is_err());
}

#[test]
fn stage_rejects_undeclared_hooks_and_validates_params() {
    let d = plane(6, 3);
    let h = Anomalies::new();
    let mut stage = AnomalyStage::new(h.clone());
    h.add(Anomaly::new("late", Shape::Global, Falloff::Hard, 0.0).with_effect(Effect::add("temperature_source", 1.0))).unwrap();
    let mut f = FieldStore::new(d.n_cells());
    use enbuilder::{DirtyRegion, Module};
    assert!(stage.run(&d, &mut f, &DirtyRegion::All).is_err(), "hook not declared at registration");
    let mut ok = AnomalyStage::new(h).with_hooks(&["temperature_source"]);
    ok.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.has("anom_temperature_source_b"));
    assert!(!f.has("anom_temperature_source_a"), "source hooks have no multiplier field");
    assert_eq!(ok.params().get_or("n_active", -1.0), 1.0);
    assert!(ok.set_param("n_active", 3.0).is_err());
}

// =====================================================================================================================
// Per-hook tests: each hook is fed through the REAL module with a compiled anomaly, no special-case code anywhere.
// =====================================================================================================================

use enbuilder::{DirtyRegion, Module, Tectonics, TerrainShaping};

fn globe() -> (Domain, FieldStore) { earth_like_world(3).unwrap() }

/// A Region anomaly on `cells` with a hard edge (weight 1 inside, 0 outside) carrying `effects`.
fn hard_set(cells: &[usize], effects: Vec<Effect>) -> AnomalySet {
    let mut a = Anomaly::new("t", Shape::Region { cells: cells.iter().map(|&c| c as u32).collect() }, Falloff::Hard, 0.0);
    for e in effects {
        a = a.with_effect(e);
    }
    let mut s = AnomalySet::new();
    s.add(a).unwrap();
    s
}

fn write_hooks(d: &Domain, set: &AnomalySet, f: &mut FieldStore) {
    set.compile(d, None).unwrap().write(f).unwrap();
}

#[test]
fn hook_light_intensity_scales_insolation() {
    let (d, f0) = globe();
    let cells: Vec<usize> = (0..20).collect();
    let run = |hook: bool| {
        let mut f = FieldStore::new(d.n_cells());
        f.put("elevation", 1, f0.data("elevation").unwrap().to_vec()).unwrap();
        if hook {
            write_hooks(&d, &hard_set(&cells, vec![Effect::multiply("light_intensity", 2.0)]), &mut f);
        }
        let mut s = Scheduler::new();
        s.register(Box::new(enbuilder::Insolation::new())).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        f.data("insolation").unwrap().to_vec()
    };
    let (a, b) = (run(false), run(true));
    for i in 0..d.n_cells() {
        if i < 20 {
            assert!((b[i] - 2.0 * a[i]).abs() < 1e-9 * a[i].max(1.0), "cell {i}");
        } else {
            assert_eq!(a[i].to_bits(), b[i].to_bits(), "cell {i} outside the anomaly is untouched");
        }
    }
}

#[test]
fn hook_albedo_and_temperature_source_move_the_energy_balance() {
    let (d, _) = globe();
    let eq: Vec<usize> = (0..d.n_cells()).filter(|&i| d.mesh.up[i][2].abs() < 0.2 && d.mesh.up[i][0] < -0.7).collect();
    assert!(!eq.is_empty());
    let temp = |effects: Vec<Effect>| {
        let (_, mut f) = globe();
        let h = Anomalies::from_set(if effects.is_empty() { AnomalySet::new() } else { hard_set(&eq, effects) });
        let mut s = Scheduler::new();
        s.register(Box::new(AnomalyStage::new(h.clone()).with_hooks(&["albedo", "temperature_source"]))).unwrap();
        enbuilder::add_energy_stage(&mut s).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (f.data("temperature").unwrap().to_vec(), f.data("albedo").unwrap().to_vec())
    };
    let (t0, a0) = temp(vec![]);
    let (t1, a1) = temp(vec![Effect::add("albedo", 0.3)]);
    let (t2, _) = temp(vec![Effect::add("temperature_source", 150.0)]);
    let (t3, a3) = temp(vec![Effect::add("albedo", 5.0)]);
    let m = |v: &Vec<f64>| eq.iter().map(|&i| v[i]).sum::<f64>() / eq.len() as f64;
    assert!(m(&t1) < m(&t0) - 2.0, "brighter surface is colder: {} vs {}", m(&t1), m(&t0));
    assert!(m(&t2) > m(&t0) + 2.0, "extra heat source is warmer");
    assert!(eq.iter().all(|&i| a1[i] > a0[i] && a1[i] <= 1.0));
    assert!(eq.iter().all(|&i| a3[i] == 1.0), "albedo is clamped to 1");
    let _ = t3;
}

#[test]
fn hook_pressure_scales_sea_level_pressure() {
    let (d, f0) = globe();
    let cells: Vec<usize> = (0..30).collect();
    let run = |hook: bool| {
        let mut f = FieldStore::new(d.n_cells());
        for n in ["elevation", "temperature", "temperature_sl"] {
            f.put(n, 1, f0.data(n).unwrap().to_vec()).unwrap();
        }
        if hook {
            write_hooks(&d, &hard_set(&cells, vec![Effect::multiply("pressure", 1.1)]), &mut f);
        }
        let mut s = Scheduler::new();
        s.register(Box::new(enbuilder::PressureModel::new())).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        f.data("pressure_sl").unwrap().to_vec()
    };
    let (a, b) = (run(false), run(true));
    assert!(cells.iter().all(|&i| (b[i] / a[i] - 1.1).abs() < 1e-12));
    assert!((30..d.n_cells()).all(|i| a[i].to_bits() == b[i].to_bits()));
}

#[test]
fn hook_gravity_pressure_exp_raises_pressure_only_when_asked() {
    let (base, f0) = globe();
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("heavy", Shape::Region { cells: vec![5] }, Falloff::Hard, 0.0).with_effect(Effect::multiply("gravity", 2.0))).unwrap();
    let eff = set.effective_domain(&base).unwrap();
    let run = |exp: f64| {
        let mut f = FieldStore::new(base.n_cells());
        for n in ["elevation", "temperature", "temperature_sl"] {
            f.put(n, 1, f0.data(n).unwrap().to_vec()).unwrap();
        }
        let mut s = Scheduler::new();
        s.register(Box::new(enbuilder::PressureModel::new())).unwrap();
        s.set_param("pressure", "gravity_pressure_exp", exp).unwrap();
        s.run(&eff, &mut f, &DirtyRegion::All).unwrap();
        f.data("pressure_sl").unwrap()[5]
    };
    assert!(run(1.0) > 1.8 * run(0.0), "exp 1 doubles the column weight: {} vs {}", run(1.0), run(0.0));
    assert!((run(0.5) / run(0.0)) > 1.3 && (run(0.5) / run(0.0)) < 1.5);
}

#[test]
fn hook_wind_forcing_and_roughness_change_the_wind() {
    let (d, _) = globe();
    let ring: Vec<usize> = (0..d.n_cells()).filter(|&i| d.mesh.up[i][0] > 0.5).collect();
    let mean_speed = |effects: Vec<Effect>| {
        let (_, mut f) = globe();
        // an equator-to-pole temperature contrast, so the pressure field (and the wind) is not zero
        let t: Vec<f64> = (0..d.n_cells()).map(|i| 300.0 - 45.0 * d.mesh.up[i][2].powi(2)).collect();
        f.put("temperature_sl", 1, t.clone()).unwrap();
        f.put("temperature", 1, t).unwrap();
        let h = Anomalies::from_set(if effects.is_empty() { AnomalySet::new() } else { hard_set(&ring, effects) });
        let mut s = Scheduler::new();
        s.register(Box::new(AnomalyStage::new(h).with_hooks(&["wind_forcing", "roughness"]))).unwrap();
        enbuilder::add_atmosphere_stage(&mut s).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        let w = f.data("wind_speed").unwrap();
        ring.iter().map(|&i| w[i]).sum::<f64>() / ring.len() as f64
    };
    let base = mean_speed(vec![]);
    let forced = mean_speed(vec![Effect::add("wind_forcing", -400.0)]);
    let rough = mean_speed(vec![Effect::add("roughness", 400.0)]);
    // a uniform forcing inside a hard-edged patch only acts at its rim, but it must register
    assert!((forced - base).abs() > 1e-3 * base.max(1e-6), "forcing changes the wind: {forced} vs {base}");
    assert!(rough < base, "more friction, slower wind: {rough} vs {base}");
}

#[test]
fn hook_evaporation_scales_the_rate() {
    let (d, _) = globe();
    let n = d.n_cells();
    let cells: Vec<usize> = (0..40).collect();
    let run = |hook: bool| {
        let mut f = FieldStore::new(n);
        f.put("temperature", 1, vec![295.0; n]).unwrap();
        f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
        f.put("elevation", 1, vec![-1000.0; n]).unwrap();
        f.put("wind_speed", 1, vec![6.0; n]).unwrap();
        if hook {
            write_hooks(&d, &hard_set(&cells, vec![Effect::multiply("evaporation", 3.0)]), &mut f);
        }
        let mut s = Scheduler::new();
        enbuilder::add_moisture_source_stage(&mut s).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        f.data("evaporation").unwrap().to_vec()
    };
    let (a, b) = (run(false), run(true));
    assert!(a[0] > 0.0);
    assert!(cells.iter().all(|&i| b[i] >= a[i] && b[i] <= 3.0 * a[i] + 1e-18), "tripled up to the max_evap cap");
    assert!(cells.iter().any(|&i| b[i] > 2.0 * a[i]));
    assert!((40..n).all(|i| a[i].to_bits() == b[i].to_bits()));
}

#[test]
fn hooks_elevation_rock_strength_and_sea_level_on_geology_modules() {
    let (d, _) = globe();
    let cells: Vec<usize> = (0..50).collect();
    let geo = |effects: Vec<Effect>| {
        let mut f = FieldStore::new(d.n_cells());
        let mut s = Scheduler::new();
        let h = Anomalies::from_set(if effects.is_empty() { AnomalySet::new() } else { hard_set(&cells, effects) });
        s.register(Box::new(AnomalyStage::new(h).with_hooks(&["elevation", "rock_strength"]))).unwrap();
        s.register(Box::new(Tectonics::new())).unwrap();
        s.register(Box::new(TerrainShaping::new())).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (f.data("elevation").unwrap().to_vec(), f.data("rock_strength").unwrap().to_vec())
    };
    let (z0, r0) = geo(vec![]);
    let (z1, _) = geo(vec![Effect::add("elevation", 1000.0)]);
    let (_, r1) = geo(vec![Effect::multiply("rock_strength", 0.5)]);
    assert!(cells.iter().all(|&i| (z1[i] - z0[i] - 1000.0).abs() < 1e-9));
    assert!((50..d.n_cells()).all(|i| z1[i].to_bits() == z0[i].to_bits()));
    assert!(cells.iter().all(|&i| (r1[i] - 0.5 * r0[i]).abs() < 1e-12));
    // sea level
    let mut f = FieldStore::new(d.n_cells());
    write_hooks(&d, &hard_set(&cells, vec![Effect::add("sea_level", 120.0)]), &mut f);
    let mut sl = enbuilder::SeaLevel::new();
    sl.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let v = f.data("sea_level").unwrap();
    assert!(cells.iter().all(|&i| v[i] == 120.0) && (50..d.n_cells()).all(|i| v[i] == 0.0));
}

#[test]
fn hook_time_flow_rate_speeds_up_erosion_locally() {
    use enbuilder::{add_erosion_module, seed_erosion_state};
    let d = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 6, ny: 3 }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap();
    let n = d.n_cells();
    let a = 10_000.0f64 * 10_000.0;
    let z = [-10.0, 100.0, 200.0, 300.0, 400.0, 500.0];
    let build = |hook: bool| {
        let (mut zz, mut fd, mut ocean, mut area) = (vec![1000.0; n], vec![-1.0; n], vec![1.0; n], vec![0.0; n]);
        for i in 0..6 {
            let c = 6 + i;
            zz[c] = z[i];
            if i >= 1 {
                fd[c] = (c - 1) as f64;
                ocean[c] = 0.0;
                area[c] = (6 - i) as f64 * a;
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
        f.put("rock_strength", 1, vec![1.0; n]).unwrap();
        seed_erosion_state(&mut f).unwrap();
        if hook {
            write_hooks(&d, &hard_set(&[8, 9], vec![Effect::multiply("time_flow_rate", 5.0)]), &mut f);
        }
        let mut s = Scheduler::new();
        add_erosion_module(&mut s).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        f.data("erosion_step").unwrap().to_vec()
    };
    let (s0, s1) = (build(false), build(true));
    for c in [8, 9] {
        assert!(s1[c] < s0[c] && s0[c] < 0.0, "cell {c}: faster incision ({} vs {})", s1[c], s0[c]);
    }
    assert!(s1[11] <= 0.0 && s0[11] <= 0.0);
}

// ------------------------------------------------------------------ the high-gravity example (PART_07 build item 3)

/// One Gaussian gravity anomaly (x`factor`) on the Earth-like sphere; returns (base domain, effective domain, inside cells, ring cells).
fn heavy_world(level: u32, factor: f64) -> (Domain, Domain, Vec<usize>, Vec<usize>) {
    let (base, _) = earth_like_world(level).unwrap();
    let ax = [0.3f64, 0.5, 0.8];
    let al = (ax[0] * ax[0] + ax[1] * ax[1] + ax[2] * ax[2]).sqrt();
    let c = [ax[0] / al * 6.371e6, ax[1] / al * 6.371e6, ax[2] / al * 6.371e6];
    let shape = Shape::Sphere { center: c, radius_m: 900_000.0 };
    let mut s = AnomalySet::new();
    s.add(Anomaly::new("heavy", shape.clone(), Falloff::Gaussian, 700_000.0).with_effect(Effect::multiply("gravity", factor))).unwrap();
    let dom = s.effective_domain(&base).unwrap();
    let dist = shape.distances(&base).unwrap();
    let inside: Vec<usize> = (0..base.n_cells()).filter(|&i| dist[i] == 0.0).collect();
    let ring: Vec<usize> = (0..base.n_cells()).filter(|&i| dist[i] > 0.0 && dist[i] < 1_000_000.0).collect();
    (base, dom, inside, ring)
}

fn mean_at(v: &[f64], idx: &[usize]) -> f64 { idx.iter().map(|&i| v[i]).sum::<f64>() / idx.len() as f64 }

#[test]
fn high_gravity_example_pressure_wind_relief_rivers_and_canopy_with_no_special_case_code() {
    use enbuilder::biomes::{BiomeDef, BiomeTable, Biomes};
    use enbuilder::hydraulics::add_river_velocity_stage;
    use enbuilder::{DirtyRegion, Module, Tectonics, TerrainShaping};

    // ---- pressure + wind: the climate pipeline run on the effective domain, only `gravity_pressure_exp` set
    let (base, heavy, inside, ring) = heavy_world(4, 1.5);
    assert!(inside.len() >= 10 && ring.len() >= 10, "zone must contain cells ({} / {})", inside.len(), ring.len());
    let climate = |dom: &Domain| {
        let (_, mut f) = earth_like_world(4).unwrap();
        let mut s = Scheduler::new();
        add_climate_pipeline(&mut s).unwrap();
        s.set_param("pressure", "gravity_pressure_exp", 0.1).unwrap();
        run_season(dom, &mut f, &mut s, 0.0).unwrap();
        (f.data("pressure_sl").unwrap().to_vec(), f.data("wind_speed").unwrap().to_vec(), f.data("wind_divergence").unwrap().to_vec())
    };
    let (p0, w0, d0) = climate(&base);
    let (p1, w1, d1) = climate(&heavy);
    let dp = mean_at(&p1, &inside) / mean_at(&p0, &inside);
    assert!(dp > 1.02 && dp < 1.08, "sea-level pressure in the zone rises a few percent, got x{dp}");
    assert!(mean_at(&w1, &ring) > 1.3 * mean_at(&w0, &ring), "winds around the zone accelerate: {} vs {}", mean_at(&w1, &ring), mean_at(&w0, &ring));
    // Recorded deviation from the master prompt: the engine gives OUTFLOW (divergence rises) from the high, not an inflow sink.
    assert!(mean_at(&d1, &inside) > mean_at(&d0, &inside), "air leaves the high-pressure zone");

    // ---- relief: tectonics + terrain on the effective domain (gravity x2.5)
    let (b2, h2, in2, _) = heavy_world(4, 2.5);
    let relief = |dom: &Domain| {
        let mut f = FieldStore::new(b2.n_cells());
        let mut s = Scheduler::new();
        s.register(Box::new(Tectonics::new())).unwrap();
        s.register(Box::new(TerrainShaping::new())).unwrap();
        s.run(dom, &mut f, &DirtyRegion::All).unwrap();
        let z = f.data("elevation").unwrap().to_vec();
        let m = mean_at(&z, &in2);
        let std = (in2.iter().map(|&i| (z[i] - m).powi(2)).sum::<f64>() / in2.len() as f64).sqrt();
        (in2.iter().map(|&i| z[i]).fold(f64::MIN, f64::max), std)
    };
    let (zmax0, std0) = relief(&b2);
    let (zmax1, std1) = relief(&h2);
    assert!(zmax1 < zmax0 && std1 < std0, "relief is lower under high gravity: max {zmax1} < {zmax0}, std {std1} < {std0}");

    // ---- rivers: identical discharge, mask and slope; only the Domain differs -> v scales with sqrt(g)
    let n = b2.n_cells();
    let river = |dom: &Domain| {
        let mut f = FieldStore::new(n);
        f.put("discharge", 1, vec![500.0; n]).unwrap();
        f.put("river_mask", 1, vec![1.0; n]).unwrap();
        f.put("slope", 1, vec![1e-3; n]).unwrap();
        let mut s = Scheduler::new();
        add_river_velocity_stage(&mut s).unwrap();
        s.run(dom, &mut f, &DirtyRegion::All).unwrap();
        f.data("river_velocity").unwrap().to_vec()
    };
    let (v0, v1) = (river(&b2), river(&h2));
    let ratio = v1[in2[0]] / v0[in2[0]];
    assert!((ratio - 2.5f64.sqrt()).abs() < 0.02, "river speed scales as sqrt(g): x{ratio}");

    // ---- canopy: one tall biome everywhere; the gravity cap alone shortens it inside the zone
    let table = BiomeTable::new(vec![BiomeDef { name: "forest".into(), color: [0, 99, 0], albedo: 0.15, evap_factor: 1.0, roughness_m: 30.0, conditions: vec![] }]);
    let canopy = |dom: &Domain| {
        let mut f = FieldStore::new(n);
        Biomes::new(table.clone()).run(dom, &mut f, &DirtyRegion::All).unwrap();
        f.data("biome_roughness_m").unwrap().to_vec()
    };
    let (c0, c1) = (canopy(&b2), canopy(&h2));
    assert!(c0.iter().all(|&c| c == 30.0), "baseline canopy untouched");
    assert!((c1[in2[0]] - 12.0).abs() < 0.5, "30 m forest under x2.5 gravity -> ~12 m, got {}", c1[in2[0]]);
    let far = (0..n).find(|&i| c1[i] == 30.0).expect("cells outside the zone keep full height");
    assert!(c1[far] >= c1[in2[0]]);
}
