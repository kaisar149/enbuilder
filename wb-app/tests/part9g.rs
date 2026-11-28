//! Part 9g acceptance tests: stored versions, diff, propagation front, explainer. Headless, through commands and queries.
use wb_app::paint::Op;
use wb_app::{Command, Query, QueryResult, Session};

fn solved(res: f64) -> Session {
    let mut s = Session::with_preset("earthlike").unwrap();
    s.execute(Command::SetParam { name: "resolution".into(), value: res }).unwrap();
    assert_eq!(s.solve_blocking().unwrap(), "completed");
    s
}

fn brush(s: &Session, centre: usize, rings: usize) -> (Vec<u32>, Vec<f64>) {
    let mut depth = std::collections::BTreeMap::new();
    depth.insert(centre as u32, 0usize);
    let mut frontier = vec![centre as u32];
    for d in 1..=rings {
        let mut next = vec![];
        for &c in &frontier {
            for &nb in s.domain().neighbors(c as usize).iter() {
                let nb = nb as u32;
                if !depth.contains_key(&nb) {
                    depth.insert(nb, d);
                    next.push(nb);
                }
            }
        }
        frontier = next;
    }
    let cells: Vec<u32> = depth.keys().copied().collect();
    let w = cells.iter().map(|c| 1.0 - depth[c] as f64 / (rings as f64 + 1.0)).collect();
    (cells, w)
}

fn land_cell(s: &Session) -> usize {
    let z = s.field("elevation", 0).unwrap().values;
    let score = |i: usize| s.domain().neighbors(i).iter().filter(|&&j| z[j as usize] > 0.0).count();
    (0..z.len()).filter(|&i| z[i] > 0.0).max_by_key(|&i| (score(i), std::cmp::Reverse(i))).expect("a land cell")
}

fn raise(s: &mut Session, centre: usize, rings: usize, value: f64) {
    let (cells, w) = brush(s, centre, rings);
    s.execute(Command::PaintStroke { layer: "elevation".into(), op: Op::Add, cells, weights: w, value: Some(value), strength: 1.0 }).unwrap();
}

fn versions(s: &mut Session) -> Vec<u64> {
    let r = s.query(Query::GetVersions).unwrap();
    r.json["versions"].as_array().unwrap().iter().map(|v| v["version"].as_u64().unwrap()).collect()
}

struct Diff {
    delta: Vec<f32>,
    before: Vec<f32>,
    after: Vec<f32>,
    front: Vec<f32>,
    dist_km: Vec<f32>,
    json: serde_json::Value,
}

fn diff(s: &mut Session, field: &str, from: u64, to: u64) -> Diff {
    let r: QueryResult = s.query(Query::GetDiff { field: field.into(), from_version: from, to_version: to, min_rel: 0.01 }).unwrap();
    let n = r.json["n_cells"].as_u64().unwrap() as usize;
    assert_eq!(r.binary.len(), n * 5 * 4, "five f32 arrays");
    let arr = |k: usize| -> Vec<f32> { r.binary[k * n * 4..(k + 1) * n * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
    Diff { delta: arr(0), before: arr(1), after: arr(2), front: arr(3), dist_km: arr(4), json: r.json }
}

/// Solve, then paint a ridge and refine: returns (session, v0, v1, edit centre).
fn edited() -> (Session, u64, u64, usize) {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    raise(&mut s, c, 2, 3000.0);
    assert_eq!(s.refresh_blocking("refined").unwrap(), "completed");
    let v = versions(&mut s);
    assert!(v.len() >= 2, "solve and refresh are both stored: {v:?}");
    (s, v[v.len() - 2], v[v.len() - 1], c)
}

#[test]
fn diff_of_a_version_with_itself_is_zero() {
    let mut s = solved(2.0);
    let v = *versions(&mut s).last().unwrap();
    for f in ["temperature", "precipitation", "elevation", "biome", "humidity"] {
        if f == "humidity" && s.field(f, 0).is_err() {
            continue;
        }
        let d = diff(&mut s, f, v, v);
        assert!(d.delta.iter().all(|&x| x == 0.0), "{f}: no change");
        assert_eq!(d.json["no_change"], true, "{f}");
        assert!(d.front.iter().all(|&x| x == -1.0));
    }
    let sum = s.query(Query::GetDiffSummary { from_version: v, to_version: v, min_rel: 0.01 }).unwrap().json;
    assert_eq!(sum["no_change"], true);
    assert_eq!(sum["fields_changed"], 0);
}

#[test]
fn front_order_is_nondecreasing_and_ends_with_exactly_the_changed_cells() {
    let (mut s, a, b, c) = edited();
    for f in ["temperature", "precipitation", "elevation"] {
        let d = diff(&mut s, f, a, b);
        let changed: Vec<usize> = (0..d.delta.len()).filter(|&i| d.delta[i] != 0.0).collect();
        assert_eq!(d.json["summary"]["changed_cells"].as_u64().unwrap() as usize, changed.len(), "{f}");
        // changed cells carry a reveal time in [0, 1], unchanged cells -1
        for i in 0..d.delta.len() {
            if d.delta[i] != 0.0 {
                assert!((0.0..=1.0).contains(&d.front[i]), "{f} cell {i} front {}", d.front[i]);
            } else {
                assert_eq!(d.front[i], -1.0, "{f} cell {i}");
            }
        }
        // the animation (reveal cells in front order) is nondecreasing in surface distance for the edit-seeded order and ends with the changed set
        let mut order = changed.clone();
        order.sort_by(|&x, &y| d.front[x].partial_cmp(&d.front[y]).unwrap().then(x.cmp(&y)));
        let dists: Vec<f32> = order.iter().map(|&i| d.dist_km[i]).collect();
        assert!(dists.windows(2).all(|w| w[0] <= w[1] + 1e-3), "{f}: distance never decreases along the front");
        let shown: Vec<usize> = (0..d.delta.len()).filter(|&i| d.front[i] >= 0.0 && d.front[i] <= 1.0).collect();
        assert_eq!(shown, changed, "{f}: at t = 1 exactly the changed cells are shown");
    }
    assert_eq!(diff(&mut s, "elevation", a, b).json["front_kind"], "edit");
    // the edit centre itself is the first thing that changed in elevation
    let d = diff(&mut s, "elevation", a, b);
    assert!(d.front[c] <= 0.05 && d.dist_km[c] == 0.0, "edit centre first: front {} dist {}", d.front[c], d.dist_km[c]);
}

#[test]
fn before_plus_delta_is_after_and_matches_the_visible_fields() {
    let (mut s, a, b, _) = edited();
    let d = diff(&mut s, "temperature", a, b);
    let now = s.field("temperature", 0).unwrap().values;
    for i in 0..now.len() {
        assert!((d.after[i] as f64 - now[i]).abs() < 1e-3, "after = the field the clients see (cell {i})");
        assert!((d.before[i] + d.delta[i] - d.after[i]).abs() < 1e-3);
    }
}

#[test]
fn lock_protected_cells_never_appear_as_changed() {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    // lock a neighbourhood of temperature, elevation and precipitation, then edit right next to it
    let (cells, _) = brush(&s, c, 1);
    for f in ["temperature", "elevation", "precipitation", "biome"] {
        s.execute(Command::SetLock { field: f.into(), cells: cells.clone(), locked: true }).unwrap();
    }
    let nb = s.domain().neighbors(c).iter().map(|&x| x as usize).find(|x| !cells.contains(&(*x as u32))).or_else(|| (0..s.domain().n_cells()).find(|i| !cells.contains(&(*i as u32)))).unwrap();
    let ring2: Vec<usize> = s.domain().neighbors(nb).iter().map(|&x| x as usize).collect();
    raise(&mut s, nb, 2, 2500.0);
    let _ = ring2;
    assert_eq!(s.refresh_blocking("refined").unwrap(), "completed");
    let v = versions(&mut s);
    let (a, b) = (v[v.len() - 2], v[v.len() - 1]);
    for f in ["temperature", "elevation", "precipitation", "biome"] {
        let d = diff(&mut s, f, a, b);
        for &c in &cells {
            assert_eq!(d.delta[c as usize], 0.0, "{f}: locked cell {c} did not change");
            assert_eq!(d.front[c as usize], -1.0, "{f}: locked cell {c} is never revealed");
        }
    }
}

#[test]
fn explainer_returns_the_core_chain_as_data() {
    let (mut s, a, b, c) = edited();
    // a changed cell of precipitation, as far from the edit as possible
    let d = diff(&mut s, "precipitation", a, b);
    let far = (0..d.delta.len()).filter(|&i| d.delta[i] != 0.0).max_by(|&x, &y| d.dist_km[x].partial_cmp(&d.dist_km[y]).unwrap()).expect("precipitation changed somewhere");
    let r = s.query(Query::Explain { cell: far, field: "precipitation".into(), from_version: Some(a), to_version: Some(b), min_rel: 0.01 }).unwrap().json;
    let steps = r["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    assert_eq!(steps[0]["field"], "precipitation");
    assert_eq!(steps[0]["cell"].as_u64().unwrap() as usize, far);
    // the first link is the measured change of the diff
    assert!((steps[0]["old"].as_f64().unwrap() as f32 - d.before[far]).abs() <= 1e-6 * d.before[far].abs().max(1e-12) + 1e-9);
    assert!((steps[0]["new"].as_f64().unwrap() as f32 - d.after[far]).abs() <= 1e-6 * d.after[far].abs().max(1e-12) + 1e-9);
    // every link names its module, hop and cause; the text is the core's, one line per link
    let text = r["text"].as_str().unwrap();
    assert_eq!(text.lines().filter(|l| l.contains(" @cell ")).count(), steps.len());
    for st in steps {
        assert!(st["hop"].as_str().is_some() && st["source_cells"].is_array());
        assert!(text.contains(&format!("{} @cell {}", st["field"].as_str().unwrap(), st["cell"])));
        if st["hop"] != "origin" && st["hop"] != "end" {
            assert!(st["module"].is_string());
        }
    }
    // with no versions given: latest against the one before, i.e. the same answer
    let r2 = s.query(Query::Explain { cell: far, field: "precipitation".into(), from_version: None, to_version: None, min_rel: 0.01 }).unwrap().json;
    assert_eq!(r["text"], r2["text"]);
    // an unchanged cell explains as "did not change"
    let quiet = (0..d.delta.len()).find(|&i| d.delta[i] == 0.0).unwrap();
    let q = s.query(Query::Explain { cell: quiet, field: "precipitation".into(), from_version: Some(a), to_version: Some(b), min_rel: 0.01 }).unwrap().json;
    assert_eq!(q["changed"], false);
    assert_eq!(q["steps"].as_array().unwrap().len(), 0);
    let _ = c;
}

#[test]
fn versions_errors_and_summary() {
    let (mut s, a, b, _) = edited();
    assert!(versions(&mut s).contains(&a) && versions(&mut s).contains(&b));
    let e = s.query(Query::GetDiff { field: "temperature".into(), from_version: 999_999, to_version: b, min_rel: 0.01 }).err().unwrap();
    assert_eq!(e.code, "unknown_version");
    let e = s.query(Query::GetDiff { field: "no_such_field".into(), from_version: a, to_version: b, min_rel: 0.01 }).err().unwrap();
    assert_eq!(e.code, "unknown_field");
    let sum = s.query(Query::GetDiffSummary { from_version: a, to_version: b, min_rel: 0.01 }).unwrap().json;
    assert_eq!(sum["no_change"], false);
    assert!(sum["edit_cells"].as_u64().unwrap() > 0);
    let rows = sum["fields"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["field"] == "elevation"));
    let z = rows.iter().find(|r| r["field"] == "elevation").unwrap();
    assert!(z["peak_abs"].as_f64().unwrap() > 100.0);
    assert!(sum["reach_km"].as_f64().unwrap() > 0.0, "the effect reached beyond the edit");
}

#[test]
fn undo_of_a_stroke_stores_the_restored_version() {
    let (mut s, _a, b, _) = edited();
    s.execute(Command::Undo).unwrap();
    let v = versions(&mut s);
    assert!(v.len() >= 3, "the restored state is a stored version too: {v:?}");
    let restored = *v.last().unwrap();
    assert_ne!(restored, b);
    // back at the original climate: a diff against the first solve is zero
    let first = v[0];
    let d = diff(&mut s, "temperature", first, restored);
    assert!(d.delta.iter().all(|&x| x == 0.0), "undo restores the exact solved state");
}

/// Reports (printed with --nocapture) which fields a basin dug into the driest land cell changed, and the chain behind a cell downwind.
#[test]
fn basin_in_a_dry_region_demo() {
    let mut s = solved(3.0);
    let p = s.field("precipitation", 0).unwrap().values;
    let z = s.field("elevation", 0).unwrap().values;
    let dry = (0..p.len()).filter(|&i| z[i] > 100.0).min_by(|&a, &b| p[a].partial_cmp(&p[b]).unwrap()).unwrap();
    let (cells, w) = brush(&s, dry, 2);
    s.execute(Command::PaintStroke { layer: "elevation".into(), op: Op::Set, cells, weights: w, value: Some(-300.0), strength: 1.0 }).unwrap();
    assert_eq!(s.refresh_blocking("refined").unwrap(), "completed");
    let v = versions(&mut s);
    let (a, b) = (v[v.len() - 2], v[v.len() - 1]);
    let sum = s.query(Query::GetDiffSummary { from_version: a, to_version: b, min_rel: 0.01 }).unwrap().json;
    println!("fields changed: {}, reach {:.0} km", sum["fields_changed"], sum["reach_km"].as_f64().unwrap());
    for r in sum["fields"].as_array().unwrap().iter().take(12) {
        println!("  {:28} cells {:5} peak {:.4e} reach {:.0} km", r["field"].as_str().unwrap(), r["changed_cells"], r["peak_abs"].as_f64().unwrap(), r["reach_km"].as_f64().unwrap());
    }
    let d = diff(&mut s, "precipitation", a, b);
    let far = (0..d.delta.len()).filter(|&i| d.delta[i] != 0.0).max_by(|&x, &y| d.dist_km[x].partial_cmp(&d.dist_km[y]).unwrap());
    if let Some(far) = far {
        let r = s.query(Query::Explain { cell: far, field: "precipitation".into(), from_version: Some(a), to_version: Some(b), min_rel: 0.01 }).unwrap().json;
        println!("chain for cell {far} ({} km away):\n{}", d.dist_km[far], r["text"].as_str().unwrap());
    }
}

#[test]
fn rerunning_a_solve_without_an_edit_shows_almost_nothing() {
    let mut s = solved(3.0);
    assert_eq!(s.refresh_blocking("refined").unwrap(), "completed");
    let v = versions(&mut s);
    let (a, b) = (v[v.len() - 2], v[v.len() - 1]);
    let sum = s.query(Query::GetDiffSummary { from_version: a, to_version: b, min_rel: 0.01 }).unwrap().json;
    let names: Vec<&str> = sum["fields"].as_array().unwrap().iter().map(|r| r["field"].as_str().unwrap()).collect();
    for quiet in ["wind", "wind_speed", "pressure_sl", "current", "current_speed"] {
        assert!(!names.contains(&quiet), "{quiet} only drifts by round-off and must be hidden: {names:?}");
    }
    assert_eq!(sum["edit_cells"], 0);
    // exact mode shows everything that differs
    let exact = s.query(Query::GetDiffSummary { from_version: a, to_version: b, min_rel: 0.0 }).unwrap().json;
    assert!(exact["fields_changed"].as_u64().unwrap() > sum["fields_changed"].as_u64().unwrap());
    // no edit cells: the front is measured from the cell that changed most
    assert_eq!(diff(&mut s, "temperature", a, b).json["front_kind"], "peak");
}
