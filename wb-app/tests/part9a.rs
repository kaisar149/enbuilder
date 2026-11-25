//! Part 9a acceptance tests: everything is driven through commands and queries, no UI.
//! Golden files live in tests/golden/. Regenerate on purpose with `UPDATE_GOLDEN=1 cargo test -p wb-app`.
use serde_json::{json, Value};
use std::time::Duration;
use wb_app::png::{decode_rgb_stored, fnv1a};
use wb_app::raster::{brute, locate, pixel_cells, projection, render_field, RenderOpts};
use wb_app::{Command, Event, Query, Session, API_VERSION};

fn small(preset: &str) -> Session {
    let mut s = Session::with_preset(preset).unwrap();
    s.execute(Command::SetParam { name: "resolution".into(), value: 2.0 }).unwrap();
    s
}

fn set(s: &mut Session, k: &str, v: f64) -> Vec<String> {
    s.execute(Command::SetParam { name: k.into(), value: v }).unwrap().invalidated
}

fn golden(name: &str, actual: &Value) {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = serde_json::to_string_pretty(actual).unwrap() + "\n";
    if std::env::var("UPDATE_GOLDEN").is_ok() || !std::path::Path::new(&path).exists() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap();
    assert!(want == text, "golden mismatch for {name}: the JSON shape changed. If intended, bump API_VERSION when it is breaking and run UPDATE_GOLDEN=1");
}

#[test]
fn command_driven_session() {
    let mut s = small("desert");
    let ack = s.execute(Command::Solve).unwrap();
    assert_eq!(ack.api_version, API_VERSION);
    let job = ack.job.expect("solve returns a job id");
    assert_eq!(s.wait_job(), Some((job, "completed".to_string())));
    let ev = s.poll_events();
    assert!(ev.iter().any(|e| matches!(e, Event::JobProgress { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::JobDone { status, .. } if status == "completed")));
    assert!(ev.iter().any(|e| matches!(e, Event::FieldChanged { names, .. } if names.iter().any(|n| n == "temperature"))));
    let n = s.domain().n_cells();
    let t = s.field("temperature", 0).unwrap();
    let p = s.field("precipitation", 0).unwrap();
    assert_eq!(t.values.len(), n);
    assert!(!t.stale);
    assert!(t.values.iter().all(|v| v.is_finite() && (150.0..400.0).contains(v)), "temperatures in kelvin");
    assert!(p.values.iter().all(|v| v.is_finite() && *v >= 0.0), "no negative precipitation");
    let hot = t.values.iter().cloned().fold(f64::MIN, f64::max);
    let cold = t.values.iter().cloned().fold(f64::MAX, f64::min);
    assert!(hot - cold > 20.0, "equator-pole contrast: {hot} vs {cold}");
    let biomes = s.field("biome", 0).unwrap();
    assert!(biomes.values.iter().all(|b| *b >= 0.0 && (*b as usize) < s.biome_table().biomes.len()));
    // binary field query: header plus n little-endian f32
    let q = s.query(Query::GetField { name: "temperature".into(), layer: 0 }).unwrap();
    assert_eq!(q.binary.len(), n * 4);
    assert_eq!(q.json["mesh_id"], json!(s.mesh_id()));
    assert!(q.json["min"].as_f64().unwrap() < q.json["max"].as_f64().unwrap());
    // cell query
    let c = s.query(Query::GetCell { cell: 3 }).unwrap();
    assert_eq!(c.json["cell"], 3);
    assert!(c.json["values"]["temperature"].is_number());
    assert_eq!(s.query(Query::GetCell { cell: n }).unwrap_err().code, "bad_cell");
}

#[test]
fn errors_are_typed() {
    let mut s = small("earthlike");
    assert_eq!(s.execute(Command::LoadPreset { name: "nope".into() }).unwrap_err().code, "unknown_preset");
    assert_eq!(s.execute(Command::SetParam { name: "nope".into(), value: 1.0 }).unwrap_err().code, "bad_param");
    assert_eq!(s.execute(Command::SetParam { name: "axial_tilt_deg".into(), value: 500.0 }).unwrap_err().code, "bad_param");
    assert_eq!(s.field("temperature", 0).err().unwrap().code, "not_solved");
    assert_eq!(s.field("elevation", 9).err().unwrap().code, "bad_layer");
    assert_eq!(s.execute(Command::Redo).unwrap_err().code, "nothing_to_redo");
}

#[test]
fn json_wire_format() {
    let c: Command = serde_json::from_str(r#"{"type":"set_param","name":"seed","value":3}"#).unwrap();
    assert_eq!(c, Command::SetParam { name: "seed".into(), value: 3.0 });
    assert_eq!(serde_json::to_value(Command::Solve).unwrap(), json!({ "type": "solve" }));
    let q: Query = serde_json::from_str(r#"{"type":"get_field","name":"biome"}"#).unwrap();
    assert_eq!(q, Query::GetField { name: "biome".into(), layer: 0 });
    let e = Event::JobDone { job: 1, status: "completed".into() };
    assert_eq!(serde_json::to_value(&e).unwrap(), json!({ "type": "job_done", "job": 1, "status": "completed" }));
}

#[test]
fn golden_schema_and_world_info() {
    let mut s = Session::new().unwrap();
    let schema = s.query(Query::GetSchema).unwrap().json;
    assert_eq!(schema["api_version"], API_VERSION);
    assert_eq!(schema["presets"].as_array().unwrap().len(), 5);
    assert_eq!(schema["params"].as_array().unwrap().len(), 11);
    golden("schema.json", &schema);
    let info = s.query(Query::GetWorldInfo).unwrap().json;
    golden("world_info.json", &info);
    // after a solve only the values change, not the shape
    s.solve_blocking().unwrap();
    let solved = s.query(Query::GetWorldInfo).unwrap().json;
    assert_eq!(solved["solved"], true);
    assert_eq!(solved["domain"], info["domain"]);
}

#[test]
fn schema_comes_from_real_module_specs() {
    let mut s = Session::new().unwrap();
    let schema = s.query(Query::GetSchema).unwrap().json;
    let modules = schema["modules"].as_array().unwrap();
    assert!(modules.len() >= 8, "the real pipeline modules are listed");
    assert!(modules.iter().any(|m| !m["params"].as_array().unwrap().is_empty()), "module parameters come from Scheduler::module_params");
    // every output named by a module appears as a field entry
    let fields: Vec<&str> = schema["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    for m in modules {
        for o in m["outputs"].as_array().unwrap() {
            assert!(fields.contains(&o.as_str().unwrap()), "{o} missing from fields");
        }
    }
}

#[test]
fn undo_restores_parameter_and_invalidates_fields() {
    let mut s = small("earthlike");
    s.solve_blocking().unwrap();
    let t0 = s.field("temperature", 0).unwrap().values;
    let before = s.params()["solar_constant"];
    let preset_before = s.preset().to_string();

    // a climate-scope change: elevation untouched, solved fields stale
    let inv = set(&mut s, "solar_constant", 1700.0);
    assert!(inv.contains(&"temperature".to_string()) && !inv.contains(&"elevation".to_string()), "{inv:?}");
    assert!(s.is_stale());
    assert!(s.field("temperature", 0).unwrap().stale);
    let inv = s.execute(Command::Undo).unwrap().invalidated;
    assert!(inv.contains(&"temperature".to_string()));
    assert_eq!(s.params()["solar_constant"], before);
    assert_eq!(s.preset(), preset_before, "undo restores the preset label too");
    assert!(s.query(Query::GetWorldInfo).unwrap().json["can_redo"].as_bool().unwrap());
    s.solve_blocking().unwrap();
    assert_eq!(s.field("temperature", 0).unwrap().values, t0, "deterministic: same parameters, same world");

    // an elevation-scope change regenerates elevation; undo gives the old elevation back exactly
    let e0 = s.field("elevation", 0).unwrap().values;
    let inv = set(&mut s, "seed", 99.0);
    assert_eq!(inv[0], "elevation");
    assert_ne!(s.field("elevation", 0).unwrap().values, e0);
    s.execute(Command::Undo).unwrap();
    assert_eq!(s.field("elevation", 0).unwrap().values, e0);
    s.execute(Command::Redo).unwrap();
    assert_eq!(s.params()["seed"], 99.0);

    // a preset load is one undo step
    s.execute(Command::LoadPreset { name: "waterworld".into() }).unwrap();
    assert_eq!(s.preset(), "waterworld");
    s.execute(Command::Undo).unwrap();
    assert_eq!(s.params()["ocean_fraction"], 0.7);

    // changing the mesh drops every solved field
    set(&mut s, "resolution", 3.0);
    assert!(!s.is_solved());
    assert_eq!(s.field("temperature", 0).err().unwrap().code, "not_solved");
}

#[test]
fn cancel_leaves_a_valid_session() {
    let mut s = small("earthlike");
    s.set_step_delay(Duration::from_millis(15));
    let job = s.execute(Command::Solve).unwrap().job.unwrap();
    assert_eq!(s.execute(Command::Solve).unwrap_err().code, "job_running", "one job at a time");
    std::thread::sleep(Duration::from_millis(60));
    s.execute(Command::Cancel).unwrap();
    assert_eq!(s.wait_job(), Some((job, "cancelled".to_string())));
    let ev = s.poll_events();
    assert!(ev.iter().any(|e| matches!(e, Event::JobDone { status, .. } if status == "cancelled")));
    assert!(!ev.iter().any(|e| matches!(e, Event::FieldChanged { names, .. } if names.iter().any(|n| n == "temperature"))), "a cancelled solve publishes nothing");
    // still valid: no half-solved state, elevation and queries work, params intact
    assert!(!s.is_solved() && !s.job_running());
    assert_eq!(s.field("temperature", 0).err().unwrap().code, "not_solved");
    assert!(s.field("elevation", 0).unwrap().values.iter().all(|v| v.is_finite()));
    s.query(Query::GetWorldInfo).unwrap();
    // and a new solve works
    s.set_step_delay(Duration::ZERO);
    assert_eq!(s.solve_blocking().unwrap(), "completed");
    assert!(s.is_solved());
}

#[test]
fn changing_params_during_a_job_cancels_it() {
    let mut s = small("earthlike");
    s.set_step_delay(Duration::from_millis(15));
    s.execute(Command::Solve).unwrap();
    set(&mut s, "solar_constant", 1500.0);
    assert!(!s.job_running() && !s.is_solved());
}

// ------------------------------------------------------------------------------------------------ rasters

#[test]
fn snapshot_png_is_stable() {
    let mut s = small("earthlike");
    s.solve_blocking().unwrap();
    let opts = RenderOpts { width: 360, ..RenderOpts::default() };
    let img = render_field(&s, "temperature", &opts).unwrap();
    let png = img.to_png();
    assert_eq!(png, render_field(&s, "temperature", &opts).unwrap().to_png(), "rendering is deterministic");
    let (w, h, rgb) = decode_rgb_stored(&png).expect("our own PNG decodes");
    assert_eq!((w, h), (img.width, img.height));
    assert_eq!(rgb, img.rgb);
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    // pinned hash of the fixed-seed image (regenerate with UPDATE_GOLDEN=1 when rendering or the simulation changes on purpose)
    let hashes = json!({
        "temperature": format!("{:016x}", fnv1a(&png)),
        "biome": format!("{:016x}", fnv1a(&render_field(&s, "biome", &opts).unwrap().to_png())),
        "elevation": format!("{:016x}", fnv1a(&render_field(&s, "elevation", &opts).unwrap().to_png())),
    });
    golden("snapshot_hashes.json", &hashes);
}

#[test]
fn snapshot_looks_like_a_planet() {
    let mut s = small("earthlike");
    s.solve_blocking().unwrap();
    let opts = RenderOpts { width: 360, legend: false, ..RenderOpts::default() };
    let img = render_field(&s, "temperature", &opts).unwrap();
    // warmest mean colour band is near the equator: the red channel of the mid rows beats the polar rows
    let row_red = |y: u32| (0..img.width).map(|x| img.get(x, y)[0] as f64 - img.get(x, y)[2] as f64).sum::<f64>() / img.width as f64;
    assert!(row_red(img.height / 2) > row_red(2) + 20.0 && row_red(img.height / 2) > row_red(img.height - 3) + 20.0);
    // legend adds a strip below the map
    let with = render_field(&s, "temperature", &RenderOpts { width: 360, ..RenderOpts::default() }).unwrap();
    assert!(with.height > img.height);
}

#[test]
fn every_shape_renders_and_locates_correctly() {
    for (shape, expect_gaps) in [(0.0, false), (1.0, true), (2.0, false)] {
        let mut s = small("earthlike");
        set(&mut s, "shape", shape);
        let d = s.domain();
        let kind = wb_app::world::shape_name(s.params());
        let proj = projection(d, kind, 160).unwrap();
        let cells = pixel_cells(d, &proj);
        assert_eq!(cells.len() as u32, proj.width * proj.height);
        assert_eq!(cells.iter().any(|c| c.is_none()), expect_gaps, "{kind}: only the cube cross has empty corners");
        // greedy walk agrees with brute force on a sample of pixels
        let mut checked = 0;
        for (i, c) in cells.iter().enumerate().step_by(37) {
            if let Some(c) = c {
                let p = proj.point(i as u32 % proj.width, i as u32 / proj.width).unwrap();
                assert_eq!(*c, brute(d, p), "{kind} pixel {i}");
                let near = d.neighbors(*c)[0] as usize;
                assert_eq!(locate(d, p, near), brute(d, p), "{kind}: walk from a neighbouring cell");
                checked += 1;
            }
        }
        assert!(checked > 50);
        // every cell is visible somewhere on a map of this size (no cell is skipped)
        let mut seen = vec![false; d.n_cells()];
        cells.iter().flatten().for_each(|&c| seen[c] = true);
        let frac = seen.iter().filter(|b| **b).count() as f64 / seen.len() as f64;
        assert!(frac > 0.85, "{kind}: {frac}");
        let img = render_field(&s, "elevation", &RenderOpts { width: 160, ..RenderOpts::default() }).unwrap();
        assert!(img.height >= proj.height);
    }
}

#[test]
fn cube_cross_edges_are_continuous() {
    let mut s = small("earthlike");
    set(&mut s, "shape", 1.0);
    let proj = projection(s.domain(), "cube", 160).unwrap();
    let side = proj.width / 4;
    let step = 2.0 * 6371e3 / side as f64; // one pixel on a face
    let dist = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    for row in 0..side {
        let y = side + row;
        for f in 0..4 {
            let (a, b) = (proj.point((f + 1) * side - 1, y).unwrap(), proj.point(((f + 1) % 4) * side + if f == 3 { 0 } else { 0 }, y).unwrap());
            if f < 3 {
                assert!(dist(a, b) < 1.6 * step, "face {f}->{} row {row}: {}", f + 1, dist(a, b) / step);
            }
        }
    }
    for col in 0..side {
        let x = side + col;
        // top face bottom row touches the -y face top row; bottom face top row touches the -y face bottom row
        assert!(dist(proj.point(x, side - 1).unwrap(), proj.point(x, side).unwrap()) < 1.6 * step);
        assert!(dist(proj.point(x, 2 * side - 1).unwrap(), proj.point(x, 2 * side).unwrap()) < 1.6 * step);
    }
}

#[test]
fn categories_use_the_biome_table() {
    let mut s = small("earthlike");
    s.solve_blocking().unwrap();
    let img = render_field(&s, "biome", &RenderOpts { width: 200, legend: false, ..RenderOpts::default() }).unwrap();
    let table = s.biome_table();
    let colors: std::collections::HashSet<[u8; 3]> = table.biomes.iter().map(|b| b.color).collect();
    for y in 0..img.height {
        for x in 0..img.width {
            let px = img.get(x, y);
            assert!(colors.contains(&px), "pixel {x},{y} is not a biome colour");
        }
    }
}
