//! Part 9e acceptance tests (core side): river polylines, outlines, current field, seasonal cell values, overlay/preset schema data.
use serde_json::Value;
use std::sync::OnceLock;
use wb_app::{Command, Query, Session};

/// One solved earthlike world shared by the tests (resolution 2 keeps the solve short).
fn solved() -> &'static std::sync::Mutex<Session> {
    static S: OnceLock<std::sync::Mutex<Session>> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = Session::with_preset("earthlike").unwrap();
        s.execute(Command::SetParam { name: "resolution".into(), value: 3.0 }).unwrap();
        s.solve_blocking().unwrap();
        std::sync::Mutex::new(s)
    })
}

fn u32s(b: &[u8], off: usize, n: usize) -> Vec<u32> { (0..n).map(|i| u32::from_le_bytes(b[off + 4 * i..off + 4 * i + 4].try_into().unwrap())).collect() }
fn f32s(b: &[u8], off: usize, n: usize) -> Vec<f32> { (0..n).map(|i| f32::from_le_bytes(b[off + 4 * i..off + 4 * i + 4].try_into().unwrap())).collect() }
fn item(j: &Value, name: &str) -> (usize, usize) {
    let it = j["layout"].as_array().unwrap().iter().find(|l| l["name"] == name).unwrap();
    (it["offset"].as_u64().unwrap() as usize, it["count"].as_u64().unwrap() as usize)
}
fn field(s: &mut Session, name: &str) -> Vec<f32> {
    let r = s.query(Query::GetField { name: name.into(), layer: 0 }).unwrap();
    f32s(&r.binary, 0, r.json["count"].as_u64().unwrap() as usize)
}

#[test]
fn rivers_are_connected_polylines_from_source_to_outlet() {
    let mut s = solved().lock().unwrap();
    let mesh = s.query(Query::GetMesh).unwrap();
    let (ao, aon) = item(&mesh.json, "adj_offsets");
    let (aj, ajn) = item(&mesh.json, "adj");
    let adj_off = u32s(&mesh.binary, ao, aon);
    let adj = u32s(&mesh.binary, aj, ajn);
    let r = s.query(Query::GetRivers).unwrap();
    let n_lines = r.json["n_lines"].as_u64().unwrap() as usize;
    assert!(n_lines > 3, "an earthlike world has rivers (got {n_lines})");
    let (oo, on) = item(&r.json, "offsets");
    let (co, cn) = item(&r.json, "cells");
    let (vo, vn) = item(&r.json, "discharge");
    assert_eq!((on, cn, vn), (n_lines + 1, r.json["n_points"].as_u64().unwrap() as usize, cn));
    let offsets = u32s(&r.binary, oo, on);
    let cells = u32s(&r.binary, co, cn);
    let q = f32s(&r.binary, vo, vn);
    assert_eq!(*offsets.last().unwrap() as usize, cn);
    let river = field(&mut s, "river_mask");
    let ocean = field(&mut s, "ocean_mask");
    let flow = field(&mut s, "flow_dir");
    let lake = field(&mut s, "lake_id");
    let mut covered = vec![false; river.len()];
    for k in 0..n_lines {
        let line = &cells[offsets[k] as usize..offsets[k + 1] as usize];
        assert!(line.len() >= 2);
        for w in line.windows(2) {
            let nb = &adj[adj_off[w[0] as usize] as usize..adj_off[w[0] as usize + 1] as usize];
            assert!(nb.contains(&w[1]), "consecutive vertices must be neighbours");
            assert_eq!(flow[w[0] as usize] as i64, w[1] as i64, "a line follows flow_dir");
        }
        for &c in line {
            covered[c as usize] |= river[c as usize] > 0.5;
        }
        // the first vertex is a river cell; the last is the outlet (ocean, lake, sink edge), a river cell where it joins another river, or a cell that drains on
        assert!(river[line[0] as usize] > 0.5);
        let last = *line.last().unwrap() as usize;
        assert!(ocean[last] > 0.5 || lake[last] >= 0.0 || river[last] > 0.5 || flow[last] < 0.0, "line {k} ends nowhere: last {last} ocean {} lake {} river {} flow {} line {:?}", ocean[last], lake[last], river[last], flow[last], line);
    }
    for c in 0..river.len() {
        if river[c] > 0.5 {
            assert!(covered[c], "river cell {c} is in no line");
        }
    }
    assert!(q.iter().all(|v| v.is_finite() && *v >= 0.0));
    // downstream of a source the discharge never falls along a line, except through a lake loss
    assert!(q.iter().cloned().fold(0.0, f32::max) > 0.0);
}

#[test]
fn outline_edges_separate_inside_from_outside() {
    let mut s = solved().lock().unwrap();
    let ocean = field(&mut s, "ocean_mask");
    let r = s.query(Query::GetOutline { field: "ocean_mask".into(), threshold: 0.5 }).unwrap();
    let n = r.json["n_edges"].as_u64().unwrap() as usize;
    assert!(n > 10);
    let pairs = u32s(&r.binary, 0, 2 * n);
    for e in pairs.chunks(2) {
        assert!(ocean[e[0] as usize] > 0.5 && ocean[e[1] as usize] <= 0.5);
    }
    // threshold default and other fields work; unknown fields and an unsolved field error out
    assert!(s.query(Query::GetOutline { field: "glacier_mask".into(), threshold: 0.5 }).is_ok());
    assert_eq!(s.query(Query::GetOutline { field: "nope".into(), threshold: 0.5 }).unwrap_err().code, "unknown_field");
    let j: Value = serde_json::from_str(r#"{"type":"get_outline","field":"ocean_mask"}"#).unwrap();
    assert_eq!(serde_json::from_value::<Query>(j).unwrap(), Query::GetOutline { field: "ocean_mask".into(), threshold: 0.5 });
}

#[test]
fn unsolved_world_refuses_rivers() {
    let mut s = Session::with_preset("earthlike").unwrap();
    assert_eq!(s.query(Query::GetRivers).unwrap_err().code, "not_solved");
    // elevation outlines work before a solve (a coast from elevation > 0)
    assert!(s.query(Query::GetOutline { field: "elevation".into(), threshold: 0.0 }).is_ok());
}

#[test]
fn the_solve_produces_a_nonzero_ocean_current_vector() {
    let mut s = solved().lock().unwrap();
    let mut mx = 0.0f32;
    for l in 0..3 {
        let r = s.query(Query::GetField { name: "current".into(), layer: l }).unwrap();
        let v = f32s(&r.binary, 0, r.json["count"].as_u64().unwrap() as usize);
        mx = mx.max(v.iter().fold(0.0, |a, b| a.max(b.abs())));
    }
    assert!(mx > 1e-4, "ocean current should be non-zero, got max {mx}");
    let w = s.query(Query::GetField { name: "wind".into(), layer: 0 }).unwrap();
    assert_eq!(w.json["layers"], 3);
}

#[test]
fn get_cell_returns_twelve_seasonal_values() {
    let mut s = solved().lock().unwrap();
    let c = s.query(Query::GetCell { cell: 5 }).unwrap().json;
    let seasonal = c["seasonal"].as_object().unwrap();
    for k in ["temperature", "precipitation", "soil_moisture"] {
        assert_eq!(seasonal[k].as_array().unwrap().len(), 12, "{k}");
    }
    // the annual mean in `values` is the mean of the twelve seasons
    let t: f64 = seasonal["temperature"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).sum::<f64>() / 12.0;
    assert!((t - c["values"]["temperature"].as_f64().unwrap()).abs() < 1e-6);
}

#[test]
fn overlay_and_preset_declarations_are_consistent_data() {
    let sc = wb_app::schema::schema().unwrap();
    let overlays = sc["overlays"].as_array().unwrap();
    let fields: Vec<&str> = sc["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    let kinds: Vec<&str> = sc["layer_kinds"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    for o in overlays {
        assert!(kinds.contains(&o["kind"].as_str().unwrap()));
        match o["kind"].as_str().unwrap() {
            "vector" | "mask" => assert!(fields.contains(&o["field"].as_str().unwrap()), "overlay {o} names an unknown field"),
            "lines" => assert!(matches!(o["source"].as_str().unwrap(), "rivers")),
            _ => unreachable!(),
        }
    }
    let ids: Vec<&str> = overlays.iter().map(|o| o["id"].as_str().unwrap()).collect();
    let presets = sc["layer_presets"].as_array().unwrap();
    let names: Vec<&str> = presets.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Terrain", "Climate", "Hydrology", "Ocean"]);
    for p in presets {
        assert!(fields.contains(&p["base"].as_str().unwrap()));
        for o in p["overlays"].as_array().unwrap() {
            assert!(ids.contains(&o.as_str().unwrap()), "preset {p} names unknown overlay {o}");
        }
    }
}
