//! Part 9c acceptance tests at the session level (headless): history list, variations job and queries, schema additions.
//! `cargo test -p wb-app --test part9c`
use serde_json::{json, Value};
use wb_app::{Command, Event, Query, Session};

fn small() -> Session {
    let mut s = Session::with_preset("earthlike").unwrap();
    for (k, v) in [("resolution", 2.0), ("spinup_years", 1.0)] {
        s.execute(Command::SetParam { name: k.into(), value: v }).unwrap();
    }
    s
}

fn info(s: &mut Session) -> Value { s.query(Query::GetWorldInfo).unwrap().json }

#[test]
fn schema_carries_groups_identity_and_seasons() {
    let mut s = Session::new().unwrap();
    let schema = s.query(Query::GetSchema).unwrap().json;
    let groups = schema["groups"].as_array().unwrap();
    let names: Vec<&str> = groups.iter().map(|g| g["name"].as_str().unwrap()).collect();
    for p in schema["params"].as_array().unwrap() {
        assert!(names.contains(&p["group"].as_str().unwrap()), "every parameter group has a display entry");
    }
    assert!(groups.iter().any(|g| g["advanced"] == true), "at least one advanced group");
    let ident: Vec<&str> = schema["params"].as_array().unwrap().iter().filter(|p| p["identity"] == true).map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(ident, ["shape", "resolution", "radius_km", "seed"]);
    assert_eq!(schema["seasons"]["count"], 12);
    assert_eq!(schema["seasons"]["variants"]["temperature"], "season_temperature");
}

#[test]
fn season_layers_are_served_by_get_field() {
    let mut s = small();
    s.solve_blocking().unwrap();
    let w = info(&mut s);
    let f = w["fields"].as_array().unwrap().iter().find(|f| f["name"] == "season_temperature").expect("season_temperature is in world_info");
    assert_eq!(f["layers"], 12);
    let q0 = s.query(Query::GetField { name: "season_temperature".into(), layer: 0 }).unwrap();
    let q6 = s.query(Query::GetField { name: "season_temperature".into(), layer: 6 }).unwrap();
    assert_eq!(q0.binary.len(), s.domain().n_cells() * 4);
    assert_ne!(q0.binary, q6.binary, "winter and summer differ with a 23 degree tilt");
    assert_eq!(s.query(Query::GetField { name: "season_temperature".into(), layer: 12 }).unwrap_err().code, "bad_layer");
}

#[test]
fn history_lists_changes_and_follows_undo_redo() {
    let mut s = Session::new().unwrap();
    assert_eq!(info(&mut s)["history"], json!({ "undo": [], "redo": [] }));
    s.execute(Command::SetParam { name: "surface_gravity".into(), value: 5.0 }).unwrap();
    s.execute(Command::LoadPreset { name: "desert".into() }).unwrap();
    s.execute(Command::SetParam { name: "axial_tilt_deg".into(), value: 60.0 }).unwrap();
    let h = info(&mut s)["history"].clone();
    let undo = h["undo"].as_array().unwrap();
    assert_eq!(undo.len(), 3);
    assert_eq!(undo[0]["label"], "surface_gravity: 9.81 -> 5");
    assert_eq!(undo[1]["label"], "preset desert");
    assert_eq!(undo[2]["changes"][0], json!({ "name": "axial_tilt_deg", "old": 23.4, "new": 60.0 }));
    s.execute(Command::Undo).unwrap();
    s.execute(Command::Undo).unwrap();
    let h = info(&mut s)["history"].clone();
    assert_eq!(h["undo"].as_array().unwrap().len(), 1);
    assert_eq!(h["redo"][0]["label"], "preset desert", "redo lists the next step first");
    // a new change clears the redo list
    s.execute(Command::SetParam { name: "solar_constant".into(), value: 1500.0 }).unwrap();
    assert!(info(&mut s)["history"]["redo"].as_array().unwrap().is_empty());
}

#[test]
fn invalid_values_are_rejected_not_clamped() {
    let mut s = Session::new().unwrap();
    let e = s.execute(Command::SetParam { name: "axial_tilt_deg".into(), value: 120.0 }).unwrap_err();
    assert_eq!(e.code, "bad_param");
    assert!(e.message.contains("axial_tilt_deg"), "{}", e.message);
    assert_eq!(s.params()["axial_tilt_deg"], 23.4, "unchanged");
    assert!(info(&mut s)["history"]["undo"].as_array().unwrap().is_empty());
}

#[test]
fn variations_job_end_to_end() {
    let mut s = small();
    assert_eq!(s.query(Query::GetVariations).unwrap().json["items"], json!([]));
    assert_eq!(s.query(Query::GetVariation { index: 0, name: "biome".into() }).unwrap_err().code, "no_variations");
    assert_eq!(s.execute(Command::Variations { count: 0, coarse: true }).unwrap_err().code, "bad_param");
    assert_eq!(s.execute(Command::Variations { count: 99, coarse: true }).unwrap_err().code, "bad_param");
    let ack = s.execute(Command::Variations { count: 6, coarse: true }).unwrap();
    assert_eq!(ack.command, "variations");
    let job = ack.job.unwrap();
    assert_eq!(s.execute(Command::Solve).unwrap_err().code, "job_running");
    assert_eq!(s.wait_job(), Some((job, "completed".to_string())));
    let ev = s.poll_events();
    assert!(ev.iter().any(|e| matches!(e, Event::JobDone { job: j, status } if *j == job && status == "completed")));
    assert!(ev.iter().any(|e| matches!(e, Event::JobProgress { stage, .. } if stage.starts_with("variation"))));
    // no parameter and no undo step was touched
    assert_eq!(s.params()["seed"], 7.0);
    assert!(info(&mut s)["history"]["undo"].as_array().unwrap().len() == 2, "only the two setup changes");
    let v = s.query(Query::GetVariations).unwrap().json;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 6);
    let mut seeds: Vec<u64> = items.iter().map(|i| i["seed"].as_u64().unwrap()).collect();
    seeds.sort();
    seeds.dedup();
    assert_eq!(seeds.len(), 6);
    let n = s.domain().n_cells();
    let b = s.query(Query::GetVariation { index: 2, name: "biome".into() }).unwrap();
    assert_eq!(b.binary.len(), n * 4);
    assert_eq!(b.json["seed"], items[2]["seed"]);
    assert_eq!(b.json["mesh_id"], json!(s.mesh_id()));
    assert_eq!(s.query(Query::GetVariation { index: 6, name: "biome".into() }).unwrap_err().code, "bad_variation");
    assert_eq!(s.query(Query::GetVariation { index: 0, name: "wind".into() }).unwrap_err().code, "unknown_field");
    // reproducible: opening variation 2 (set seed, solve) gives the same biome map as its thumbnail
    let seed = items[2]["seed"].as_f64().unwrap();
    s.execute(Command::SetParam { name: "seed".into(), value: seed }).unwrap();
    s.solve_blocking().unwrap();
    let solved = s.query(Query::GetField { name: "biome".into(), layer: 0 }).unwrap();
    assert_eq!(solved.binary, b.binary, "the thumbnail is the world you get when you open it");
    // the batch survives a climate change but a new mesh drops it
    s.execute(Command::SetParam { name: "solar_constant".into(), value: 1400.0 }).unwrap();
    assert_eq!(s.query(Query::GetVariations).unwrap().json["items"].as_array().unwrap().len(), 6);
    s.execute(Command::SetParam { name: "resolution".into(), value: 1.0 }).unwrap();
    assert!(s.query(Query::GetVariations).unwrap().json["items"].as_array().unwrap().is_empty());
}

#[test]
fn a_parameter_change_cancels_a_running_variation_job() {
    let mut s = small();
    s.execute(Command::Variations { count: 4, coarse: true }).unwrap();
    s.execute(Command::SetParam { name: "solar_constant".into(), value: 1300.0 }).unwrap();
    assert!(!s.job_running());
}
