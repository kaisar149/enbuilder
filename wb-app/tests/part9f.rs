//! Part 9f acceptance tests: paint strokes, locks, realism blend, preview / refined refresh and unified undo. Headless, through commands and queries.
use serde_json::json;
use wb_app::paint::Op;
use wb_app::{Command, Event, Query, Session};

fn world(res: f64) -> Session {
    let mut s = Session::with_preset("earthlike").unwrap();
    s.execute(Command::SetParam { name: "resolution".into(), value: res }).unwrap();
    s
}

fn solved(res: f64) -> Session {
    let mut s = world(res);
    assert_eq!(s.solve_blocking().unwrap(), "completed");
    s
}

/// Cells within `rings` neighbour steps of `centre`, with a linear falloff weight (1 at the centre).
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
    // the land cell with the most land neighbours (the interior of the biggest-looking continent), lowest index on ties
    let z = s.field("elevation", 0).unwrap().values;
    let score = |i: usize| s.domain().neighbors(i).iter().filter(|&&j| z[j as usize] > 0.0).count();
    (0..z.len()).filter(|&i| z[i] > 0.0).max_by_key(|&i| (score(i), std::cmp::Reverse(i))).expect("a land cell")
}

fn stroke(s: &mut Session, layer: &str, op: Op, cells: &[u32], w: &[f64], value: Option<f64>) -> wb_app::Ack {
    s.execute(Command::PaintStroke { layer: layer.into(), op, cells: cells.to_vec(), weights: w.to_vec(), value, strength: 1.0 }).unwrap()
}

fn hashes(s: &Session) -> Vec<Option<u64>> { ["elevation", "temperature", "precipitation", "biome"].iter().map(|f| s.content_hash(f)).collect() }

fn field(s: &Session, n: &str) -> Vec<f64> { s.field(n, 0).unwrap().values }

#[test]
fn stroke_then_undo_restores_the_world_exactly() {
    let mut s = solved(2.0);
    let before = hashes(&s);
    let (before_t, before_p) = (field(&s, "temperature"), field(&s, "precipitation"));
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 2);
    let z0 = field(&s, "elevation");
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(3000.0));
    assert_ne!(hashes(&s)[0], before[0], "the stroke changed the elevation view");
    assert!(s.is_stale(), "a terrain edit makes the climate stale until a refresh");
    // refresh (so there is a newer solved state to throw away on undo)
    assert_eq!(s.refresh_blocking("preview").unwrap(), "completed");
    assert!(!s.is_stale());
    let after = hashes(&s);
    s.execute(Command::Undo).unwrap();
    assert_eq!(hashes(&s), before, "undo restores elevation, temperature, precipitation and biome exactly (content hashes)");
    assert_eq!(field(&s, "elevation"), z0);
    assert_eq!(field(&s, "temperature"), before_t);
    assert_eq!(field(&s, "precipitation"), before_p);
    assert!(!s.is_stale());
    assert_eq!(s.quality(), Some("full"));
    s.execute(Command::Redo).unwrap();
    assert_eq!(hashes(&s), after, "redo brings the edited world back");
    assert_eq!(s.quality(), Some("preview"));
    // versions are monotonic even though contents are equal again
    let v1 = s.field("temperature", 0).unwrap().version;
    s.execute(Command::Undo).unwrap();
    assert!(s.field("temperature", 0).unwrap().version > v1);
}

#[test]
fn locked_cells_never_change_after_a_solve() {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    let (lock_cells, _) = brush(&s, c, 1);
    for f in ["elevation", "temperature", "precipitation", "biome"] {
        let a = s.execute(Command::SetLock { field: f.into(), cells: lock_cells.clone(), locked: true }).unwrap();
        assert_eq!(a.detail.unwrap()["changed"], lock_cells.len());
    }
    let snap: Vec<Vec<f64>> = ["elevation", "temperature", "precipitation", "biome"].iter().map(|f| field(&s, f)).collect();
    // paint a big mountain around the valley, including the locked cells
    let (cells, w) = brush(&s, c, 3);
    let ack = stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(2500.0));
    let d = ack.detail.unwrap();
    assert_eq!(d["skipped_locked"], lock_cells.len(), "painting a locked cell is skipped and reported");
    assert!(d["applied"].as_u64().unwrap() > 0);
    for mode in ["refined", "full"] {
        assert_eq!(s.refresh_blocking(mode).unwrap(), "completed");
        for (k, f) in ["elevation", "temperature", "precipitation", "biome"].iter().enumerate() {
            let now = field(&s, f);
            for &lc in &lock_cells {
                assert_eq!(now[lc as usize], snap[k][lc as usize], "{f} changed at locked cell {lc} after {mode}");
            }
        }
        // the solver itself kept elevation and temperature pinned (not only the published view)
        for f in ["elevation", "temperature"] {
            let raw = s.query(Query::GetFieldView { name: f.into(), layer: 0, view: "computed".into() }).unwrap();
            let raw: Vec<f32> = raw.binary.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let k = if f == "elevation" { 0 } else { 1 };
            for &lc in &lock_cells {
                assert_eq!(raw[lc as usize], snap[k][lc as usize] as f32, "solver changed pinned {f} at {lc}");
            }
        }
    }
    // unlock then repaint: the valley can move again
    s.execute(Command::SetLock { field: "elevation".into(), cells: lock_cells.clone(), locked: false }).unwrap();
    let ack = stroke(&mut s, "elevation", Op::Add, &lock_cells, &vec![1.0; lock_cells.len()], Some(500.0));
    assert_eq!(ack.detail.unwrap()["skipped_locked"], 0);
}

#[test]
fn realism_blend_is_linear() {
    let mut s = world(2.0);
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 2);
    let computed = field(&s, "elevation");
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(2000.0));
    let painted = field(&s, "elevation"); // default realism 0: painted as given
    for (k, &cc) in cells.iter().enumerate() {
        let want = (computed[cc as usize] + 2000.0 * w[k]).clamp(-12000.0, 12000.0);
        assert!((painted[cc as usize] - want).abs() < 1e-9, "r=0 shows the painted value as given");
    }
    let set = |s: &mut Session, r: f64| s.execute(Command::SetRealism { field: "elevation".into(), value: r }).unwrap();
    set(&mut s, 100.0);
    assert_eq!(field(&s, "elevation"), computed, "r=100 ignores the paint");
    for r in [25.0, 50.0, 80.0] {
        set(&mut s, r);
        let z = field(&s, "elevation");
        for &cc in &cells {
            let i = cc as usize;
            let want = computed[i] + (1.0 - r / 100.0) * (painted[i] - computed[i]);
            assert!((z[i] - want).abs() < 1e-9, "r={r}: linear blend");
        }
    }
    // both views stay available whatever the slider says
    set(&mut s, 50.0);
    let view = |s: &mut Session, v: &str| -> Vec<f32> {
        let q = s.query(Query::GetFieldView { name: "elevation".into(), layer: 0, view: v.into() }).unwrap();
        q.binary.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
    };
    let (pv, cv) = (view(&mut s, "painted"), view(&mut s, "computed"));
    for &cc in &cells {
        assert_eq!(pv[cc as usize], painted[cc as usize] as f32);
        assert_eq!(cv[cc as usize], computed[cc as usize] as f32);
    }
    // realism is one undo step
    s.execute(Command::Undo).unwrap();
    assert_eq!(s.query(Query::GetWorldInfo).unwrap().json["edit"]["realism"]["elevation"], json!(80.0), "undo reverts the last slider step (50 -> back to 80)");
}

#[test]
fn bias_and_biome_layers_blend_on_the_solved_fields() {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    let (cells, _) = brush(&s, c, 1);
    let ones = vec![1.0; cells.len()];
    let (t0, p0, b0) = (field(&s, "temperature"), field(&s, "precipitation"), field(&s, "biome"));
    let stale_before = s.is_stale();
    stroke(&mut s, "temperature_bias", Op::Add, &cells, &ones, Some(5.0));
    stroke(&mut s, "rainfall_bias", Op::Add, &cells, &ones, Some(0.5));
    stroke(&mut s, "biome_override", Op::Set, &cells, &ones, Some(3.0));
    assert_eq!(s.is_stale(), stale_before, "bias and biome paint do not need a re-solve");
    let (t1, p1, b1) = (field(&s, "temperature"), field(&s, "precipitation"), field(&s, "biome"));
    for &cc in &cells {
        let i = cc as usize;
        assert!((t1[i] - t0[i] - 5.0).abs() < 1e-9);
        assert!((p1[i] - p0[i] * 1.5).abs() < 1e-15);
        assert_eq!(b1[i], 3.0);
    }
    s.execute(Command::SetRealism { field: "all".into(), value: 100.0 }).unwrap();
    assert_eq!((field(&s, "temperature"), field(&s, "precipitation"), field(&s, "biome")), (t0.clone(), p0.clone(), b0.clone()));
    s.execute(Command::SetRealism { field: "all".into(), value: 50.0 }).unwrap();
    let (t2, b2) = (field(&s, "temperature"), field(&s, "biome"));
    assert!((t2[cells[0] as usize] - t0[cells[0] as usize] - 2.5).abs() < 1e-9, "midpoint is half the bias");
    assert_eq!(b2[cells[0] as usize], 3.0, "(1-r)*mask = 0.5 still wins");
    s.execute(Command::SetRealism { field: "biome".into(), value: 51.0 }).unwrap();
    assert_eq!(field(&s, "biome")[cells[0] as usize], b0[cells[0] as usize]);
    // the eraser returns cells to the physics
    s.execute(Command::SetRealism { field: "all".into(), value: 0.0 }).unwrap();
    stroke(&mut s, "temperature_bias", Op::Erase, &cells, &ones, None);
    assert_eq!(field(&s, "temperature"), t0);
}

#[test]
fn preview_and_refined_agree_on_a_test_edit() {
    let mut s = solved(3.0);
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 3);
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(3000.0));
    assert_eq!(s.refresh_blocking("preview").unwrap(), "completed");
    let info = s.query(Query::GetWorldInfo).unwrap().json;
    let lr = info["edit"]["last_refresh"].clone();
    eprintln!("preview: {lr}");
    assert_eq!(info["edit"]["quality"], "preview");
    let (tp, pp) = (field(&s, "temperature"), field(&s, "precipitation"));
    if !cfg!(debug_assertions) {
        assert!(lr["ms"].as_u64().unwrap() < 100, "preview must stay under 100 ms, took {lr}");
    }
    assert_eq!(s.refresh_blocking("refined").unwrap(), "completed");
    let lr = s.query(Query::GetWorldInfo).unwrap().json["edit"]["last_refresh"].clone();
    eprintln!("refined: {lr}");
    let (tr, pr) = (field(&s, "temperature"), field(&s, "precipitation"));
    let n = tp.len() as f64;
    let mae_t = tp.iter().zip(&tr).map(|(a, b)| (a - b).abs()).sum::<f64>() / n;
    let mean_p = pr.iter().sum::<f64>() / n;
    let mae_p = pp.iter().zip(&pr).map(|(a, b)| (a - b).abs()).sum::<f64>() / n;
    let max_t = tp.iter().zip(&tr).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    eprintln!("preview vs refined: temperature MAE {mae_t:.3} K (max {max_t:.2}), precipitation MAE {:.1} % of the mean", 100.0 * mae_p / mean_p);
    assert!(mae_t < 1.5, "mean absolute temperature difference {mae_t} K");
    assert!(mae_p / mean_p < 0.25, "mean absolute precipitation difference {} of the mean", mae_p / mean_p);
    // refined is not far from a full solve either
    assert_eq!(s.solve_blocking().unwrap(), "completed");
    let tf = field(&s, "temperature");
    let mae_f = tr.iter().zip(&tf).map(|(a, b)| (a - b).abs()).sum::<f64>() / n;
    eprintln!("refined vs full: temperature MAE {mae_f:.3} K");
    assert!(mae_f < 2.0);
}

#[test]
fn a_mountain_makes_a_rain_shadow_after_refresh() {
    let mut s = solved(3.0);
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 2);
    let p0 = field(&s, "precipitation");
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(4000.0));
    s.refresh_blocking("refined").unwrap();
    let p1 = field(&s, "precipitation");
    let moved = p0.iter().zip(&p1).filter(|(a, b)| (*a - *b).abs() > 1e-9).count();
    assert!(moved > cells.len(), "the climate responded beyond the painted cells ({moved} cells changed)");
}

#[test]
fn strokes_are_batched_and_history_is_unified() {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 3);
    assert!(cells.len() > 20);
    let a = stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(300.0));
    assert_eq!(a.command, "paint_stroke");
    s.execute(Command::SetLock { field: "elevation".into(), cells: vec![cells[0]], locked: true }).unwrap();
    s.execute(Command::SetRealism { field: "elevation".into(), value: 30.0 }).unwrap();
    s.execute(Command::SetParam { name: "axial_tilt_deg".into(), value: 10.0 }).unwrap();
    let h = s.query(Query::GetWorldInfo).unwrap().json["history"].clone();
    let kinds: Vec<&str> = h["undo"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["params", "stroke", "lock", "realism", "params"], "one entry per resolution change / stroke / lock / realism / param change, oldest first");
    for e in h["undo"].as_array().unwrap() {
        assert!(e["label"].is_string() && e["changes"].is_array());
    }
    for _ in 0..5 {
        s.execute(Command::Undo).unwrap();
    }
    assert_eq!(s.execute(Command::Undo).unwrap_err().code, "nothing_to_undo");
    let info = s.query(Query::GetWorldInfo).unwrap().json;
    assert_eq!(info["edit"]["paint"]["elevation"], 0);
    assert_eq!(info["edit"]["locks"]["elevation"], 0);
    for _ in 0..5 {
        s.execute(Command::Redo).unwrap();
    }
    let info = s.query(Query::GetWorldInfo).unwrap().json;
    assert_eq!(info["edit"]["paint"]["elevation"].as_u64().unwrap() as usize, cells.len());
    assert_eq!(info["edit"]["locks"]["elevation"], 1);
    assert_eq!(info["edit"]["realism"]["elevation"], json!(30.0));
}

#[test]
fn smooth_flatten_and_validation() {
    let mut s = world(2.0);
    let c = land_cell(&s);
    let (cells, _) = brush(&s, c, 2);
    let ones = vec![1.0; cells.len()];
    stroke(&mut s, "elevation", Op::Flatten, &cells, &ones, Some(1234.0));
    let z = field(&s, "elevation");
    assert!(cells.iter().all(|&i| (z[i as usize] - 1234.0).abs() < 1e-9));
    let (cells2, _) = brush(&s, c, 1);
    stroke(&mut s, "elevation", Op::Add, &cells2[..1], &[1.0], Some(800.0));
    let spike = field(&s, "elevation")[cells2[0] as usize];
    stroke(&mut s, "elevation", Op::Smooth, &cells2, &vec![1.0; cells2.len()], None);
    assert!(field(&s, "elevation")[cells2[0] as usize] < spike, "smoothing lowers a spike");
    let err = |s: &mut Session, c: Command| s.execute(c).unwrap_err().code;
    let bad = |layer: &str, op: Op, cells: Vec<u32>, weights: Vec<f64>, value: Option<f64>| Command::PaintStroke { layer: layer.into(), op, cells, weights, value, strength: 1.0 };
    assert_eq!(err(&mut s, bad("nope", Op::Add, vec![0], vec![1.0], Some(1.0))), "bad_layer");
    assert_eq!(err(&mut s, bad("elevation", Op::Add, vec![0, 1], vec![1.0], Some(1.0))), "bad_stroke");
    assert_eq!(err(&mut s, bad("elevation", Op::Add, vec![99999], vec![1.0], Some(1.0))), "bad_cell");
    assert_eq!(err(&mut s, bad("biome_override", Op::Smooth, vec![0], vec![1.0], None)), "bad_op");
    assert_eq!(err(&mut s, bad("elevation", Op::Add, vec![0], vec![1.0], None)), "bad_stroke");
    assert_eq!(err(&mut s, Command::SetRealism { field: "elevation".into(), value: 101.0 }), "bad_param");
    assert_eq!(err(&mut s, Command::SetLock { field: "temperature".into(), cells: vec![0], locked: true }), "not_solved");
    assert_eq!(err(&mut s, Command::Refresh { mode: "preview".into() }), "not_solved");
    assert_eq!(err(&mut s, Command::Refresh { mode: "fast".into() }), "bad_param");
}

#[test]
fn refresh_jobs_report_their_mode_and_are_superseded() {
    let mut s = solved(2.0);
    s.set_step_delay(std::time::Duration::from_millis(30));
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 2);
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(1000.0));
    s.execute(Command::Refresh { mode: "refined".into() }).unwrap();
    let info = s.query(Query::GetWorldInfo).unwrap().json;
    assert_eq!(info["job"]["mode"], "refined");
    // a newer request replaces the running refresh
    s.execute(Command::Refresh { mode: "preview".into() }).unwrap();
    assert_eq!(s.query(Query::GetWorldInfo).unwrap().json["job"]["mode"], "preview");
    // a stroke cancels it
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(10.0));
    assert!(!s.job_running());
    let ev = s.poll_events();
    assert!(ev.iter().any(|e| matches!(e, Event::JobDone { status, .. } if status == "cancelled")));
    s.set_step_delay(std::time::Duration::ZERO);
    // but a refresh cannot replace a full solve
    s.execute(Command::Solve).unwrap();
    assert_eq!(s.execute(Command::Refresh { mode: "preview".into() }).unwrap_err().code, "job_running");
    s.execute(Command::Cancel).unwrap();
    let _ = s.wait_job();
}

#[test]
fn paint_and_lock_queries_and_wire_format() {
    let mut s = solved(2.0);
    let c = land_cell(&s);
    let n = s.domain().n_cells();
    let (cells, w) = brush(&s, c, 1);
    stroke(&mut s, "temperature_bias", Op::Add, &cells, &w, Some(2.0));
    s.execute(Command::SetLock { field: "biome".into(), cells: cells.clone(), locked: true }).unwrap();
    let p = s.query(Query::GetPaint { layer: "temperature_bias".into() }).unwrap();
    assert_eq!(p.binary.len(), n * 8);
    assert_eq!(p.json["painted_cells"].as_u64().unwrap() as usize, cells.len());
    let l = s.query(Query::GetLocks).unwrap();
    let biome = l.json["layout"].as_array().unwrap().iter().find(|e| e["name"] == "biome").unwrap().clone();
    assert_eq!(biome["count"].as_u64().unwrap() as usize, cells.len());
    assert_eq!(s.query(Query::GetPaint { layer: "x".into() }).unwrap_err().code, "bad_layer");
    // JSON shapes
    let cmd: Command = serde_json::from_str(r#"{"type":"paint_stroke","layer":"elevation","op":"add","cells":[1,2],"weights":[1,0.5],"value":300}"#).unwrap();
    assert!(matches!(cmd, Command::PaintStroke { strength, .. } if strength == 1.0));
    let cmd: Command = serde_json::from_str(r#"{"type":"set_lock","field":"biome","cells":[1],"locked":true}"#).unwrap();
    assert_eq!(cmd.name(), "set_lock");
    let cmd: Command = serde_json::from_str(r#"{"type":"refresh","mode":"preview"}"#).unwrap();
    assert_eq!(cmd.name(), "refresh");
    let q: Query = serde_json::from_str(r#"{"type":"get_field_view","name":"elevation"}"#).unwrap();
    assert!(matches!(q, Query::GetFieldView { view, .. } if view == "blended"));
    // cell inspector says what is painted / locked
    let cell = s.query(Query::GetCell { cell: c }).unwrap().json;
    assert_eq!(cell["edit_state"]["biome"]["locked"], true);
    assert!(cell["edit_state"]["temperature"]["painted"].as_f64().unwrap() > 0.0);
}

#[test]
fn paint_survives_a_mesh_change_round_trip() {
    let mut s = world(2.0);
    let c = land_cell(&s);
    let (cells, w) = brush(&s, c, 1);
    stroke(&mut s, "elevation", Op::Add, &cells, &w, Some(900.0));
    let h = s.content_hash("elevation");
    s.execute(Command::SetParam { name: "resolution".into(), value: 3.0 }).unwrap();
    assert_eq!(s.query(Query::GetWorldInfo).unwrap().json["edit"]["paint"]["elevation"], 0, "a new mesh starts clean");
    s.execute(Command::Undo).unwrap();
    assert_eq!(s.content_hash("elevation"), h, "undoing the mesh change brings the paint of the old mesh back");
}
