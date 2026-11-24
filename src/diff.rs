//! Part 6, step 3: the diff engine.
//!
//! `ChangeSet` = every field value that differs from the state at an older store version (read from the FieldStore's sparse history, so
//! it costs O(changed cells)). `field_diff` turns one field's changes into a `FieldDiff`: signed and absolute change per cell, summary
//! numbers, and the PROPAGATION FRONT used to animate how the effect spread.
//!
//! Front definition: `front[cell]` is the scheduler TICK (one tick = one module execution, see `scheduler::Trace`) at which the cell first
//! deviated from its pre-edit value by more than the trace tolerance; `u32::MAX` = never. Inside a feedback loop this is a real
//! "pass at which it changed" (an ice-albedo front creeping outward shows up as growing ticks); for cells set by a single module call
//! (a diffusion solved inside one `run`) many cells share a tick, so `dist` (surface distance from the edit) breaks ties. The animation
//! order is therefore `(front, dist)`.
use crate::dirty::grow_region;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::scheduler::Trace;
use std::collections::BTreeMap;

/// All changes of one field since an older version. Indices are flat (`cell * layers + layer`).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    pub layers: usize,
    /// (flat index, old, new), ascending by index.
    pub entries: Vec<(u32, f64, f64)>,
    /// The field did not exist at the older version (old values are 0).
    pub created: bool,
}

impl FieldChange {
    /// Per changed cell: (cell, layer, old, new) of the layer with the largest |change| (first layer wins ties), ascending by cell.
    pub fn per_cell(&self) -> Vec<(u32, usize, f64, f64)> {
        let ly = self.layers.max(1) as u32;
        let mut out: Vec<(u32, usize, f64, f64)> = Vec::new();
        for &(i, o, n) in &self.entries {
            let (c, l) = (i / ly, (i % ly) as usize);
            match out.last_mut() {
                Some(last) if last.0 == c => {
                    if (n - o).abs() > (last.3 - last.2).abs() {
                        *last = (c, l, o, n);
                    }
                }
                _ => out.push((c, l, o, n)),
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeSet {
    pub since: u64,
    pub n_cells: usize,
    pub fields: BTreeMap<String, FieldChange>,
}

impl ChangeSet {
    /// Everything that changed after store version `since` (a value from `FieldStore::current_version()`). Fields without changes are
    /// omitted. Fails with `HistoryTruncated` when the history no longer reaches back that far.
    pub fn from_store(fields: &FieldStore, since: u64) -> Result<ChangeSet> {
        let mut fs = BTreeMap::new();
        for name in fields.names() {
            let d = fields.diff(&name, since)?;
            if d.entries.is_empty() {
                continue;
            }
            fs.insert(name.clone(), FieldChange { layers: fields.layers(&name)?, entries: d.entries, created: d.created });
        }
        Ok(ChangeSet { since, n_cells: fields.n_cells(), fields: fs })
    }

    /// The changes made by one commit (the id `IncrementalReport::commit` returns), assuming it is the latest.
    pub fn of_commit(fields: &FieldStore, commit: u64) -> Result<ChangeSet> {
        if commit == 0 || commit > fields.current_version() {
            return Err(Error::InvalidParam(format!("commit {commit} does not exist")));
        }
        Self::from_store(fields, commit - 1)
    }

    pub fn has(&self, field: &str) -> bool { self.fields.contains_key(field) }
    pub fn field_names(&self) -> Vec<String> { self.fields.keys().cloned().collect() }

    /// Dense per-cell signed change: the layer with the largest |change| at that cell (0 where nothing changed).
    pub fn signed(&self, field: &str) -> Vec<f64> {
        let mut out = vec![0.0; self.n_cells];
        if let Some(fc) = self.fields.get(field) {
            for (c, _, o, n) in fc.per_cell() {
                out[c as usize] = n - o;
            }
        }
        out
    }

    /// (old, new) of the layer with the largest |change| at `cell`, or None when the cell did not change.
    pub fn cell_change(&self, field: &str, cell: usize) -> Option<(f64, f64)> {
        let fc = self.fields.get(field)?;
        let ly = fc.layers.max(1);
        let (lo, hi) = ((cell * ly) as u32, ((cell + 1) * ly) as u32);
        let a = fc.entries.partition_point(|e| e.0 < lo);
        let b = fc.entries.partition_point(|e| e.0 < hi);
        fc.entries[a..b].iter().max_by(|x, y| (x.2 - x.1).abs().partial_cmp(&(y.2 - y.1).abs()).unwrap_or(std::cmp::Ordering::Equal)).map(|e| (e.1, e.2))
    }

    /// Largest |change| of the field anywhere (0 if the field is not in the set).
    pub fn peak(&self, field: &str) -> f64 {
        self.fields.get(field).map(|fc| fc.entries.iter().map(|e| (e.2 - e.1).abs()).fold(0.0, f64::max)).unwrap_or(0.0)
    }

    /// Cells that changed in `field`, ascending.
    pub fn cells(&self, field: &str) -> Vec<u32> { self.fields.get(field).map(|fc| fc.per_cell().into_iter().map(|t| t.0).collect()).unwrap_or_default() }
}

#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    /// A cell counts as reached when |change| >= `reach_frac` x peak. Default 0.01.
    pub reach_frac: f64,
}
impl Default for DiffOptions {
    fn default() -> Self { DiffOptions { reach_frac: 0.01 } }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffSummary {
    pub changed_cells: usize,
    pub peak_abs: f64,
    /// Mean |change| over the changed cells.
    pub mean_abs: f64,
    /// Sum of signed changes (cell values, not area weighted).
    pub sum_signed: f64,
    /// Area (mesh units) of the cells that changed.
    pub changed_area: f64,
    /// Area-weighted mean signed change over the WHOLE mesh.
    pub global_mean_shift: f64,
    /// Largest surface distance from the edit of a cell with |change| >= reach_frac x peak (0 if nothing changed).
    pub reach_m: f64,
    /// Smallest and largest finite front tick among changed cells (None when the trace saw none).
    pub front_ticks: Option<(u32, u32)>,
}

/// One field's before/after view with the propagation front, ready for an overlay and for animation.
#[derive(Debug, Clone)]
pub struct FieldDiff {
    pub field: String,
    pub n_cells: usize,
    pub layers: usize,
    /// Per cell: change of the layer with the largest |change| (0 if unchanged).
    pub signed: Vec<f64>,
    pub abs: Vec<f64>,
    /// Per cell, the same layer before and after (unchanged cells: layer 0, equal).
    pub before: Vec<f64>,
    pub after: Vec<f64>,
    /// Scheduler tick of the first deviation (`u32::MAX` = never or no trace).
    pub front: Vec<u32>,
    /// Surface distance from the nearest edit seed (INFINITY if unreachable).
    pub dist: Vec<f64>,
    /// Changed cells in animation order: `(front, dist, cell)` ascending.
    pub order: Vec<u32>,
    pub summary: DiffSummary,
}

/// Build the `FieldDiff` of `field`. `fields` is the store the change set was read from (current values), `seeds` the edited cells (distance
/// origin), `trace` the trace of the solve (`IncrementalReport::trace`) or None (then the order is by distance only).
pub fn field_diff(domain: &Domain, fields: &FieldStore, cs: &ChangeSet, field: &str, seeds: &[u32], trace: Option<&Trace>, opts: &DiffOptions) -> Result<FieldDiff> {
    let n = domain.n_cells();
    if cs.n_cells != n || fields.n_cells() != n {
        return Err(Error::InvalidParam("field_diff: change set, store and domain disagree on the cell count".into()));
    }
    let cur = fields.get(field).ok_or_else(|| Error::UnknownField(field.into()))?;
    let ly = cur.layers;
    let mut signed = vec![0.0; n];
    let mut before: Vec<f64> = (0..n).map(|c| cur.data[c * ly]).collect();
    let mut after = before.clone();
    let mut changed: Vec<u32> = Vec::new();
    if let Some(fc) = cs.fields.get(field) {
        for (c, l, o, nw) in fc.per_cell() {
            let c = c as usize;
            signed[c] = nw - o;
            before[c] = o;
            after[c] = nw;
            let _ = l;
            changed.push(c as u32);
        }
    }
    let abs: Vec<f64> = signed.iter().map(|v| v.abs()).collect();
    let dist = if seeds.is_empty() { vec![f64::INFINITY; n] } else { grow_region(domain, seeds, f64::INFINITY).dist };
    let front: Vec<u32> = match trace.and_then(|t| t.front.get(field)) {
        Some(f) if f.len() == n => f.clone(),
        _ => vec![u32::MAX; n],
    };
    let mut order = changed.clone();
    order.sort_by(|&a, &b| {
        let (a, b) = (a as usize, b as usize);
        front[a].cmp(&front[b]).then(dist[a].partial_cmp(&dist[b]).unwrap_or(std::cmp::Ordering::Equal)).then(a.cmp(&b))
    });
    let peak = abs.iter().cloned().fold(0.0, f64::max);
    let total_area = domain.mesh.total_area().max(1e-300);
    let area_w: f64 = (0..n).map(|c| signed[c] * domain.mesh.area[c]).sum();
    let ticks: Vec<u32> = changed.iter().map(|&c| front[c as usize]).filter(|&t| t != u32::MAX).collect();
    let summary = DiffSummary {
        changed_cells: changed.len(),
        peak_abs: peak,
        mean_abs: if changed.is_empty() { 0.0 } else { changed.iter().map(|&c| abs[c as usize]).sum::<f64>() / changed.len() as f64 },
        sum_signed: signed.iter().sum(),
        changed_area: changed.iter().map(|&c| domain.mesh.area[c as usize]).sum(),
        global_mean_shift: area_w / total_area,
        reach_m: changed.iter().filter(|&&c| abs[c as usize] >= opts.reach_frac * peak && dist[c as usize].is_finite()).map(|&c| dist[c as usize]).fold(0.0, f64::max),
        front_ticks: ticks.iter().min().copied().zip(ticks.iter().max().copied()),
    };
    Ok(FieldDiff { field: field.into(), n_cells: n, layers: ly, signed, abs, before, after, front, dist, order, summary })
}

impl FieldDiff {
    /// Animation frame at time `t` in [0, 1]: the first `ceil(t x changed)` cells of `order` show their NEW value, the rest the old one.
    /// Dense, one number per cell. `t = 0` is the state before the edit, `t = 1` the state after.
    pub fn frame(&self, t: f64) -> Vec<f64> {
        let k = ((t.clamp(0.0, 1.0)) * self.order.len() as f64).ceil() as usize;
        let mut out = self.before.clone();
        for &c in &self.order[..k.min(self.order.len())] {
            out[c as usize] = self.after[c as usize];
        }
        out
    }
    /// `n >= 2` frames from before to after.
    pub fn frames(&self, n: usize) -> Vec<Vec<f64>> { (0..n.max(2)).map(|i| self.frame(i as f64 / (n.max(2) - 1) as f64)).collect() }
    /// Reveal time in [0, 1] of a changed cell (its rank in `order`), or None when it did not change.
    pub fn reveal_time(&self, cell: usize) -> Option<f64> {
        if self.signed[cell] == 0.0 {
            return None;
        }
        let r = self.order.iter().position(|&c| c as usize == cell)?;
        Some(if self.order.len() <= 1 { 0.0 } else { r as f64 / (self.order.len() - 1) as f64 })
    }
}
