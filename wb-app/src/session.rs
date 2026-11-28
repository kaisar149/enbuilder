//! `Session`: one world with its parameters, solved state, undo history and background job. No UI types, no rendering types.
use crate::api::*;
use crate::diffq::{self, Bundle, Snap};
use crate::paint::{self, Edits, StrokeSpec, OUTPUT_FIELDS, PAINT_LAYERS};
use crate::params::{self, Scope};
use crate::variations::{self, Batch};
use crate::world::{self, EditInput, Params, Quality, RefreshStats, SolveError, Solved};
use enbuilder::{Domain, FieldStore};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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

/// Part 9f: one brush stroke. Elevation strokes also keep a snapshot of the solved state on the OTHER side of the step, swapped in on every
/// undo / redo, so undo restores the climate exactly (not just the paint).
struct StrokeStep {
    label: String,
    mesh_id: String,
    layer: String,
    cells: Vec<u32>,
    old: Vec<(f64, f64)>,
    new: Vec<(f64, f64)>,
    other: Option<Option<SolvedState>>, // Some(..) only for elevation strokes
}

struct LockStep {
    label: String,
    mesh_id: String,
    field: String,
    cells: Vec<u32>,
    before: Vec<Option<f64>>,
    after: Vec<Option<f64>>,
}

struct RealismStep {
    label: String,
    mesh_id: String,
    changes: Vec<(String, f64, f64)>,
}

/// One entry of the unified history (undo / redo share one stack).
enum Step {
    Params(Group),
    Stroke(StrokeStep),
    Lock(LockStep),
    Realism(RealismStep),
}

#[derive(Clone)]
struct SolvedState {
    solved: Arc<Solved>,
    version: u64,
    stale: bool,
    quality: Quality,
    /// Content hash of the elevation this state was solved with (the state is stale when the current elevation view differs).
    elev_hash: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Solve,
    Refresh(Quality),
    Variations,
}

/// What a background job produces.
enum JobOutput {
    Solve(Result<Solved, SolveError>),
    Refresh(Result<(Solved, RefreshStats), SolveError>, Quality),
    Variations(Result<Batch, SolveError>),
}

struct Job {
    id: u64,
    kind: JobKind,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<JobOutput>,
}

/// The last variation batch, tied to the mesh it was solved on (thumbnails need that mesh to be drawn).
struct VariationSet {
    mesh_id: String,
    batch: Batch,
}

/// The fields as the clients see them: computed physics, then paint blended by realism, then locks. Rebuilt after every mutation.
#[derive(Default)]
struct View {
    data: BTreeMap<&'static str, Vec<f64>>,
    hashes: BTreeMap<&'static str, u64>,
    versions: BTreeMap<&'static str, u64>,
}

pub struct Session {
    params: Params,
    preset: String,
    domain: Domain,
    /// COMPUTED elevation (before paint and locks).
    elevation: FieldStore,
    mesh_id: String,
    world_version: u64,
    version: u64,
    solved: Option<SolvedState>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Part 9f: edits (paint, locks, realism) per mesh, so undoing a mesh change brings the paint of that mesh back.
    edits: BTreeMap<String, Edits>,
    view: View,
    edit_generation: u64,
    last_refresh: Option<(Quality, RefreshStats)>,
    job: Option<Job>,
    variations: Option<VariationSet>,
    /// Part 9g: stored versions for diff and explain (newest last, at most `diffq::MAX_SNAPSHOTS`) and the cached comparison.
    snaps: Vec<Snap>,
    diff_cache: Option<Bundle>,
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
        "kind": "params",
        "label": g.label,
        "preset": g.preset_after,
        "changes": g.changes.iter().map(|(n, o, v)| json!({ "name": n, "old": o, "new": v })).collect::<Vec<_>>(),
    })
}

fn step_json(s: &Step, preset: &str) -> Value {
    match s {
        Step::Params(g) => group_json(g),
        Step::Stroke(t) => json!({ "kind": "stroke", "label": t.label, "preset": preset, "changes": [], "layer": t.layer, "cells": t.cells.len() }),
        Step::Lock(t) => json!({ "kind": "lock", "label": t.label, "preset": preset, "changes": [], "field": t.field, "cells": t.cells.len() }),
        Step::Realism(t) => json!({
            "kind": "realism", "label": t.label, "preset": preset,
            "changes": t.changes.iter().map(|(n, o, v)| json!({ "name": n, "old": o, "new": v })).collect::<Vec<_>>(),
        }),
    }
}

fn f32_bytes(v: &[f64]) -> Vec<u8> { v.iter().flat_map(|&x| (x as f32).to_le_bytes()).collect() }
fn u32_bytes(v: &[u32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

fn out_index(field: &str) -> Option<usize> { OUTPUT_FIELDS.iter().position(|f| *f == field) }

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
        let mut s = Session {
            params: p, preset: preset.into(), domain, elevation, mesh_id: mesh_id.clone(), world_version: 1, version: 1, solved: None,
            undo: vec![], redo: vec![], edits: BTreeMap::new(), view: View::default(), edit_generation: 0, last_refresh: None,
            job: None, variations: None, snaps: vec![], diff_cache: None, next_job: 1, tx, rx, step_delay: Duration::ZERO,
        };
        s.rebuild_view(&[]);
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
    pub fn is_stale(&self) -> bool {
        let eh = self.view.hashes.get("elevation").copied();
        self.solved.as_ref().map(|s| s.stale || Some(s.elev_hash) != eh).unwrap_or(false)
    }
    pub fn job_running(&self) -> bool { self.job.is_some() }
    pub fn biome_table(&self) -> enbuilder::BiomeTable { self.solved.as_ref().map(|s| s.solved.table.clone()).unwrap_or_else(world::biome_table) }
    /// Which quality the current solved state has (`None` before the first solve).
    pub fn quality(&self) -> Option<&'static str> { self.solved.as_ref().map(|s| s.quality.name()) }
    /// FNV-1a content hash of the view of an output field (the acceptance tests' "field versions equal").
    pub fn content_hash(&self, field: &str) -> Option<u64> { self.view.hashes.get(field).copied() }

    fn edits_ref(&self) -> Option<&Edits> { self.edits.get(&self.mesh_id) }

    // ------------------------------------------------------------------------------------------ view (computed -> paint -> locks)

    /// One output field's values. `r_override`: `Some(0.0)` is the painted view. `None` blends with the field's realism. Needs a solve for
    /// everything but elevation.
    fn blended(&self, out: &str, r_override: Option<f64>) -> Option<Vec<f64>> {
        let i = out_index(out)?;
        let default = Edits::default();
        let e = self.edits_ref().unwrap_or(&default);
        let r = r_override.unwrap_or_else(|| e.r(out));
        let layer = e.paint.layer(PAINT_LAYERS[i]);
        let mut v = if out == "elevation" {
            paint::blend_elevation(self.elevation.data("elevation").ok()?, layer, r)
        } else {
            let s = self.solved.as_ref()?;
            let c = s.solved.fields.data(out).ok()?;
            match out {
                "temperature" => paint::blend_temperature(c, layer, r),
                "precipitation" => paint::blend_precipitation(c, layer, r),
                _ => paint::blend_biome(c, layer, r),
            }
        };
        paint::apply_locks(&mut v, e.locks.get(out));
        Some(v)
    }

    /// Rebuilds the view; returns the names whose content changed (or that are in `force`). Versions stay monotonic: they bump only on change.
    fn rebuild_view(&mut self, force: &[&str]) -> Vec<String> {
        let mut changed = vec![];
        for out in OUTPUT_FIELDS {
            match self.blended(out, None) {
                Some(v) => {
                    let h = paint::hash_f64(&v);
                    let old = self.view.hashes.get(out).copied();
                    if out == "elevation" && old.is_none() {
                        self.view.versions.insert(out, self.version); // first build: no bump
                    } else if old != Some(h) || force.contains(&out) {
                        self.version += 1;
                        self.view.versions.insert(out, self.version);
                        changed.push(out.to_string());
                    }
                    self.view.hashes.insert(out, h);
                    self.view.data.insert(out, v);
                }
                None => {
                    self.view.data.remove(out);
                    self.view.hashes.remove(out);
                    self.view.versions.remove(out);
                }
            }
        }
        changed
    }

    fn announce(&mut self, names: &[String]) {
        if !names.is_empty() {
            let _ = self.tx.send(Event::FieldChanged { names: names.to_vec(), version: self.version });
        }
    }

    fn edit_input(&self) -> EditInput {
        let mut inp = EditInput::default();
        if let Some(e) = self.edits_ref() {
            let z_edited = e.paint.layer("elevation").map(|l| l.painted_cells() > 0).unwrap_or(false) || e.lock_count("elevation") > 0;
            if z_edited {
                inp.elevation = self.view.data.get("elevation").cloned();
            }
            for f in ["elevation", "temperature"] {
                if let Some(l) = e.locks.get(f).filter(|l| !l.is_empty()) {
                    inp.pins.push((f.to_string(), l.keys().copied().collect(), l.values().copied().collect()));
                }
            }
        }
        inp
    }


    // ------------------------------------------------------------------------------------------ Part 9g: stored versions

    fn step_label(s: &Step) -> String {
        match s {
            Step::Params(g) => g.label.clone(),
            Step::Stroke(t) => t.label.clone(),
            Step::Lock(t) => t.label.clone(),
            Step::Realism(t) => t.label.clone(),
        }
    }

    /// Stores the current solved state (with the output views the clients see) so two versions can be compared later.
    fn record_snapshot(&mut self) {
        let Some(st) = self.solved.as_ref() else { return };
        if self.snaps.iter().any(|s| s.version == st.version) {
            return;
        }
        let what = self.undo.last().map(Self::step_label).unwrap_or_else(|| "initial world".into());
        let snap = Snap {
            version: st.version,
            label: format!("{}: {}", st.quality.name(), what),
            quality: st.quality.name(),
            mesh_id: self.mesh_id.clone(),
            solved: st.solved.clone(),
            views: self.view.data.clone(),
            pins: diffq::pins_of(&self.edit_input()),
            generation: self.edit_generation,
        };
        self.snaps.push(snap);
        while self.snaps.len() > diffq::MAX_SNAPSHOTS {
            self.snaps.remove(0);
        }
    }

    fn snap(&self, version: u64) -> ApiResult<&Snap> { self.snaps.iter().find(|s| s.version == version && s.mesh_id == self.mesh_id).ok_or_else(|| diffq::bad_version(version)) }

    /// (from, to): explicit versions, else the latest stored version against the one before it (against itself when there is only one).
    fn default_pair(&self, from: Option<u64>, to: Option<u64>) -> ApiResult<(u64, u64)> {
        let here: Vec<u64> = self.snaps.iter().filter(|s| s.mesh_id == self.mesh_id).map(|s| s.version).collect();
        let t = match to {
            Some(t) => t,
            None => *here.last().ok_or_else(|| ApiError::new("not_solved", "no stored version yet: solve first"))?,
        };
        let f = match from {
            Some(f) => f,
            None => {
                let i = here.iter().position(|&v| v == t).ok_or_else(|| diffq::bad_version(t))?;
                if i == 0 { t } else { here[i - 1] }
            }
        };
        Ok((f, t))
    }

    fn ensure_bundle(&mut self, from: u64, to: u64) -> ApiResult<()> {
        if self.diff_cache.as_ref().map(|b| (b.from, b.to) == (from, to)).unwrap_or(false) {
            return Ok(());
        }
        let b = diffq::build(self.snap(from)?, self.snap(to)?)?;
        self.diff_cache = Some(b);
        Ok(())
    }

    fn solved_state(&self, solved: Solved, quality: Quality) -> SolvedState {
        let elev_hash = solved.fields.data("elevation").map(paint::hash_f64).unwrap_or(0);
        SolvedState { solved: Arc::new(solved), version: self.version, stale: false, quality, elev_hash }
    }

    // ------------------------------------------------------------------------------------------ commands

    pub fn execute(&mut self, cmd: Command) -> ApiResult<Ack> {
        let name = cmd.name().to_string();
        let mut job = None;
        let mut detail = None;
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
                    self.undo.push(Step::Params(g));
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
                    self.undo.push(Step::Params(g));
                    self.redo.clear();
                    inv
                }
            }
            Command::Undo => {
                let st = self.undo.pop().ok_or_else(|| ApiError::new("nothing_to_undo", "the undo stack is empty"))?;
                self.step(st, true)?
            }
            Command::Redo => {
                let st = self.redo.pop().ok_or_else(|| ApiError::new("nothing_to_redo", "the redo stack is empty"))?;
                self.step(st, false)?
            }
            Command::Solve => {
                self.supersede_refresh()?;
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
                self.supersede_refresh()?;
                if count == 0 || count > variations::MAX_VARIATIONS {
                    return Err(ApiError::new("bad_param", format!("count must be in [1, {}], got {count}", variations::MAX_VARIATIONS)));
                }
                job = Some(self.start_variations(count, coarse));
                vec![]
            }
            Command::PaintStroke { layer, op, cells, weights, value, strength } => {
                let (inv, d) = self.paint_stroke(&layer, StrokeSpec { op, cells, weights, value, strength })?;
                detail = Some(d);
                inv
            }
            Command::SetLock { field, cells, locked } => {
                let (inv, d) = self.set_lock(&field, &cells, locked)?;
                detail = Some(d);
                inv
            }
            Command::SetRealism { field, value } => self.set_realism(&field, value)?,
            Command::Refresh { mode } => {
                let q = match mode.as_str() {
                    "preview" => Quality::Preview,
                    "refined" => Quality::Refined,
                    "full" => Quality::Full,
                    other => return Err(ApiError::new("bad_param", format!("refresh mode must be preview, refined or full, got '{other}'"))),
                };
                self.supersede_refresh()?;
                if q == Quality::Full {
                    job = Some(self.start_solve());
                } else {
                    if self.solved.is_none() {
                        return Err(ApiError::new("not_solved", "a preview or refined refresh needs a solved world; use mode `full` first"));
                    }
                    job = Some(self.start_refresh(q));
                }
                vec![]
            }
        };
        Ok(Ack { api_version: API_VERSION, command: name, invalidated, job, detail })
    }

    /// A running refresh is replaced by a newer request; any other running job refuses it.
    fn supersede_refresh(&mut self) -> ApiResult<()> {
        match self.job.as_ref().map(|j| j.kind) {
            None => Ok(()),
            Some(JobKind::Refresh(_)) => {
                self.cancel_and_join();
                Ok(())
            }
            Some(_) => Err(ApiError::new("job_running", "a job is already running")),
        }
    }

    // ---- Part 9f edit commands

    fn paint_stroke(&mut self, layer: &str, spec: StrokeSpec) -> ApiResult<(Vec<String>, Value)> {
        let li = PAINT_LAYERS.iter().position(|l| *l == layer).ok_or_else(|| ApiError::new("bad_layer", format!("paint layer must be one of {PAINT_LAYERS:?}")))?;
        let out = OUTPUT_FIELDS[li];
        self.cancel_and_join();
        let n = self.domain.n_cells();
        let mesh_id = self.mesh_id.clone();
        let mut e = self.edits.remove(&mesh_id).unwrap_or_default();
        let effect = {
            let locks = e.locks.get(out).cloned().unwrap_or_default();
            let base = if layer == "elevation" { Some(self.elevation.data("elevation")?) } else { None };
            let m = &self.domain.mesh;
            let lay = e.paint.layer_mut(layer, n);
            paint::apply_stroke(layer, lay, base, &m.adj_offsets, &m.adj, &spec, &|c| locks.contains_key(&(c as u32)))
        };
        let effect = match effect {
            Ok(x) => x,
            Err(err) => {
                self.edits.insert(mesh_id, e);
                return Err(err);
            }
        };
        self.edits.insert(mesh_id.clone(), e);
        let detail = json!({ "applied": effect.cells.len(), "skipped_locked": effect.skipped_locked });
        if effect.cells.is_empty() {
            return Ok((vec![], detail));
        }
        let verb = format!("{:?}", spec.op).to_lowercase();
        let step = StrokeStep {
            label: format!("{verb} {layer} ({} cells)", effect.cells.len()),
            mesh_id,
            layer: layer.to_string(),
            cells: effect.cells,
            old: effect.old,
            new: effect.new,
            other: if layer == "elevation" { Some(self.solved.clone()) } else { None },
        };
        self.undo.push(Step::Stroke(step));
        self.redo.clear();
        self.edit_generation += 1;
        let changed = self.rebuild_view(&[]);
        self.announce(&changed);
        Ok((changed, detail))
    }

    fn set_lock(&mut self, field: &str, cells: &[u32], locked: bool) -> ApiResult<(Vec<String>, Value)> {
        if out_index(field).is_none() {
            return Err(ApiError::new("bad_field", format!("lockable fields are {OUTPUT_FIELDS:?}")));
        }
        let n = self.domain.n_cells();
        if let Some(&c) = cells.iter().find(|&&c| c as usize >= n) {
            return Err(ApiError::new("bad_cell", format!("cell {c} out of range (n_cells = {n})")));
        }
        let current = if locked {
            Some(self.view.data.get(field).cloned().ok_or_else(|| ApiError::new("not_solved", format!("locking '{field}' needs a solve first (the lock keeps the current value)")))?)
        } else {
            None
        };
        self.cancel_and_join();
        let mesh_id = self.mesh_id.clone();
        let e = self.edits.entry(mesh_id.clone()).or_default();
        let table = e.locks.entry(field.to_string()).or_default();
        let (mut cs, mut before, mut after) = (vec![], vec![], vec![]);
        let mut seen = std::collections::BTreeSet::new();
        for &c in cells {
            if !seen.insert(c) {
                continue;
            }
            let was = table.get(&c).copied();
            let now = if locked { was.or_else(|| current.as_ref().map(|v| v[c as usize])) } else { None };
            if was != now {
                cs.push(c);
                before.push(was);
                after.push(now);
                match now {
                    Some(v) => table.insert(c, v),
                    None => table.remove(&c),
                };
            }
        }
        if table.is_empty() {
            e.locks.remove(field);
        }
        let detail = json!({ "changed": cs.len(), "locked_total": e.lock_count(field) });
        if cs.is_empty() {
            return Ok((vec![], detail));
        }
        let label = format!("{} {field} ({} cells)", if locked { "lock" } else { "unlock" }, cs.len());
        self.undo.push(Step::Lock(LockStep { label, mesh_id, field: field.to_string(), cells: cs, before, after }));
        self.redo.clear();
        self.edit_generation += 1;
        let changed = self.rebuild_view(&[]);
        self.announce(&changed);
        Ok((changed, detail))
    }

    fn set_realism(&mut self, field: &str, value: f64) -> ApiResult<Vec<String>> {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(ApiError::new("bad_param", format!("realism must be in [0, 100], got {value}")));
        }
        let targets: Vec<String> = if field == "all" {
            OUTPUT_FIELDS.iter().map(|s| s.to_string()).collect()
        } else if out_index(field).is_some() {
            vec![field.to_string()]
        } else {
            return Err(ApiError::new("bad_field", format!("realism fields are {OUTPUT_FIELDS:?} or `all`")));
        };
        self.cancel_and_join();
        let mesh_id = self.mesh_id.clone();
        let e = self.edits.entry(mesh_id.clone()).or_default();
        let changes: Vec<(String, f64, f64)> = targets.iter().map(|f| (f.clone(), e.realism[f], value)).filter(|(_, o, v)| o != v).collect();
        if changes.is_empty() {
            return Ok(vec![]);
        }
        for (f, _, v) in &changes {
            e.realism.insert(f.clone(), *v);
        }
        let label = if changes.len() == 1 { format!("realism {}: {} -> {}", changes[0].0, changes[0].1, value) } else { format!("realism all -> {value}") };
        self.undo.push(Step::Realism(RealismStep { label, mesh_id, changes }));
        self.redo.clear();
        self.edit_generation += 1;
        let changed = self.rebuild_view(&[]);
        self.announce(&changed);
        Ok(changed)
    }

    /// Undo (`undo = true`) or redo of one history entry; the entry goes on the opposite stack.
    fn step(&mut self, st: Step, undo: bool) -> ApiResult<Vec<String>> {
        match st {
            Step::Params(g) => match self.apply_group(&g, undo) {
                Ok(inv) => {
                    if undo { self.redo.push(Step::Params(g)) } else { self.undo.push(Step::Params(g)) }
                    Ok(inv)
                }
                Err(e) => {
                    // put it back where it came from
                    if undo { self.undo.push(Step::Params(g)) } else { self.redo.push(Step::Params(g)) }
                    Err(e)
                }
            },
            Step::Stroke(mut t) => {
                debug_assert_eq!(t.mesh_id, self.mesh_id, "history steps are undone in stack order, so the mesh matches");
                self.cancel_and_join();
                let n = self.domain.n_cells();
                let mesh = self.mesh_id.clone();
                let e = self.edits.entry(mesh).or_default();
                paint::restore(e.paint.layer_mut(&t.layer, n), &t.cells, if undo { &t.old } else { &t.new });
                let mut force: Vec<&str> = vec![];
                if let Some(other) = t.other.as_mut() {
                    std::mem::swap(&mut self.solved, other);
                    if let Some(s) = &mut self.solved {
                        // the restored state keeps its content; its version moves forward so client caches refetch it
                        self.version += 1;
                        s.version = self.version;
                        force = vec!["temperature", "precipitation", "biome"];
                    }
                }
                self.edit_generation += 1;
                let mut changed = self.rebuild_view(&force);
                if !force.is_empty() {
                    self.record_snapshot();
                    if let Some(s) = &self.solved {
                        for n in s.solved.fields.names() {
                            if !changed.contains(&n) {
                                changed.push(n);
                            }
                        }
                    }
                }
                self.announce(&changed);
                if undo { self.redo.push(Step::Stroke(t)) } else { self.undo.push(Step::Stroke(t)) }
                Ok(changed)
            }
            Step::Lock(t) => {
                debug_assert_eq!(t.mesh_id, self.mesh_id);
                self.cancel_and_join();
                let e = self.edits.entry(self.mesh_id.clone()).or_default();
                let table = e.locks.entry(t.field.clone()).or_default();
                for (k, &c) in t.cells.iter().enumerate() {
                    match if undo { t.before[k] } else { t.after[k] } {
                        Some(v) => table.insert(c, v),
                        None => table.remove(&c),
                    };
                }
                if table.is_empty() {
                    e.locks.remove(&t.field);
                }
                self.edit_generation += 1;
                let changed = self.rebuild_view(&[]);
                self.announce(&changed);
                if undo { self.redo.push(Step::Lock(t)) } else { self.undo.push(Step::Lock(t)) }
                Ok(changed)
            }
            Step::Realism(t) => {
                debug_assert_eq!(t.mesh_id, self.mesh_id);
                self.cancel_and_join();
                let e = self.edits.entry(self.mesh_id.clone()).or_default();
                for (f, o, v) in &t.changes {
                    e.realism.insert(f.clone(), if undo { *o } else { *v });
                }
                self.edit_generation += 1;
                let changed = self.rebuild_view(&[]);
                self.announce(&changed);
                if undo { self.redo.push(Step::Realism(t)) } else { self.undo.push(Step::Realism(t)) }
                Ok(changed)
            }
        }
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
        let mut mesh_event = None;
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
                mesh_event = Some(Event::WorldLoaded { mesh_id: new_mesh, n_cells: self.domain.n_cells() });
            }
            // the elevation view is rebuilt from scratch: its version is this step's version (no extra bump)
            self.view.hashes.remove("elevation");
        }
        if let Some(s) = &mut self.solved {
            s.stale = true;
        }
        self.rebuild_view(&[]);
        if let Some(ev) = mesh_event {
            let _ = self.tx.send(ev);
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
        let (c2, tx, p, delay, edit) = (cancel.clone(), self.tx.clone(), self.params.clone(), self.step_delay, self.edit_input());
        let handle = std::thread::spawn(move || {
            let mut last = -1.0;
            JobOutput::Solve(world::solve_with(&p, &edit, &c2, &mut |fr, stage| {
                if fr - last >= 0.0 {
                    last = fr;
                    let _ = tx.send(Event::JobProgress { job: id, fraction: fr, stage: stage.to_string() });
                }
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }))
        });
        self.job = Some(Job { id, kind: JobKind::Solve, cancel, handle });
        id
    }

    fn start_refresh(&mut self, q: Quality) -> u64 {
        let id = self.next_job;
        self.next_job += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let prev = self.solved.as_ref().expect("checked by the caller").solved.clone();
        let (c2, tx, edit, delay) = (cancel.clone(), self.tx.clone(), self.edit_input(), self.step_delay);
        let handle = std::thread::spawn(move || {
            let r = world::refresh(&prev, &edit, q, &c2, &mut |fr, stage| {
                let _ = tx.send(Event::JobProgress { job: id, fraction: fr, stage: stage.to_string() });
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            });
            JobOutput::Refresh(r, q)
        });
        self.job = Some(Job { id, kind: JobKind::Refresh(q), cancel, handle });
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
        self.job = Some(Job { id, kind: JobKind::Variations, cancel, handle });
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

    fn install(&mut self, solved: Solved, quality: Quality) {
        self.version += 1;
        let names = solved.fields.names();
        let st = self.solved_state(solved, quality);
        self.solved = Some(st);
        let mut all = names;
        for n in self.rebuild_view(&["temperature", "precipitation", "biome"]) {
            if !all.contains(&n) {
                all.push(n);
            }
        }
        self.record_snapshot();
        let _ = self.tx.send(Event::FieldChanged { names: all, version: self.version });
    }

    fn finish(&mut self, id: u64, res: std::thread::Result<JobOutput>) -> String {
        let status = match res {
            Ok(JobOutput::Solve(Ok(solved))) => {
                self.install(solved, Quality::Full);
                self.last_refresh = None;
                "completed"
            }
            Ok(JobOutput::Refresh(Ok((solved, stats)), q)) => {
                self.install(solved, q);
                self.last_refresh = Some((q, stats));
                "completed"
            }
            Ok(JobOutput::Variations(Ok(batch))) => {
                // parameters cannot have changed while the job ran (a change cancels and joins it first)
                self.variations = Some(VariationSet { mesh_id: self.mesh_id.clone(), batch });
                "completed"
            }
            Ok(JobOutput::Solve(Err(SolveError::Cancelled))) | Ok(JobOutput::Variations(Err(SolveError::Cancelled))) | Ok(JobOutput::Refresh(Err(SolveError::Cancelled), _)) => "cancelled",
            Ok(JobOutput::Solve(Err(SolveError::Failed(m)))) | Ok(JobOutput::Variations(Err(SolveError::Failed(m)))) | Ok(JobOutput::Refresh(Err(SolveError::Failed(m)), _)) => {
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

    /// Convenience for headless callers: refresh in `mode` and wait. Returns the job status.
    pub fn refresh_blocking(&mut self, mode: &str) -> ApiResult<String> {
        self.execute(Command::Refresh { mode: mode.into() })?;
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
            Query::GetRivers => self.rivers_query(),
            Query::GetOutline { field, threshold } => self.outline_query(&field, threshold),
            Query::GetFieldView { name, layer, view } => self.field_view_query(&name, layer, &view),
            Query::GetPaint { layer } => self.paint_query(&layer),
            Query::GetLocks => Ok(self.locks_query()),
            Query::GetVersions => Ok(QueryResult { json: diffq::versions_json(&self.snaps, &self.mesh_id, self.solved.as_ref().map(|s| s.version)), binary: vec![] }),
            Query::GetDiff { field, from_version, to_version, min_rel } => {
                self.ensure_bundle(from_version, to_version)?;
                let b = self.diff_cache.as_ref().unwrap();
                diffq::diff_query(b, &self.domain, &field, &self.mesh_id, min_rel)
            }
            Query::GetDiffSummary { from_version, to_version, min_rel } => {
                self.ensure_bundle(from_version, to_version)?;
                Ok(QueryResult { json: diffq::summary_query(self.diff_cache.as_ref().unwrap(), &self.domain, &self.mesh_id, min_rel), binary: vec![] })
            }
            Query::Explain { cell, field, from_version, to_version, min_rel } => {
                let (f, t) = self.default_pair(from_version, to_version)?;
                self.ensure_bundle(f, t)?;
                let b = self.snap(t)?.clone();
                diffq::explain_query(self.diff_cache.as_ref().unwrap(), &self.domain, &b, &field, cell, min_rel)
            }
        }
    }

    fn world_info(&self) -> Value {
        let hash = |n: &str| self.view.hashes.get(n).map(|h| format!("{h:016x}"));
        let mut fields = vec![json!({ "name": "elevation", "layers": 1, "version": self.view.versions.get("elevation").copied().unwrap_or(self.world_version), "stale": false, "hash": hash("elevation") })];
        let stale = self.is_stale();
        if let Some(s) = &self.solved {
            for n in s.solved.fields.names() {
                if n != "elevation" {
                    let layers = s.solved.fields.layers(&n).unwrap_or(1);
                    match self.view.versions.get(n.as_str()) {
                        Some(v) => fields.push(json!({ "name": n, "layers": layers, "version": v, "stale": stale, "hash": hash(&n) })),
                        None => fields.push(json!({ "name": n, "layers": layers, "version": s.version, "stale": stale })),
                    }
                }
            }
        }
        let default_edits = Edits::default();
        let e = self.edits_ref().unwrap_or(&default_edits);
        let edit = json!({
            "generation": self.edit_generation,
            "realism": e.realism,
            "locks": OUTPUT_FIELDS.iter().map(|f| (f.to_string(), json!(e.lock_count(f)))).collect::<serde_json::Map<String, Value>>(),
            "paint": PAINT_LAYERS.iter().map(|l| (l.to_string(), json!(e.paint.layer(l).map(|x| x.painted_cells()).unwrap_or(0)))).collect::<serde_json::Map<String, Value>>(),
            "quality": self.solved.as_ref().map(|s| s.quality.name()),
            "last_refresh": self.last_refresh.map(|(q, st)| json!({ "mode": q.name(), "seasons_run": st.seasons_run, "ms": st.ms as u64 })),
        });
        let kind = world::shape_name(&self.params);
        json!({
            "api_version": API_VERSION,
            "preset": self.preset,
            "params": self.params,
            "domain": { "kind": kind, "mesh_id": self.mesh_id, "n_cells": self.domain.n_cells(), "total_area_m2": self.domain.mesh.total_area() },
            "solved": self.solved.is_some(),
            "stale": stale,
            "fields": fields,
            "edit": edit,
            "job": self.job.as_ref().map(|j| json!({ "id": j.id, "running": true, "mode": match j.kind { JobKind::Solve => "full", JobKind::Refresh(q) => q.name(), JobKind::Variations => "variations" } })),
            "can_undo": !self.undo.is_empty(),
            "can_redo": !self.redo.is_empty(),
            // Part 9c: `undo` is oldest first; `redo` lists what would be re-applied, next one first.
            "history": { "undo": self.undo.iter().map(|s| step_json(s, &self.preset)).collect::<Vec<_>>(), "redo": self.redo.iter().rev().map(|s| step_json(s, &self.preset)).collect::<Vec<_>>() },
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
        // elevation, temperature, precipitation and biome come from the view (physics + paint + locks); the rest from the last solve
        if let Some(v) = self.view.data.get(name) {
            if layer >= 1 {
                return Err(ApiError::new("bad_layer", format!("field '{name}' has 1 layers")));
            }
            let version = self.view.versions.get(name).copied().unwrap_or(self.world_version);
            let stale = name != "elevation" && self.is_stale();
            return Ok(FieldData { name: name.into(), values: v.clone(), layers: 1, layer, version, stale });
        }
        if name == "elevation" {
            unreachable!("the view always has elevation");
        }
        let s = self.solved.as_ref().ok_or_else(|| ApiError::new("not_solved", format!("field '{name}' needs a solve first")))?;
        let (fs, version, stale) = (&s.solved.fields, s.version, self.is_stale());
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

    /// Part 9e: river polylines (cells, offsets, discharge per vertex). Layout table in the header, like `get_mesh`.
    fn rivers_query(&self) -> ApiResult<QueryResult> {
        let s = self.solved.as_ref().ok_or_else(|| ApiError::new("not_solved", "rivers need a solve first"))?;
        let fs = &s.solved.fields;
        for n in ["flow_dir", "river_mask", "discharge"] {
            if !fs.has(n) {
                return Err(ApiError::new("unknown_field", format!("no field '{n}'")));
            }
        }
        let r = crate::overlays::rivers(fs.data("flow_dir")?, fs.data("river_mask")?, fs.data("discharge")?)?;
        let mut bin = vec![];
        let mut layout = vec![];
        layout.push(json!({ "name": "offsets", "dtype": "u32", "count": r.offsets.len(), "offset": bin.len() }));
        bin.extend(u32_bytes(&r.offsets));
        layout.push(json!({ "name": "cells", "dtype": "u32", "count": r.cells.len(), "offset": bin.len() }));
        bin.extend(u32_bytes(&r.cells));
        layout.push(json!({ "name": "discharge", "dtype": "f32", "count": r.values.len(), "offset": bin.len() }));
        bin.extend(r.values.iter().flat_map(|x| x.to_le_bytes()));
        let json = json!({
            "mesh_id": self.mesh_id, "kind": "rivers", "version": s.version, "stale": s.stale, "n_lines": r.n_lines(), "n_points": r.cells.len(),
            "endianness": "little", "layout": layout, "bytes": bin.len(),
        });
        Ok(QueryResult { json, binary: bin })
    }

    /// Part 9e: outline of the cells where `field > threshold` (layer 0). Works for any field, including `elevation` (before a solve).
    fn outline_query(&self, field: &str, threshold: f64) -> ApiResult<QueryResult> {
        if !threshold.is_finite() {
            return Err(ApiError::new("bad_param", "threshold must be finite"));
        }
        let f = self.field(field, 0)?;
        let m = &self.domain.mesh;
        let pairs = crate::overlays::outline(&m.adj_offsets, &m.adj, &f.values, threshold);
        let bin = u32_bytes(&pairs);
        let json = json!({
            "mesh_id": self.mesh_id, "kind": "outline", "field": field, "threshold": threshold, "version": f.version, "stale": f.stale,
            "n_edges": pairs.len() / 2, "endianness": "little",
            "layout": [{ "name": "pairs", "dtype": "u32", "count": pairs.len(), "offset": 0 }], "bytes": bin.len(),
        });
        Ok(QueryResult { json, binary: bin })
    }

    fn meta_for(&self, name: &str, values: &[f64]) -> Value {
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
        meta["layer"] = json!(0);
        meta["layers"] = json!(1);
        meta["min"] = json!(lo);
        meta["max"] = json!(hi);
        meta["dtype"] = json!("f32");
        meta["count"] = json!(values.len());
        meta
    }

    /// Part 9f: `blended` (default view), `computed` (physics only, no paint, no locks) or `painted` (realism 0, locks applied).
    fn field_view_query(&self, name: &str, layer: usize, view: &str) -> ApiResult<QueryResult> {
        if view == "blended" {
            return self.field_query(name, layer);
        }
        if out_index(name).is_none() {
            return Err(ApiError::new("bad_field", format!("views exist for {OUTPUT_FIELDS:?}")));
        }
        if layer != 0 {
            return Err(ApiError::new("bad_layer", format!("field '{name}' has 1 layers")));
        }
        let values = match view {
            "computed" => {
                if name == "elevation" {
                    self.elevation.data("elevation")?.to_vec()
                } else {
                    let s = self.solved.as_ref().ok_or_else(|| ApiError::new("not_solved", format!("field '{name}' needs a solve first")))?;
                    s.solved.fields.data(name)?.to_vec()
                }
            }
            "painted" => self.blended(name, Some(0.0)).ok_or_else(|| ApiError::new("not_solved", format!("field '{name}' needs a solve first")))?,
            other => return Err(ApiError::new("bad_param", format!("view must be blended, computed or painted, got '{other}'"))),
        };
        let mut meta = self.meta_for(name, &values);
        meta["view"] = json!(view);
        meta["version"] = json!(self.view.versions.get(name).copied().unwrap_or(self.world_version));
        meta["stale"] = json!(name != "elevation" && self.is_stale());
        Ok(QueryResult { json: meta, binary: f32_bytes(&values) })
    }

    /// Part 9f: a paint layer as `value` then `mask` (f32 each). A layer nobody painted is all zeros.
    fn paint_query(&self, layer: &str) -> ApiResult<QueryResult> {
        if !PAINT_LAYERS.contains(&layer) {
            return Err(ApiError::new("bad_layer", format!("paint layer must be one of {PAINT_LAYERS:?}")));
        }
        let n = self.domain.n_cells();
        let l = self.edits_ref().and_then(|e| e.paint.layer(layer)).cloned().unwrap_or_else(|| paint::Layer::new(n));
        let mut bin = f32_bytes(&l.value);
        let off = bin.len();
        bin.extend(f32_bytes(&l.mask));
        let json = json!({
            "mesh_id": self.mesh_id, "kind": "paint", "layer": layer, "output": paint::output_of(layer), "generation": self.edit_generation,
            "painted_cells": l.painted_cells(), "endianness": "little",
            "layout": [{ "name": "value", "dtype": "f32", "count": n, "offset": 0 }, { "name": "mask", "dtype": "f32", "count": n, "offset": off }], "bytes": bin.len(),
        });
        Ok(QueryResult { json, binary: bin })
    }

    /// Part 9f: locked cells per output field (u32 arrays, ascending).
    fn locks_query(&self) -> QueryResult {
        let default_edits = Edits::default();
        let e = self.edits_ref().unwrap_or(&default_edits);
        let mut bin = vec![];
        let mut layout = vec![];
        for f in OUTPUT_FIELDS {
            let cells: Vec<u32> = e.locks.get(f).map(|l| l.keys().copied().collect()).unwrap_or_default();
            layout.push(json!({ "name": f, "dtype": "u32", "count": cells.len(), "offset": bin.len() }));
            bin.extend(u32_bytes(&cells));
        }
        let json = json!({ "mesh_id": self.mesh_id, "kind": "locks", "generation": self.edit_generation, "endianness": "little", "layout": layout, "bytes": bin.len() });
        QueryResult { json, binary: bin }
    }

    fn cell(&self, cell: usize) -> ApiResult<QueryResult> {
        let n = self.domain.n_cells();
        if cell >= n {
            return Err(ApiError::new("bad_cell", format!("cell {cell} out of range (n_cells = {n})")));
        }
        let c = self.domain.mesh.centroid[cell];
        let up = self.domain.mesh.up[cell];
        let mut values = serde_json::Map::new();
        values.insert("elevation".into(), json!(self.view.data["elevation"][cell]));
        let mut biome_name = Value::Null;
        if let Some(s) = &self.solved {
            for name in s.solved.fields.names() {
                if name == "elevation" {
                    continue;
                }
                if let Some(v) = self.view.data.get(name.as_str()) {
                    values.insert(name.clone(), json!(v[cell]));
                    continue;
                }
                let l = s.solved.fields.layers(&name)?;
                values.insert(name.clone(), json!(s.solved.fields.data(&name)?[cell * l]));
            }
            if let Some(b) = self.view.data.get("biome") {
                biome_name = json!(s.solved.table.name(b[cell] as usize));
            }
        }
        let mut j = json!({
            "cell": cell, "centroid": c, "area_m2": self.domain.mesh.area[cell], "values": values, "biome_name": biome_name,
            "neighbors": self.domain.neighbors(cell), "stale": self.is_stale(),
        });
        // Part 9e: the twelve seasonal values of every seasonal twin (temperature, precipitation, soil moisture) at this cell
        if let Some(sol) = &self.solved {
            let mut seasonal = serde_json::Map::new();
            for (base, twin) in crate::schema::season_variants() {
                if let (Ok(l), Ok(d)) = (sol.solved.fields.layers(twin), sol.solved.fields.data(twin)) {
                    seasonal.insert(base.to_string(), json!((0..l).map(|k| d[cell * l + k]).collect::<Vec<f64>>()));
                }
            }
            j["seasonal"] = Value::Object(seasonal);
        }
        // Part 9f: which values are physics, painted or locked here
        {
            let default_edits = Edits::default();
            let e = self.edits_ref().unwrap_or(&default_edits);
            let mut st = serde_json::Map::new();
            for (k, f) in OUTPUT_FIELDS.iter().enumerate() {
                let mask = e.paint.layer(PAINT_LAYERS[k]).map(|l| l.mask[cell]).unwrap_or(0.0);
                st.insert(f.to_string(), json!({ "painted": mask, "locked": e.is_locked(f, cell), "realism": e.realism[*f] }));
            }
            j["edit_state"] = Value::Object(st);
        }
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
