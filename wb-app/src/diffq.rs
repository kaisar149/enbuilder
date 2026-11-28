//! Part 9g: diff between two stored versions, the propagation front, and the "why did this change?" explainer.
//!
//! No physics here: this module only (1) builds the core's `ChangeSet` from two stored solved states, (2) calls the core's `field_diff` and
//! `explain`, (3) serialises their results. The UI renders what comes out and infers nothing.
//!
//! STORED VERSION (`Snap`): one installed solve or refresh (and every undo that swaps a solved state back in). It holds the solved fields
//! (`Arc`), the four output views as the user saw them (physics + paint + locks, see `session::View`), the pins the solve used, and a label.
//!
//! FRONT. The core records a per-cell scheduler tick only when a traced `run` is made (`Trace`); the app's land-cycle solve is not traced, so
//! the front here is the core's own fallback order: `(front tick = never, surface distance from the edit seeds, cell id)`. `front` on the wire is
//! the REVEAL TIME in [0, 1] of a changed cell (its rank in that order divided by (changed - 1)) and -1 for unchanged cells; the playhead `t`
//! shows exactly the cells with `0 <= front <= t`. `front_kind` says where the seeds came from: `edit` (cells whose elevation or pin differ
//! between the two versions) or `peak` (no edit cells, e.g. a parameter change: distance from the cell with the largest change of this field).
use crate::api::{ApiError, ApiResult, QueryResult};
use crate::paint::OUTPUT_FIELDS;
use crate::schema::field_json;
use crate::world::{self, Solved};
use enbuilder::{field_diff, grow_region, ChangeSet, DiffOptions, Domain, ExplainOptions, FieldChange, FieldStore, Hop, Provenance};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Stored versions kept per session (oldest dropped first).
pub const MAX_SNAPSHOTS: usize = 8;

#[derive(Clone)]
pub struct Snap {
    pub version: u64,
    pub label: String,
    pub quality: &'static str,
    pub mesh_id: String,
    pub solved: Arc<Solved>,
    /// The output fields as the clients saw them when this version was installed.
    pub views: BTreeMap<&'static str, Vec<f64>>,
    /// Pinned cells (field -> cell -> value) the solve used.
    pub pins: BTreeMap<String, BTreeMap<u32, f64>>,
    pub generation: u64,
}

/// Everything derived from one (from, to) pair; cached by the session because summary, field diffs and explanations all start here.
pub struct Bundle {
    pub from: u64,
    pub to: u64,
    pub cs: ChangeSet,
    /// `to` fields with the output views put in, the store `field_diff` reads current values from.
    pub store: FieldStore,
    pub seeds: Vec<u32>,
    pub pin_labels: Vec<(String, Vec<u32>)>,
    pub dist: Vec<f64>,
}

fn pins_map(p: &[(String, Vec<u32>, Vec<f64>)]) -> BTreeMap<String, BTreeMap<u32, f64>> {
    p.iter().map(|(f, c, v)| (f.clone(), c.iter().copied().zip(v.iter().copied()).collect())).collect()
}

pub fn pins_of(inp: &world::EditInput) -> BTreeMap<String, BTreeMap<u32, f64>> { pins_map(&inp.pins) }

pub fn bad_version(v: u64) -> ApiError { ApiError::new("unknown_version", format!("version {v} is not stored (the last {MAX_SNAPSHOTS} solves are kept)")) }

/// ChangeSet between two stored states: every value that differs, over the fields both have with the same layer count; the four output
/// fields come from the stored views.
fn change_set(a: &Snap, b: &Snap) -> ApiResult<(ChangeSet, FieldStore)> {
    let n = b.solved.domain.n_cells();
    let mut store = b.solved.fields.fork();
    for out in OUTPUT_FIELDS {
        if let Some(v) = b.views.get(out) {
            store.put(out, 1, v.clone())?;
        }
    }
    let mut fields: BTreeMap<String, FieldChange> = BTreeMap::new();
    for name in b.solved.fields.names() {
        let (Ok(la), Ok(lb)) = (a.solved.fields.layers(&name), b.solved.fields.layers(&name)) else { continue };
        if la != lb {
            continue;
        }
        let (da, db): (Vec<f64>, Vec<f64>) = if let (true, Some(va), Some(vb)) = (OUTPUT_FIELDS.contains(&name.as_str()), a.views.get(name.as_str()), b.views.get(name.as_str())) {
            (va.clone(), vb.clone())
        } else {
            (a.solved.fields.data(&name)?.to_vec(), b.solved.fields.data(&name)?.to_vec())
        };
        if da.len() != db.len() {
            continue;
        }
        let entries: Vec<(u32, f64, f64)> = da.iter().zip(db.iter()).enumerate().filter(|(_, (x, y))| x != y && !(x.is_nan() && y.is_nan())).map(|(i, (x, y))| (i as u32, *x, *y)).collect();
        if !entries.is_empty() {
            fields.insert(name.clone(), FieldChange { layers: la, entries, created: false });
        }
    }
    Ok((ChangeSet { since: a.version, n_cells: n, fields }, store))
}

pub fn build(a: &Snap, b: &Snap) -> ApiResult<Bundle> {
    if a.mesh_id != b.mesh_id {
        return Err(ApiError::new("mesh_mismatch", "the two versions were solved on different meshes and cannot be compared"));
    }
    let (cs, store) = change_set(a, b)?;
    let n = b.solved.domain.n_cells();
    // edit seeds: cells whose elevation view differs, plus cells whose pinned value differs between the versions
    let mut seeds: Vec<u32> = match (a.views.get("elevation"), b.views.get("elevation")) {
        (Some(x), Some(y)) => (0..n as u32).filter(|&i| x[i as usize] != y[i as usize]).collect(),
        _ => vec![],
    };
    let mut pin_labels: Vec<(String, Vec<u32>)> = vec![];
    for f in a.pins.keys().chain(b.pins.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (pa, pb) = (a.pins.get(f), b.pins.get(f));
        let cells: Vec<u32> = pa.into_iter().chain(pb).flat_map(|m| m.keys().copied()).collect::<std::collections::BTreeSet<u32>>().into_iter().filter(|c| pa.and_then(|m| m.get(c)) != pb.and_then(|m| m.get(c))).collect();
        if !cells.is_empty() {
            seeds.extend(&cells);
            pin_labels.push((f.clone(), cells));
        }
    }
    seeds.sort_unstable();
    seeds.dedup();
    let dist = if seeds.is_empty() { vec![] } else { grow_region(&b.solved.domain, &seeds, f64::INFINITY).dist };
    Ok(Bundle { from: a.version, to: b.version, cs, store, seeds, pin_labels, dist })
}

/// Default significance threshold: a change counts when it is at least this fraction of the largest change of ITS field. Re-running a solve
/// with no edit drifts by roughly 0.2 % (temperature) to 4 % (rainfall) of the effect of a real edit, so 1 % hides numerical noise while the
/// header still reports how many cells were hidden.
pub const DEFAULT_MIN_REL: f64 = 0.01;

/// The change set restricted to changes of at least `min_rel` x the largest change of their field and 1e-4 of the field's value range
/// (`min_rel = 0` keeps everything).
pub fn significant(cs: &ChangeSet, store: &FieldStore, min_rel: f64) -> ChangeSet {
    let mut out = ChangeSet { since: cs.since, n_cells: cs.n_cells, fields: BTreeMap::new() };
    for (name, fc) in &cs.fields {
        let peak = fc.entries.iter().map(|e| (e.2 - e.1).abs()).fold(0.0, f64::max);
        // absolute floor: a change must also be at least 1e-4 of the field's own value range (kills pure round-off such as a 1e-12 wind drift)
        let floor = if min_rel > 0.0 {
            store.data(name).map(|d| d.iter().cloned().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |a, v| (a.0.min(v), a.1.max(v)))).map(|(lo, hi)| 1e-4 * (hi - lo).max(0.0)).unwrap_or(0.0)
        } else {
            0.0
        };
        let keep: Vec<(u32, f64, f64)> = fc.entries.iter().copied().filter(|e| (e.2 - e.1).abs() >= (min_rel * peak).max(floor) && e.2 != e.1).collect();
        if !keep.is_empty() {
            out.fields.insert(name.clone(), FieldChange { layers: fc.layers, entries: keep, created: fc.created });
        }
    }
    out
}

fn f32_bytes(v: &[f64]) -> Vec<u8> { v.iter().flat_map(|&x| (x as f32).to_le_bytes()).collect() }

/// `get_diff`: header + binary `delta`, `before`, `after`, `front`, `dist_km` (f32 each, n_cells values, in that order).
pub fn diff_query(bundle: &Bundle, domain: &Domain, field: &str, mesh_id: &str, min_rel: f64) -> ApiResult<QueryResult> {
    if !bundle.store.has(field) {
        return Err(ApiError::new("unknown_field", format!("no field '{field}'")));
    }
    let n = domain.n_cells();
    let cs = significant(&bundle.cs, &bundle.store, min_rel);
    let raw_changed = bundle.cs.fields.get(field).map(|fc| fc.per_cell().len()).unwrap_or(0);
    let peak_cell = cs.fields.get(field).and_then(|fc| fc.per_cell().into_iter().max_by(|x, y| (x.3 - x.2).abs().partial_cmp(&(y.3 - y.2).abs()).unwrap_or(std::cmp::Ordering::Equal)).map(|t| t.0));
    let (seeds, kind): (Vec<u32>, &str) = if !bundle.seeds.is_empty() { (bundle.seeds.clone(), "edit") } else if let Some(c) = peak_cell { (vec![c], "peak") } else { (vec![], "none") };
    let fd = field_diff(domain, &bundle.store, &cs, field, &seeds, None, &DiffOptions::default())?;
    let k = fd.order.len();
    let mut front = vec![-1.0f64; n];
    for (r, &c) in fd.order.iter().enumerate() {
        front[c as usize] = if k <= 1 { 0.0 } else { r as f64 / (k - 1) as f64 };
    }
    let dist_km: Vec<f64> = fd.dist.iter().map(|d| if d.is_finite() { d / 1e3 } else { -1.0 }).collect();
    let mut meta = field_json(field);
    let s = &fd.summary;
    meta["field"] = json!(field);
    meta["from_version"] = json!(bundle.from);
    meta["to_version"] = json!(bundle.to);
    meta["mesh_id"] = json!(mesh_id);
    meta["n_cells"] = json!(n);
    meta["layers"] = json!(fd.layers);
    meta["no_change"] = json!(s.changed_cells == 0);
    meta["min_rel"] = json!(min_rel);
    meta["hidden_cells"] = json!(raw_changed - s.changed_cells);
    meta["changed_cells_exact"] = json!(raw_changed);
    meta["front_kind"] = json!(kind);
    meta["layout"] = json!(["delta", "before", "after", "front", "dist_km"].iter().enumerate().map(|(k, name)| json!({ "name": name, "dtype": "f32", "count": n, "offset": k * n * 4 })).collect::<Vec<_>>());
    meta["summary"] = json!({
        "changed_cells": s.changed_cells, "peak_abs": s.peak_abs, "mean_abs": s.mean_abs, "changed_area": s.changed_area,
        "global_mean_shift": s.global_mean_shift, "reach_km": s.reach_m / 1e3, "edit_cells": bundle.seeds.len(),
    });
    let mut bin = Vec::with_capacity(n * 20);
    for arr in [&fd.signed, &fd.before, &fd.after, &front, &dist_km] {
        bin.extend(f32_bytes(arr));
    }
    Ok(QueryResult { json: meta, binary: bin })
}

/// `get_diff_summary`: one row per changed field (cells changed, largest change, mean change, distance reached) plus totals.
pub fn summary_query(bundle: &Bundle, domain: &Domain, mesh_id: &str, min_rel: f64) -> Value {
    let n = domain.n_cells();
    let cs = significant(&bundle.cs, &bundle.store, min_rel);
    let near = |c: usize| bundle.dist.get(c).copied().filter(|d| d.is_finite());
    let mut rows = vec![];
    let mut any = vec![false; n];
    for (name, fc) in &cs.fields {
        let pc = fc.per_cell();
        let peak = pc.iter().map(|t| (t.3 - t.2).abs()).fold(0.0, f64::max);
        let mean = pc.iter().map(|t| (t.3 - t.2).abs()).sum::<f64>() / pc.len().max(1) as f64;
        let reach = if bundle.dist.is_empty() { 0.0 } else { pc.iter().filter(|t| (t.3 - t.2).abs() >= 0.01 * peak).filter_map(|t| near(t.0 as usize)).fold(0.0, f64::max) };
        for t in &pc {
            any[t.0 as usize] = true;
        }
        let meta = field_json(name);
        rows.push(json!({
            "field": name, "kind": meta["kind"], "unit": meta["unit"], "display": meta["display"], "featured": meta["featured"],
            "layers": fc.layers, "changed_cells": pc.len(), "peak_abs": peak, "mean_abs": mean, "reach_km": reach / 1e3,
        }));
    }
    rows.sort_by(|a, b| b["featured"].as_bool().cmp(&a["featured"].as_bool()).then(b["changed_cells"].as_u64().cmp(&a["changed_cells"].as_u64())).then(a["field"].as_str().cmp(&b["field"].as_str())));
    let changed_any = any.iter().filter(|b| **b).count();
    let reach = rows.iter().filter_map(|r| r["reach_km"].as_f64()).fold(0.0, f64::max);
    json!({
        "mesh_id": mesh_id, "from_version": bundle.from, "to_version": bundle.to, "n_cells": n, "no_change": cs.fields.is_empty(), "min_rel": min_rel, "fields_changed_exact": bundle.cs.fields.len(),
        "changed_cells_any_field": changed_any, "fields_changed": rows.len(), "edit_cells": bundle.seeds.len(), "reach_km": reach,
        "edit_fields": bundle.pin_labels.iter().map(|(f, c)| json!({ "field": f, "cells": c.len() })).collect::<Vec<_>>(),
        "fields": rows,
    })
}

fn hop_name(h: Hop) -> &'static str {
    match h {
        Hop::Local => "local",
        Hop::Global => "global",
        Hop::Origin => "origin",
        Hop::End => "end",
    }
}

/// `explain`: the core's causal chain as data. Every field of a step is copied from the core's `Link`; `text` is the core's own
/// `Explanation::text()`. `source_cells` are the cells a click on the step highlights (the cause the chain moved to, plus the equally ranked
/// alternatives; for the origin step, the cells carrying the origin contributor).
pub fn explain_query(bundle: &Bundle, domain: &Domain, b: &Snap, field: &str, cell: usize, min_rel: f64) -> ApiResult<QueryResult> {
    let cs = significant(&bundle.cs, &bundle.store, min_rel);
    let n = domain.n_cells();
    if cell >= n {
        return Err(ApiError::new("bad_cell", format!("cell {cell} out of range ({n} cells)")));
    }
    if !bundle.store.has(field) {
        return Err(ApiError::new("unknown_field", format!("no field '{field}'")));
    }
    let specs = world::pipeline()?.specs().to_vec();
    let mut prov = Provenance::from_producers(n, &specs)?;
    if !bundle.seeds.is_empty() {
        // the user's edits: elevation paint (and elevation locks) replace the terrain, pins hold a field at a value
        let z_cells: Vec<u32> = cs.cells("elevation");
        prov.mark("elevation", &z_cells, "pin:painted elevation")?;
        for (f, cells) in &bundle.pin_labels {
            prov.mark(f, cells, &format!("pin:locked {f}"))?;
        }
    }
    let ex = enbuilder::explain(domain, &specs, &cs, &prov, field, cell, &ExplainOptions::for_domain(domain))?;
    let steps: Vec<Value> = ex
        .chain
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut src: Vec<u32> = vec![];
            if let Some(next) = ex.chain.get(i + 1) {
                src.push(next.cell);
            }
            src.extend(l.also.iter().map(|(_, c)| *c));
            if l.hop == Hop::Origin {
                for o in &ex.origin {
                    src.extend(prov.cells_with(&l.field, o).into_iter().take(2000));
                }
            }
            src.sort_unstable();
            src.dedup();
            let meta = field_json(&l.field);
            json!({
                "index": i, "field": l.field, "cell": l.cell, "old": l.old, "new": l.new, "delta": l.new - l.old,
                "unit": meta["unit"], "display": meta["display"], "kind": meta["kind"],
                "module": l.module, "hop": hop_name(l.hop), "dist_km": l.dist_to_cause_m / 1e3,
                "contributors": l.contributors,
                "modules": l.contributors.iter().filter_map(|c| c.strip_prefix("module:")).collect::<Vec<_>>(),
                "anomalies": l.contributors.iter().filter_map(|c| c.strip_prefix("anomaly:")).collect::<Vec<_>>(),
                "also": l.also.iter().map(|(f, c)| json!({ "field": f, "cell": c })).collect::<Vec<_>>(),
                "via_feedback": l.via_feedback, "source_cells": src,
            })
        })
        .collect();
    let json = json!({
        "mesh_id": b.mesh_id, "from_version": bundle.from, "to_version": bundle.to, "field": field, "cell": cell, "min_rel": min_rel,
        "changed": !ex.chain.is_empty(), "found_origin": ex.found_origin(), "has_global_hop": ex.has_global_hop(),
        "origin": ex.origin, "stopped": ex.stopped, "steps": steps, "text": ex.text(),
        "note": "Each link is a measured change plus a real module dependency; choosing among several candidate causes is a ranking, not a proof.",
    });
    Ok(QueryResult { json, binary: vec![] })
}

pub fn versions_json(snaps: &[Snap], mesh_id: &str, current: Option<u64>) -> Value {
    json!({
        "mesh_id": mesh_id, "current": current, "max_stored": MAX_SNAPSHOTS,
        "versions": snaps.iter().filter(|s| s.mesh_id == mesh_id).map(|s| json!({
            "version": s.version, "label": s.label, "quality": s.quality, "generation": s.generation, "n_cells": s.solved.domain.n_cells(),
            "edit_cells": s.pins.values().map(|m| m.len()).sum::<usize>(),
        })).collect::<Vec<_>>(),
    })
}
