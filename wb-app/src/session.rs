//! `Session`: one world with its parameters, solved state, undo history and background job. No UI types, no rendering types.
use crate::api::*;
use crate::params::{self, Scope};
use crate::variations::{self, Batch};
use crate::world::{self, Params, SolveError, Solved};
use enbuilder::{Domain, FieldStore};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

struct Group {
    /// Short text for the history list, e.g. `surface_gravity: 9.81 -> 5`.
    label: String,
    changes: Vec<(String, f64, f64)>, // (name, old, new)
    preset_before: String,
    preset_after: String,
}

struct SolvedState {
    solved: Solved,
    version: u64,
    stale: bool,
}

/// What a background job produces.
enum JobOutput {
    Solve(Result<Solved, SolveError>),
    Variations(Result<Batch, SolveError>),
}

struct Job {
    id: u64,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<JobOutput>,
}

/// The last variation batch, tied to the mesh it was solved on (thumbnails need that mesh to be drawn).
struct VariationSet {
    mesh_id: String,
    batch: Batch,
}

pub struct Session {
    params: Params,
    preset: String,
    domain: Domain,
    elevation: FieldStore,
    mesh_id: String,
    world_version: u64,
    version: u64,
    solved: Option<SolvedState>,
    undo: Vec<Group>,
    redo: Vec<Group>,
    job: Option<Job>,
    variations: Option<VariationSet>,
    next_job: u64,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    step_delay: Duration,
}

/// A field as the clients see it (layer 0 unless asked).
pub struct FieldData {
    pub name: String,
    pub values: Vec<f64>,
    pub layers: usize,
    pub layer: usize,
    pub version: u64,
    pub stale: bool,
}

fn group_json(g: &Group) -> Value {
    json!({
        "label": g.label,
        "preset": g.preset_after,
        "changes": g.changes.iter().map(|(n, o, v)| json!({ "name": n, "old": o, "new": v })).collect::<Vec<_>>(),
    })
}

fn f32_bytes(v: &[f64]) -> Vec<u8> { v.iter().flat_map(|&x| (x as f32).to_le_bytes()).collect() }
fn u32_bytes(v: &[u32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

impl Session {
    pub fn new() -> ApiResult<Session> { Self::with_params(params::defaults(), "earthlike") }

    pub fn with_preset(name: &str) -> ApiResult<Session> {
        let p = params::find_preset(name).ok_or_else(|| ApiError::new("unknown_preset", format!("no preset '{name}'")))?;
        let mut all = params::defaults();
        all.extend(p.params);
        Self::with_params(all, name)
    }

    fn with_params(p: Params, preset: &str) -> ApiResult<Session> {
        let (domain, elevation) = world::build_world(&p)?;
        let (tx, rx) = channel();
        let mesh_id = world::mesh_id(&p);
        let s = Session {
            params: p, preset: preset.into(), domain, elevation, mesh_id: mesh_id.clone(), world_version: 1, version: 1, solved: None,
            undo: vec![], redo: vec![], job: None, variations: None, next_job: 1, tx, rx, step_delay: Duration::ZERO,
        };
        let _ = s.tx.send(Event::WorldLoaded { mesh_id, n_cells: s.domain.n_cells() });
        Ok(s)
    }

    /// Test hook: pause this long after every season of a solve (so a test can cancel mid-run).
    pub fn set_step_delay(&mut self, d: Duration) { self.step_delay = d; }

    pub fn params(&self) -> &Params { &self.params }
    pub fn preset(&self) -> &str { &self.preset }
    pub fn domain(&self) -> &Domain { &self.domain }
    pub fn mesh_id(&self) -> &str { &self.mesh_id }
    pub fn is_solved(&self) -> bool { self.solved.is_some() }
    pub fn is_stale(&self) -> bool { self.solved.as_ref().map(|s| s.stale).unwrap_or(false) }
    pub fn job_running(&self) -> bool { self.job.is_some() }
    pub fn biome_table(&self) -> enbuilder::BiomeTable { self.solved.as_ref().map(|s| s.solved.table.clone()).unwrap_or_else(world::biome_table) }

    // ------------------------------------------------------------------------------------------ commands

    pub fn execute(&mut self, cmd: Command) -> ApiResult<Ack> {
        let name = cmd.name().to_string();
        let mut job = None;
        let invalidated = match cmd {
            Command::LoadPreset { name } => {
                let pr = params::find_preset(&name).ok_or_else(|| ApiError::new("unknown_preset", format!("no preset '{name}'")))?;
                let mut target = params::defaults();
                target.extend(pr.params);
                let changes: Vec<(String, f64, f64)> = target.iter().filter(|(k, v)| self.params[*k] != **v).map(|(k, v)| (k.clone(), self.params[k], *v)).collect();
                if changes.is_empty() && self.preset == name {
                    vec![]
                } else {
                    let g = Group { label: format!("preset {name}"), changes, preset_before: self.preset.clone(), preset_after: name };
                    let inv = self.apply_group(&g, false)?;
                    self.undo.push(g);
                    self.redo.clear();
                    inv
                }
            }
            Command::SetParam { name, value } => {
                params::validate(&name, value).map_err(|m| ApiError::new("bad_param", m))?;
                let old = self.params[&name];
                if old == value {
                    vec![]
                } else {
                    let g = Group { label: format!("{name}: {old} -> {value}"), changes: vec![(name, old, value)], preset_before: self.preset.clone(), preset_after: "custom".into() };
                    let inv = self.apply_group(&g, false)?;
                    self.undo.push(g);
                    self.redo.clear();
                    inv
                }
            }
            Command::Undo => {
                let g = self.undo.pop().ok_or_else(|| ApiError::new("nothing_to_undo", "the undo stack is empty"))?;
                let inv = self.apply_group(&g, true)?;
                self.redo.push(g);
                inv
            }
            Command::Redo => {
                let g = self.redo.pop().ok_or_else(|| ApiError::new("nothing_to_redo", "the redo stack is empty"))?;
                let inv = self.apply_group(&g, false)?;
                self.undo.push(g);
                inv
            }
            Command::Solve => {
                if self.job.is_some() {
                    return Err(ApiError::new("job_running", "a solve is already running"));
                }
                job = Some(self.start_solve());
                vec![]
            }
            Command::Cancel => {
                if let Some(j) = &self.job {
                    j.cancel.store(true, Ordering::SeqCst);
                }
                vec![]
            }
            Command::Variations { count, coarse } => {
                if self.job.is_some() {
                    return Err(ApiError::new("job_running", "a job is already running"));
                }
                if count == 0 || count > variations::MAX_VARIATIONS {
                    return Err(ApiError::new("bad_param", format!("count must be in [1, {}], got {count}", variations::MAX_VARIATIONS)));
                }
                job = Some(self.start_variations(count, coarse));
                vec![]
            }
        };
        Ok(Ack { api_version: API_VERSION, command: name, invalidated, job })
    }

    /// Applies (or reverts) a parameter group. Pushes the group on the undo stack unless it comes from undo/redo.
    fn apply_group(&mut self, g: &Group, revert: bool) -> ApiResult<Vec<String>> {
        self.cancel_and_join();
        let mut scope = Scope::Climate;
        let defs = params::param_defs();
        let mut next = self.params.clone();
        for (name, old, new) in &g.changes {
            next.insert(name.clone(), if revert { *old } else { *new });
            let d = defs.iter().find(|d| &d.name == name).expect("known parameter");
            scope = scope.max(d.scope);
        }
        let preset = if revert { g.preset_before.clone() } else { g.preset_after.clone() };
        // build first, so a failure leaves the session untouched
        let rebuilt = if scope >= Scope::Elevation { Some(world::build_world(&next)?) } else { None };
        let was_solved: Vec<String> = self.solved.as_ref().map(|s| s.solved.fields.names().into_iter().filter(|n| n != "elevation").collect()).unwrap_or_default();
        self.params = next;
        self.preset = preset;
        self.version += 1;
        let mut invalidated = was_solved;
        if let Some((d, f)) = rebuilt {
            let new_mesh = world::mesh_id(&self.params);
            let mesh_changed = new_mesh != self.mesh_id;
            self.domain = d;
            self.elevation = f;
            self.mesh_id = new_mesh.clone();
            self.world_version = self.version;
            invalidated.insert(0, "elevation".into());
            if mesh_changed {
                self.solved = None;
                self.variations = None;
                let _ = self.tx.send(Event::WorldLoaded { mesh_id: new_mesh, n_cells: self.domain.n_cells() });
            }
        }
        if let Some(s) = &mut self.solved {
            s.stale = true;
        }
        if !invalidated.is_empty() {
            let _ = self.tx.send(Event::FieldChanged { names: invalidated.clone(), version: self.version });
        }
        Ok(invalidated)
    }

    fn cancel_and_join(&mut self) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::SeqCst);
        }
        let _ = self.wait_job();
    }

    // ------------------------------------------------------------------------------------------ jobs

    fn start_solve(&mut self) -> u64 {
        let id = self.next_job;
        self.next_job += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let (c2, tx, p, delay) = (cancel.clone(), self.tx.clone(), self.params.clone(), self.step_delay);
        let handle = std::thread::spawn(move || {
            let mut last = -1.0;
            JobOutput::Solve(world::solve(&p, &c2, &mut |fr, stage| {
                if fr - last >= 0.0 {
                    last = fr;
                    let _ = tx.send(Event::JobProgress { job: id, fraction: fr, stage: stage.to_string() });
                }
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }))
        });
        self.job = Some(Job { id, cancel, handle });
        id
    }

    fn start_variations(&mut self, count: usize, coarse: bool) -> u64 {
        let id = self.next_job;
        self.next_job += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let (c2, tx, p) = (cancel.clone(), Mutex::new(self.tx.clone()), self.params.clone());
        let handle = std::thread::spawn(move || {
            let progress = move |fr: f64, stage: &str| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send(Event::JobProgress { job: id, fraction: fr, stage: stage.to_string() });
                }
            };
            JobOutput::Variations(variations::run(&p, count, coarse, &c2, &progress))
        });
        self.job = Some(Job { id, cancel, handle });
        id
    }

    /// Blocks until the running job (if any) is finished and installs its result. Returns the job id and status.
    pub fn wait_job(&mut self) -> Option<(u64, String)> {
        let job = self.job.take()?;
        let id = job.id;
        let res = job.handle.join();
        Some((id, self.finish(id, res)))
    }

    /// Non-blocking: installs the result of a finished job.
    pub fn poll(&mut self) {
        if self.job.as_ref().map(|j| j.handle.is_finished()).unwrap_or(false) {
            let _ = self.wait_job();
        }
    }

    fn finish(&mut self, id: u64, res: std::thread::Result<JobOutput>) -> String {
        let status = match res {
            Ok(JobOutput::Solve(Ok(solved))) => {
                self.version += 1;
                let names = solved.fields.names();
                self.solved = Some(SolvedState { solved, version: self.version, stale: false });
                let _ = self.tx.send(Event::FieldChanged { names, version: self.version });
                "completed"
            }
            Ok(JobOutput::Variations(Ok(batch))) => {
                // parameters cannot have changed while the job ran (a change cancels and joins it first)
                self.variations = Some(VariationSet { mesh_id: self.mesh_id.clone(), batch });
                "completed"
            }
            Ok(JobOutput::Solve(Err(SolveError::Cancelled))) | Ok(JobOutput::Variations(Err(SolveError::Cancelled))) => "cancelled",
            Ok(JobOutput::Solve(Err(SolveError::Failed(m)))) | Ok(JobOutput::Variations(Err(SolveError::Failed(m)))) => {
                let _ = self.tx.send(Event::Error { message: m });
                "failed"
            }
            Err(_) => {
                let _ = self.tx.send(Event::Error { message: "the solve worker panicked".into() });
                "failed"
            }
        };
        let _ = self.tx.send(Event::JobDone { job: id, status: status.into() });
        status.into()
    }

    /// Everything pushed since the last call (installs a finished job first).
    pub fn poll_events(&mut self) -> Vec<Event> {
        self.poll();
        self.rx.try_iter().collect()
    }

    /// Convenience for headless callers: solve and wait. Returns the job status.
    pub fn solve_blocking(&mut self) -> ApiResult<String> {
        self.execute(Command::Solve)?;
        Ok(self.wait_job().map(|(_, s)| s).unwrap_or_else(|| "none".into()))
    }

    // ------------------------------------------------------------------------------------------ queries

    pub fn query(&mut self, q: Query) -> ApiResult<QueryResult> {
        self.poll();
        match q {
            Query::GetSchema => Ok(QueryResult { json: crate::schema::schema()?, binary: vec![] }),
            Query::GetWorldInfo => Ok(QueryResult { json: self.world_info(), binary: vec![] }),
            Query::GetMesh => Ok(self.mesh()),
            Query::GetField { name, layer } => self.field_query(&name, layer),
            Query::GetCell { cell } => self.cell(cell),
            Query::GetVariations => Ok(QueryResult { json: self.variations_info(), binary: vec![] }),
            Query::GetVariation { index, name } => self.variation_field(index, &name),
        }
    }

    fn world_info(&self) -> Value {
        let mut fields = vec![json!({ "name": "elevation", "layers": 1, "version": self.world_version, "stale": false })];
        if let Some(s) = &self.solved {
            for n in s.solved.fields.names() {
                if n != "elevation" {
                    fields.push(json!({ "name": n, "layers": s.solved.fields.layers(&n).unwrap_or(1), "version": s.version, "stale": s.stale }));
                }
            }
        }
        let kind = world::shape_name(&self.params);
        json!({
            "api_version": API_VERSION,
            "preset": self.preset,
            "params": self.params,
            "domain": { "kind": kind, "mesh_id": self.mesh_id, "n_cells": self.domain.n_cells(), "total_area_m2": self.domain.mesh.total_area() },
            "solved": self.solved.is_some(),
            "stale": self.is_stale(),
            "fields": fields,
            "job": self.job.as_ref().map(|j| json!({ "id": j.id, "running": true })),
            "can_undo": !self.undo.is_empty(),
            "can_redo": !self.redo.is_empty(),
            // Part 9c: `undo` is oldest first; `redo` lists what would be re-applied, next one first.
            "history": { "undo": self.undo.iter().map(group_json).collect::<Vec<_>>(), "redo": self.redo.iter().rev().map(group_json).collect::<Vec<_>>() },
        })
    }

    fn mesh(&self) -> QueryResult {
        let m = &self.domain.mesh;
        let n = m.n_cells;
        let centroid: Vec<f64> = m.centroid.iter().flat_map(|c| c.iter().copied()).collect();
        let mut bin = vec![];
        let mut layout = vec![];
        let mut add = |name: &str, dtype: &str, count: usize, bytes: Vec<u8>, bin: &mut Vec<u8>| {
            layout.push(json!({ "name": name, "dtype": dtype, "count": count, "offset": bin.len() }));
            bin.extend(bytes);
        };
        add("centroid", "f32x3", n, f32_bytes(&centroid), &mut bin);
        add("area", "f32", n, f32_bytes(&m.area), &mut bin);
        add("adj_offsets", "u32", m.adj_offsets.len(), u32_bytes(&m.adj_offsets), &mut bin);
        add("adj", "u32", m.adj.len(), u32_bytes(&m.adj), &mut bin);
        let json = json!({ "mesh_id": self.mesh_id, "kind": world::shape_name(&self.params), "n_cells": n, "endianness": "little", "layout": layout, "bytes": bin.len() });
        QueryResult { json, binary: bin }
    }

    /// Field values (layer `layer`). `elevation` always reflects the current parameters; every other field comes from the last solve.
    pub fn field(&self, name: &str, layer: usize) -> ApiResult<FieldData> {
        let (fs, version, stale) = if name == "elevation" {
            (&self.elevation, self.world_version, false)
        } else {
            let s = self.solved.as_ref().ok_or_else(|| ApiError::new("not_solved", format!("field '{name}' needs a solve first")))?;
            (&s.solved.fields, s.version, s.stale)
        };
        if !fs.has(name) {
            return Err(ApiError::new("unknown_field", format!("no field '{name}'")));
        }
        let layers = fs.layers(name)?;
        if layer >= layers {
            return Err(ApiError::new("bad_layer", format!("field '{name}' has {layers} layers")));
        }
        let d = fs.data(name)?;
        let n = self.domain.n_cells();
        Ok(FieldData { name: name.into(), values: (0..n).map(|i| d[i * layers + layer]).collect(), layers, layer, version, stale })
    }

    fn field_query(&self, name: &str, layer: usize) -> ApiResult<QueryResult> {
        let f = self.field(name, layer)?;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in &f.values {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if !lo.is_finite() {
            lo = 0.0;
            hi = 0.0;
        }
        let mut meta = crate::schema::field_json(name);
        meta["mesh_id"] = json!(self.mesh_id);
        meta["version"] = json!(f.version);
        meta["stale"] = json!(f.stale);
        meta["layer"] = json!(f.layer);
        meta["layers"] = json!(f.layers);
        meta["min"] = json!(lo);
        meta["max"] = json!(hi);
        meta["dtype"] = json!("f32");
        meta["count"] = json!(f.values.len());
        Ok(QueryResult { json: meta, binary: f32_bytes(&f.values) })
    }

    /// The seeds of the last batch, or an empty list when there is none (or the mesh has changed since).
    fn variations_info(&self) -> Value {
        match self.variations.as_ref().filter(|v| v.mesh_id == self.mesh_id) {
            Some(v) => json!({
                "mesh_id": v.mesh_id, "base_seed": v.batch.base_seed, "coarse": v.batch.coarse,
                "fields": variations::KEPT_FIELDS,
                "items": v.batch.items.iter().map(|i| json!({ "index": i.index, "seed": i.seed })).collect::<Vec<_>>(),
            }),
            None => json!({ "mesh_id": self.mesh_id, "base_seed": Value::Null, "coarse": Value::Null, "fields": variations::KEPT_FIELDS, "items": [] }),
        }
    }

    /// One field of one variation, in the same binary layout as `get_field` (layer 0, f32).
    fn variation_field(&self, index: usize, name: &str) -> ApiResult<QueryResult> {
        let set = self.variations.as_ref().filter(|v| v.mesh_id == self.mesh_id).ok_or_else(|| ApiError::new("no_variations", "no variations yet: send `variations` first"))?;
        let item = set.batch.items.get(index).ok_or_else(|| ApiError::new("bad_variation", format!("variation {index} out of range (have {})", set.batch.items.len())))?;
        let values = item.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v).ok_or_else(|| ApiError::new("unknown_field", format!("variations keep only {:?}", variations::KEPT_FIELDS)))?;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in values {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if !lo.is_finite() {
            lo = 0.0;
            hi = 0.0;
        }
        let mut meta = crate::schema::field_json(name);
        meta["mesh_id"] = json!(self.mesh_id);
        meta["variation"] = json!(index);
        meta["seed"] = json!(item.seed);
        meta["version"] = json!(0);
        meta["stale"] = json!(false);
        meta["layer"] = json!(0);
        meta["layers"] = json!(1);
        meta["min"] = json!(lo);
        meta["max"] = json!(hi);
        meta["dtype"] = json!("f32");
        meta["count"] = json!(values.len());
        Ok(QueryResult { json: meta, binary: f32_bytes(values) })
    }

    fn cell(&self, cell: usize) -> ApiResult<QueryResult> {
        let n = self.domain.n_cells();
        if cell >= n {
            return Err(ApiError::new("bad_cell", format!("cell {cell} out of range (n_cells = {n})")));
        }
        let c = self.domain.mesh.centroid[cell];
        let up = self.domain.mesh.up[cell];
        let mut values = serde_json::Map::new();
        values.insert("elevation".into(), json!(self.elevation.data("elevation")?[cell]));
        let mut biome_name = Value::Null;
        if let Some(s) = &self.solved {
            for name in s.solved.fields.names() {
                if name == "elevation" {
                    continue;
                }
                let l = s.solved.fields.layers(&name)?;
                values.insert(name.clone(), json!(s.solved.fields.data(&name)?[cell * l]));
            }
            if let Ok(b) = s.solved.fields.data("biome") {
                biome_name = json!(s.solved.table.name(b[cell] as usize));
            }
        }
        let mut j = json!({
            "cell": cell, "centroid": c, "area_m2": self.domain.mesh.area[cell], "values": values, "biome_name": biome_name,
            "neighbors": self.domain.neighbors(cell), "stale": self.is_stale(),
        });
        if world::shape_name(&self.params) == "sphere" {
            j["lat_deg"] = json!(up[2].clamp(-1.0, 1.0).asin().to_degrees());
            j["lon_deg"] = json!(up[1].atan2(up[0]).to_degrees());
        }
        Ok(QueryResult { json: j, binary: vec![] })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::SeqCst);
            let _ = j.handle.join();
        }
    }
}
