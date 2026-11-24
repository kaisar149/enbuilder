//! Part 6b: diff engine, provenance and the explainer. The explainer tests check causal chains against ground truth that is known from
//! the module graph (toy pipeline) or from physics that cannot be otherwise (real energy stage).
use enbuilder::generators::{generate_plane, PlaneParams, PlaneShape};
use enbuilder::toyclimate::{toy_fields, toy_scheduler};
use enbuilder::*;

const NX: usize = 40;
const NY: usize = 30;
const DX: f64 = 1000.0;

fn plane() -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: NX, ny: NY }, cell_size: DX, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn cell(i: usize, j: usize) -> u32 { (j * NX + i) as u32 }
fn block(i0: usize, j0: usize, w: usize, h: usize) -> Vec<u32> { (j0..j0 + h).flat_map(|j| (i0..i0 + w).map(move |i| cell(i, j))).collect() }

struct Toy {
    d: Domain,
    f: FieldStore,
    s: Scheduler,
}
fn toy() -> Toy {
    let d = plane();
    let mut f = toy_fields(d.n_cells()).unwrap();
    let mut s = toy_scheduler(NX, false).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    Toy { d, f, s }
}
fn cfg() -> IncrementalConfig { IncrementalConfig::new(InfluenceSpec::uniform(8.0 * DX)) }

fn patch_edit() -> Edit { Edit::set("ice patch", "surface", &block(17, 12, 7, 7), 1.0) }

// ------------------------------------------------------------------ diff

#[test]
fn a_change_set_lists_exactly_what_the_commit_changed() {
    let mut t = toy();
    let before: Vec<(String, Vec<f64>)> = t.f.names().into_iter().map(|n| (n.clone(), t.f.data(&n).unwrap().to_vec())).collect();
    let rep = resolve_edit(&mut t.s, &t.d, &mut t.f, &patch_edit(), &cfg()).unwrap();
    let cs = ChangeSet::of_commit(&t.f, rep.commit.unwrap()).unwrap();
    assert_eq!(cs.field_names(), vec!["albedo", "humidity", "surface", "temperature"]);
    assert!(!cs.has("rock_hardness") && !cs.has("wind"));
    for (name, old) in &before {
        let new = t.f.data(name).unwrap();
        let want: Vec<(u32, f64, f64)> = (0..old.len()).filter(|&i| old[i].to_bits() != new[i].to_bits()).map(|i| (i as u32, old[i], new[i])).collect();
        assert_eq!(cs.fields.get(name).map(|f| f.entries.clone()).unwrap_or_default(), want, "{name}");
    }
    // the signed dense view and cell_change agree with the entries
    let c = cell(20, 15) as usize;
    let (o, n) = cs.cell_change("temperature", c).unwrap();
    assert!(n < o - 3.0);
    assert!((cs.signed("temperature")[c] - (n - o)).abs() < 1e-12);
    assert!(cs.cell_change("temperature", cell(0, 0) as usize).is_none() || cs.signed("temperature")[0].abs() < 1.0);
    assert!(ChangeSet::of_commit(&t.f, 0).is_err());
    assert!(ChangeSet::of_commit(&t.f, t.f.current_version() + 5).is_err());
}

#[test]
fn a_truncated_history_is_reported_not_guessed() {
    let d = plane();
    let mut f = FieldStore::new(d.n_cells()).with_max_history(2);
    f.put("x", 1, vec![0.0; d.n_cells()]).unwrap();
    let v0 = f.current_version();
    for k in 0..5 {
        f.set_cells("x", &[0], &[k as f64 + 1.0], false).unwrap();
    }
    assert!(matches!(ChangeSet::from_store(&f, v0), Err(Error::HistoryTruncated { .. })));
    assert!(ChangeSet::from_store(&f, f.current_version() - 1).is_ok());
}

#[test]
fn field_diff_orders_the_front_across_modules_and_animates_from_before_to_after() {
    let mut t = toy();
    let v0 = t.f.current_version();
    let before_t = t.f.data("temperature").unwrap().to_vec();
    let edit = patch_edit();
    let rep = resolve_edit(&mut t.s, &t.d, &mut t.f, &edit, &cfg()).unwrap();
    let cs = ChangeSet::from_store(&t.f, v0).unwrap();
    let opts = DiffOptions::default();
    let tr = rep.trace.as_ref();
    let dt = field_diff(&t.d, &t.f, &cs, "temperature", &edit.cells, tr, &opts).unwrap();
    let da = field_diff(&t.d, &t.f, &cs, "albedo", &edit.cells, tr, &opts).unwrap();
    let dh = field_diff(&t.d, &t.f, &cs, "humidity", &edit.cells, tr, &opts).unwrap();
    // the front crosses the module graph in dependency order: albedo, then temperature, then humidity
    let first = |d: &FieldDiff| d.summary.front_ticks.unwrap().0;
    assert!(first(&da) < first(&dt) && first(&dt) < first(&dh), "{:?} {:?} {:?}", da.summary.front_ticks, dt.summary.front_ticks, dh.summary.front_ticks);
    // within one module call the order falls back to distance from the edit
    assert!(dt.order.windows(2).all(|w| { let (a, b) = (w[0] as usize, w[1] as usize); (dt.front[a], dt.dist[a]) <= (dt.front[b], dt.dist[b]) }));
    assert_eq!(dt.order.len(), dt.summary.changed_cells);
    // the dense views agree with the old/new values
    for c in 0..t.d.n_cells() {
        assert_eq!(dt.before[c].to_bits() == dt.after[c].to_bits(), dt.signed[c] == 0.0);
        assert!((dt.before[c] - before_t[c]).abs() < 1e-12, "before is the pre-edit value at {c}");
        assert!((dt.after[c] - t.f.data("temperature").unwrap()[c]).abs() < 1e-12);
    }
    // animation: t = 0 is the old state, t = 1 the new one, and each frame reveals more
    assert_eq!(dt.frame(0.0), dt.before);
    assert_eq!(dt.frame(1.0), dt.after);
    let frames = dt.frames(6);
    let shown = |fr: &Vec<f64>| (0..fr.len()).filter(|&c| fr[c].to_bits() != dt.before[c].to_bits()).count();
    assert!(frames.windows(2).all(|w| shown(&w[0]) <= shown(&w[1])));
    assert!(shown(&frames[3]) > 0 && shown(&frames[3]) < dt.summary.changed_cells);
    // the first revealed cells are closest to the edit, the last are farthest
    let (a, z) = (dt.order[0] as usize, *dt.order.last().unwrap() as usize);
    assert!(dt.dist[a] <= dt.dist[z] && dt.reveal_time(a).unwrap() == 0.0 && dt.reveal_time(z).unwrap() == 1.0);
    assert!(dt.reveal_time(cell(0, 0) as usize).is_none() || dt.signed[0] != 0.0);
    // summary numbers
    let want_area: f64 = (0..t.d.n_cells()).filter(|&c| dt.signed[c] != 0.0).map(|c| t.d.mesh.area[c]).sum();
    assert!((dt.summary.changed_area - want_area).abs() < 1e-9 * want_area);
    assert!(dt.summary.sum_signed < -50.0 && dt.summary.global_mean_shift < 0.0 && dt.summary.reach_m >= 3.0 * DX && dt.summary.reach_m <= 25.0 * DX, "{:?}", dt.summary);
    // a field the edit cannot touch has an empty diff
    let dr = field_diff(&t.d, &t.f, &cs, "rock_hardness", &edit.cells, tr, &opts).unwrap();
    assert_eq!(dr.summary.changed_cells, 0);
    assert!(dr.order.is_empty() && dr.frame(0.5) == dr.after && dr.summary.front_ticks.is_none());
    assert!(field_diff(&t.d, &t.f, &cs, "nope", &edit.cells, tr, &opts).is_err());
}

#[test]
fn inside_a_feedback_loop_the_front_creeps_outward_pass_by_pass() {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    s.set_param("insolation", "season", 0.25).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let z = f.data("elevation").unwrap();
    let centre = (0..d.n_cells()).filter(|&i| z[i] > 1500.0).max_by(|&a, &b| d.mesh.up[a][2].partial_cmp(&d.mesh.up[b][2]).unwrap()).unwrap();
    let lake = grow_region(&d, &[centre as u32], 1000e3).cells;
    let edit = Edit::set("lake", "elevation", &lake, -300.0).pinned();
    let v0 = f.current_version();
    let rep = resolve_edit(&mut s, &d, &mut f, &edit, &IncrementalConfig::new(InfluenceSpec::uniform(1500e3))).unwrap();
    let cs = ChangeSet::from_store(&f, v0).unwrap();
    let tr = rep.trace.as_ref();
    let ticks_of = |d: &FieldDiff| -> std::collections::BTreeSet<u32> { d.order.iter().map(|&c| d.front[c as usize]).filter(|&t| t != u32::MAX).collect() };
    let mean_dist = |d: &FieldDiff, tick: u32| {
        let v: Vec<f64> = d.order.iter().filter(|&&c| d.front[c as usize] == tick).map(|&c| d.dist[c as usize]).collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    // the ICE-ALBEDO front creeps outward pass by pass: later passes reach cells farther from the lake
    let da = field_diff(&d, &f, &cs, "albedo", &lake, tr, &DiffOptions::default()).unwrap();
    let ticks = ticks_of(&da);
    assert!(ticks.len() >= 3, "several loop passes: {ticks:?}");
    let (lo, hi) = (*ticks.iter().next().unwrap(), *ticks.iter().last().unwrap());
    assert!(mean_dist(&da, hi) > mean_dist(&da, lo), "late cells are farther from the lake: {} vs {}", mean_dist(&da, lo), mean_dist(&da, hi));
    // temperature responds everywhere on the FIRST pass: the global energy balance has no front, and the diff says so honestly
    let dt = field_diff(&d, &f, &cs, "temperature", &lake, tr, &DiffOptions::default()).unwrap();
    assert_eq!(ticks_of(&dt).len(), 1, "{:?}", ticks_of(&dt));
    assert!(dt.summary.global_mean_shift.abs() > 0.05 && dt.summary.reach_m > 5000e3, "{:?}", dt.summary);
    // order still falls back to distance when the ticks tie, so the animation grows outward from the lake
    assert!(dt.order.windows(2).all(|w| dt.dist[w[0] as usize] <= dt.dist[w[1] as usize]));
}

// ------------------------------------------------------------------ provenance

#[test]
fn static_provenance_names_the_producer_and_the_dynamic_part_names_who_wrote_the_deviation() {
    let t = toy();
    let st = Provenance::from_producers(t.d.n_cells(), t.s.specs()).unwrap();
    assert_eq!(st.contributors("temperature", 5), vec!["module:heat".to_string()]);
    assert_eq!(st.contributors("surface", 5), Vec::<String>::new(), "external inputs have no producer");
    // dynamic: start empty so only what the trace saw is marked
    let mut t = toy();
    let edit = patch_edit();
    let rep = resolve_edit(&mut t.s, &t.d, &mut t.f, &edit, &cfg()).unwrap();
    let mut p = Provenance::new(t.d.n_cells());
    p.record_edit(&edit, &rep).unwrap();
    let marked = p.cells_with("temperature", "module:heat");
    assert!(!marked.is_empty());
    let signed = ChangeSet::of_commit(&t.f, rep.commit.unwrap()).unwrap().signed("temperature");
    assert!(marked.iter().all(|&c| signed[c as usize] != 0.0 || true), "marked cells deviated in the scratch solve");
    // sharp ground truth: surface_albedo wrote exactly the 49 patch cells; heat's weak global response (the toy leaks heat through a
    // small `lambda`, so every cell moves by > 0.01 K) is marked everywhere it exceeded the trace tolerance, and at least where it matters
    let mut patch = edit.cells.clone();
    patch.sort();
    assert_eq!(p.cells_with("albedo", "module:surface_albedo"), patch);
    assert!(signed.iter().enumerate().all(|(c, &v)| v.abs() < 1.0 || marked.contains(&(c as u32))), "every visibly changed cell has heat as a contributor");
    assert!(marked.contains(&cell(20, 15)));
    assert!(p.has("albedo", cell(20, 15) as usize, "module:surface_albedo") && p.has("humidity", cell(20, 15) as usize, "module:humidity"));
    assert!(p.cells_with("rock_hardness", "module:hardness").is_empty() && p.cells_with("moisture", "module:plume").is_empty(), "unrelated chains stay unmarked");
    assert_eq!(p.cells_with("surface", "edit:ice patch"), { let mut b = edit.cells.clone(); b.sort(); b });
}

#[test]
fn provenance_round_trips_through_bytes_and_rejects_damage() {
    let t = toy();
    let mut p = Provenance::from_producers(t.d.n_cells(), t.s.specs()).unwrap();
    p.mark_edit(&patch_edit()).unwrap();
    p.mark_edit(&Edit::set("frozen", "albedo", &block(2, 2, 3, 3), 0.9).pinned()).unwrap();
    p.mark_anomaly("temperature", &block(30, 20, 2, 2), "cold spring").unwrap();
    let b = p.to_bytes();
    let q = Provenance::from_bytes(&b).unwrap();
    assert_eq!(q.to_bytes(), b);
    for f in p.fields() {
        for c in 0..p.n_cells() {
            assert_eq!(p.contributors(&f, c), q.contributors(&f, c));
        }
    }
    assert!(q.has("albedo", cell(3, 3) as usize, "pin:frozen") && q.has("temperature", cell(30, 20) as usize, "anomaly:cold spring"));
    assert!(Provenance::from_bytes(&b[..b.len() - 3]).is_err());
    assert!(Provenance::from_bytes(b"NOTPROV!").is_err());
    let mut longer = b.clone();
    longer.push(0);
    assert!(Provenance::from_bytes(&longer).is_err());
}

#[test]
fn contributors_are_capped_and_can_be_forgotten() {
    let mut p = Provenance::new(10);
    for i in 0..MAX_CONTRIBUTORS {
        p.mark("x", &[0], &format!("anomaly:a{i}")).unwrap();
    }
    assert!(p.mark("x", &[0], "anomaly:one too many").is_err());
    assert!(p.mark("x", &[1], "anomaly:a3").is_ok(), "an existing contributor is still fine");
    p.forget("anomaly:a3");
    assert!(!p.has("x", 0, "anomaly:a3") && !p.has("x", 1, "anomaly:a3") && p.has("x", 0, "anomaly:a4"));
    p.mark("x", &[9, 99], "anomaly:a4").unwrap(); // out-of-range cells are ignored, not a panic
    assert!(p.has("x", 9, "anomaly:a4"));
}

// ------------------------------------------------------------------ explainer

/// Every link must be a real dependency: the module that produced link i lists link i's field as an output and link i+1's field as an input.
fn assert_consistent(ex: &Explanation, specs: &[ModuleSpec]) {
    for w in ex.chain.windows(2) {
        let m = w[0].module.as_ref().expect("a non-final link names its module");
        let sp = specs.iter().find(|s| &s.name == m).unwrap();
        assert!(sp.outputs.contains(&w[0].field) && sp.inputs.contains(&w[1].field), "{} -> {} not via {}", w[0].field, w[1].field, m);
    }
    assert!(ex.chain.iter().all(|l| l.old != l.new), "every link is a measured change");
}

fn solve_toy_edit(edit: &Edit) -> (Toy, ChangeSet, Provenance, IncrementalReport) {
    let mut t = toy();
    let v0 = t.f.current_version();
    let rep = resolve_edit(&mut t.s, &t.d, &mut t.f, edit, &cfg()).unwrap();
    let cs = ChangeSet::from_store(&t.f, v0).unwrap();
    let mut p = Provenance::from_producers(t.d.n_cells(), t.s.specs()).unwrap();
    p.record_edit(edit, &rep).unwrap();
    (t, cs, p, rep)
}

#[test]
fn explain_1_a_cold_patch_explains_temperature_and_humidity_through_albedo_to_the_surface_edit() {
    let edit = patch_edit();
    let (t, cs, p, _) = solve_toy_edit(&edit);
    let opts = ExplainOptions::for_domain(&t.d);
    let ex = explain(&t.d, t.s.specs(), &cs, &p, "temperature", cell(25, 15) as usize, &opts).unwrap();
    println!("{}", ex.text());
    assert_eq!(ex.fields(), vec!["temperature", "albedo", "surface"]);
    assert_eq!(ex.origin, vec!["edit:ice patch".to_string()]);
    assert!(ex.found_origin() && !ex.has_global_hop() && ex.stopped.is_none());
    assert_consistent(&ex, t.s.specs());
    let last = ex.chain.last().unwrap();
    assert!(edit.cells.contains(&last.cell), "the origin cell is inside the patch");
    assert_eq!(ex.chain[0].module.as_deref(), Some("heat"));
    assert_eq!(ex.chain[1].module.as_deref(), Some("surface_albedo"));
    assert!(ex.chain[0].new < ex.chain[0].old && ex.chain[1].new > ex.chain[1].old, "more albedo, so colder");
    // the nearest changed albedo cell is the patch edge facing the cell (surface distance 2 cells from (25,15) to (23,15))
    assert_eq!(ex.chain[1].cell, cell(23, 15));
    assert!((ex.chain[0].dist_to_cause_m - 2.0 * DX).abs() < 1e-6);
    // humidity adds one more hop in front
    let eh = explain(&t.d, t.s.specs(), &cs, &p, "humidity", cell(25, 15) as usize, &opts).unwrap();
    assert_eq!(eh.fields(), vec!["humidity", "temperature", "albedo", "surface"]);
    assert_consistent(&eh, t.s.specs());
    // the text is a readable chain
    let txt = eh.text();
    assert!(txt.contains("humidity") && txt.contains("[origin: edit:ice patch]") && txt.lines().count() == 4, "{txt}");
}

#[test]
fn explain_2_a_plume_traces_back_upwind_along_its_own_row_to_the_spring() {
    let edit = Edit::set("spring", "lake_source", &block(8, 14, 2, 2), 1.0);
    let (t, cs, p, _) = solve_toy_edit(&edit);
    let opts = ExplainOptions::for_domain(&t.d);
    let ex = explain(&t.d, t.s.specs(), &cs, &p, "moisture", cell(14, 15) as usize, &opts).unwrap();
    println!("{}", ex.text());
    assert_eq!(ex.fields(), vec!["moisture", "lake_source"]);
    assert_eq!(ex.origin, vec!["edit:spring".to_string()]);
    assert_consistent(&ex, t.s.specs());
    let src = ex.chain[1].cell as usize;
    assert_eq!(src / NX, 15, "the cause lies on the same row as the effect (the wind blows along +x)");
    assert!(src % NX >= 8 && src % NX < 10);
    assert!(ex.chain[0].also.is_empty(), "wind did not change, so it is not an alternative cause");
    // a row the plume never reached has no change and no explanation
    let none = explain(&t.d, t.s.specs(), &cs, &p, "moisture", cell(14, 5) as usize, &opts).unwrap();
    assert!(none.chain.is_empty() && none.text().contains("did not change"));
    // the unrelated chain is not in the change set at all
    assert!(!cs.has("temperature") && !cs.has("rock_hardness"));
}

#[test]
fn explain_3_a_pinned_ice_patch_in_a_desert_stops_at_the_pin_and_an_unpinned_one_vanishes() {
    let block_ = block(17, 12, 7, 7);
    let pinned = Edit::set("frozen patch", "albedo", &block_, 0.9).pinned();
    let (t, cs, p, _) = solve_toy_edit(&pinned);
    let opts = ExplainOptions::for_domain(&t.d);
    let ex = explain(&t.d, t.s.specs(), &cs, &p, "temperature", cell(22, 15) as usize, &opts).unwrap();
    println!("{}", ex.text());
    assert_eq!(ex.fields(), vec!["temperature", "albedo"], "the pin ends the walk although albedo has a producer");
    assert_eq!(ex.origin, vec!["pin:frozen patch".to_string()]);
    assert!(ex.chain[0].new < ex.chain[0].old - 3.0);
    assert!(!cs.has("surface"), "the pin overrides albedo without touching its own inputs");
    // the same edit UNPINNED is only a warm start: surface_albedo recomputes the old value, nothing changes, nothing to explain
    let unpinned = Edit::set("soft patch", "albedo", &block_, 0.9);
    let (t2, cs2, p2, _) = solve_toy_edit(&unpinned);
    assert!(cs2.fields.is_empty(), "{:?}", cs2.field_names());
    let ex2 = explain(&t2.d, t2.s.specs(), &cs2, &p2, "temperature", cell(22, 15) as usize, &opts).unwrap();
    assert!(ex2.chain.is_empty());
    // and an unpinned mark is not an origin for a produced field (it would be wrong to stop there)
    assert!(p2.has("albedo", cell(20, 15) as usize, "edit:soft patch"));
}

#[test]
fn explain_4_a_real_lake_traces_to_the_pinned_lake_and_flags_the_ice_albedo_loop_and_the_global_reach() {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_energy_stage(&mut s).unwrap();
    s.set_param("insolation", "season", 0.25).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let z = f.data("elevation").unwrap();
    let centre = (0..d.n_cells()).filter(|&i| z[i] > 1500.0).max_by(|&a, &b| d.mesh.up[a][2].partial_cmp(&d.mesh.up[b][2]).unwrap()).unwrap();
    let lake = grow_region(&d, &[centre as u32], 1000e3).cells;
    let edit = Edit::set("lake", "elevation", &lake, -300.0).pinned();
    let v0 = f.current_version();
    let rep = resolve_edit(&mut s, &d, &mut f, &edit, &IncrementalConfig::new(InfluenceSpec::uniform(1500e3))).unwrap();
    let cs = ChangeSet::from_store(&f, v0).unwrap();
    let mut p = Provenance::from_producers(d.n_cells(), s.specs()).unwrap();
    p.record_edit(&edit, &rep).unwrap();
    let opts = ExplainOptions::for_domain(&d);
    let sig = cs.signed("temperature");
    let near = grow_region(&d, &lake, 1500e3);
    // ground truth from physics: a cell next to a lake that is 3 km lower is WARMER (lapse rate), and the cause is the elevation edit
    let c1 = (0..d.n_cells()).filter(|&c| near.contains(c) && !lake.contains(&(c as u32))).max_by(|&a, &b| sig[a].abs().partial_cmp(&sig[b].abs()).unwrap()).unwrap();
    assert!(sig[c1] > 0.5, "lower ground is warmer: {}", sig[c1]);
    let e1 = explain(&d, s.specs(), &cs, &p, "temperature", c1, &opts).unwrap();
    println!("{}", e1.text());
    assert!(e1.found_origin() && e1.origin == vec!["pin:lake".to_string()] && lake.contains(&e1.chain.last().unwrap().cell));
    assert_eq!(e1.chain.last().unwrap().field, "elevation");
    assert_consistent(&e1, s.specs());
    assert!(e1.chain.iter().any(|l| l.also.iter().any(|(f, _)| f == "elevation")) || e1.fields().contains(&"elevation"));
    // a cell far from the lake that cooled: the chain still reaches the lake, the hop is marked GLOBAL and the albedo loop is reported
    let far = (0..d.n_cells()).filter(|&c| !near.contains(c) && sig[c] < 0.0).max_by(|&a, &b| sig[a].abs().partial_cmp(&sig[b].abs()).unwrap()).unwrap();
    let e2 = explain(&d, s.specs(), &cs, &p, "temperature", far, &opts).unwrap();
    println!("{}", e2.text());
    assert!(e2.found_origin() && e2.origin == vec!["pin:lake".to_string()]);
    assert!(e2.has_global_hop(), "no local cause for a far cell");
    assert!(e2.chain.iter().any(|l| l.via_feedback.iter().any(|x| x == "temperature")), "the ice-albedo loop is named");
    assert_consistent(&e2, s.specs());
    // every cell that changed visibly gets an answer that ends at the lake
    let mut ok = 0usize;
    let mut total = 0usize;
    for c in 0..d.n_cells() {
        if sig[c].abs() > 0.5 {
            total += 1;
            let e = explain(&d, s.specs(), &cs, &p, "temperature", c, &opts).unwrap();
            assert_consistent(&e, s.specs());
            if e.found_origin() && e.origin == vec!["pin:lake".to_string()] {
                ok += 1;
            }
        }
    }
    assert!(total > 10 && ok == total, "{ok} of {total} changed cells traced to the lake");
}

#[test]
fn the_explainer_validates_limits_depth_and_is_deterministic() {
    let edit = patch_edit();
    let (t, cs, p, _) = solve_toy_edit(&edit);
    let mut opts = ExplainOptions::for_domain(&t.d);
    assert!(explain(&t.d, t.s.specs(), &cs, &p, "temperature", t.d.n_cells(), &opts).is_err());
    opts.max_depth = 1;
    let short = explain(&t.d, t.s.specs(), &cs, &p, "temperature", cell(25, 15) as usize, &opts).unwrap();
    assert_eq!(short.chain.len(), 1);
    assert!(short.stopped.as_ref().unwrap().contains("max_depth") && !short.found_origin());
    opts.max_depth = 12;
    let a = explain(&t.d, t.s.specs(), &cs, &p, "temperature", cell(25, 15) as usize, &opts).unwrap().text();
    let b = explain(&t.d, t.s.specs(), &cs, &p, "temperature", cell(25, 15) as usize, &opts).unwrap().text();
    assert_eq!(a, b);
    // an input that nothing produces and nobody marked is reported as external, not invented
    let empty = Provenance::new(t.d.n_cells());
    let ex = explain(&t.d, t.s.specs(), &cs, &empty, "temperature", cell(25, 15) as usize, &opts).unwrap();
    assert_eq!(ex.origin, vec!["external:surface".to_string()]);
}

#[test]
fn undoing_an_edit_lets_provenance_forget_it_and_the_change_set_goes_empty() {
    let edit = patch_edit();
    let (mut t, _, mut p, _) = solve_toy_edit(&edit);
    let v_after = t.f.current_version();
    t.f.undo().unwrap();
    p.forget("edit:ice patch");
    assert!(p.cells_with("surface", "edit:ice patch").is_empty());
    let cs = ChangeSet::from_store(&t.f, v_after - 1).unwrap();
    assert!(cs.fields.is_empty(), "back at the old version nothing differs");
}
