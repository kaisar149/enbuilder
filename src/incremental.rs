//! Part 6, steps 2 and 5: incremental re-solve with warm start, adaptive widening, and locks/pins.
//!
//! `resolve_edit` applies a user `Edit`, solves the pipeline WARM (the fields already hold the previous solution, so feedback loops start
//! near their fixed point and need few passes) on a scratch fork of the store, then commits the result only inside each field's dirty
//! region. The region starts at the field's influence radius and is widened (x `widen_factor`) until the change on its border is under
//! `border_tol` x the peak change of that field, or the region is the whole mesh.
//!
//! HONEST STATUS: the modules still recompute every cell (they ignore `DirtyRegion`, see ARCHITECTURE "Open decisions"), so the saving
//! today is the warm start plus the exact locality of the commit: everything outside a region is BIT-IDENTICAL to before the edit (no
//! solver noise outside the dirty region). The `DirtyRegion` is passed to the scheduler so modules that learn to honour it speed up
//! without an API change. Because the scratch solve is global, the report also carries the TRUE truncation error (largest change that
//! was left uncommitted outside the region), not just the border estimate.
//!
//! Locks: `FieldStore::lock_cells` pins cells; every module write restores them, so they act as boundary conditions the rest of the
//! pipeline sees. They are a "weak" Dirichlet condition (a module's own inner iteration may use the unpinned value within one call; the
//! next module and the next loop pass see the pinned one). `Edit::pin` pins the edited cells at the edited values.
use crate::dirty::{downstream_fields, plan_dirty, InfluenceSpec, Region};
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::scheduler::{RunReport, Scheduler, Trace, TraceCfg};
use std::collections::BTreeMap;

/// A user edit of one field. `values` has `cells.len() * layers` numbers (cell-major) or exactly `layers` numbers (broadcast).
#[derive(Debug, Clone)]
pub struct Edit {
    pub label: String,
    pub field: String,
    pub cells: Vec<u32>,
    pub values: Vec<f64>,
    /// Lock the cells at the new values (an override the pipeline cannot undo). Unpinned edits are free to be recomputed by the producer.
    pub pin: bool,
}

impl Edit {
    /// Same value in every layer of every cell (single-layer fields: that value).
    pub fn set(label: &str, field: &str, cells: &[u32], value: f64) -> Self {
        Edit { label: label.into(), field: field.into(), cells: cells.to_vec(), values: vec![value], pin: false }
    }
    pub fn pinned(mut self) -> Self { self.pin = true; self }
    pub fn per_cell(label: &str, field: &str, cells: &[u32], values: Vec<f64>) -> Self {
        Edit { label: label.into(), field: field.into(), cells: cells.to_vec(), values, pin: false }
    }
    fn expand(&self, layers: usize) -> Result<Vec<f64>> {
        if self.values.len() == self.cells.len() * layers {
            Ok(self.values.clone())
        } else if self.values.len() == layers {
            Ok((0..self.cells.len()).flat_map(|_| self.values.iter().copied()).collect())
        } else {
            Err(Error::InvalidParam(format!("edit '{}': {} values for {} cells x {} layers", self.label, self.values.len(), self.cells.len(), layers)))
        }
    }
}

#[derive(Debug, Clone)]
pub struct IncrementalConfig {
    pub spec: InfluenceSpec,
    /// Widen while (border change) > `border_tol` x (peak change of the field). Default 0.01.
    pub border_tol: f64,
    /// Changes below `noise_floor_rel` x range(field) count as noise (not a signal): such a field is left untouched. Default 1e-9.
    pub noise_floor_rel: f64,
    /// Solver noise: a feedback loop that watches a field stops at a relative residual `tolerance`, so differences of about
    /// `loop_noise_factor x tolerance x max|field|` are not signal. The border test is `border <= max(border_tol x peak, that)`, so the
    /// region does not widen to the whole mesh chasing convergence noise. 0 = off (pure `border_tol`). Default 1.
    pub loop_noise_factor: f64,
    /// Add cells outside the region whose change exceeds the threshold (see `FieldReport::far_field_added`). Default true. Switch it off only
    /// to study the radius/widening/downwind logic on its own.
    pub far_field_guard: bool,
    pub max_widen: usize,
    pub widen_factor: f64,
    /// Record a `Trace` (propagation front and module contributions). Default true.
    pub trace: bool,
    /// Passed to `TraceCfg::front_rel_tol`.
    pub front_rel_tol: f64,
}

impl IncrementalConfig {
    pub fn new(spec: InfluenceSpec) -> Self {
        IncrementalConfig { spec, border_tol: 0.01, noise_floor_rel: 1e-9, loop_noise_factor: 1.0, far_field_guard: true, max_widen: 12, widen_factor: 1.5, trace: true, front_rel_tol: 1e-3 }
    }
    pub fn validate(&self) -> Result<()> {
        if !(self.border_tol >= 0.0) || !(self.loop_noise_factor >= 0.0) || !(self.widen_factor > 1.0) || !(self.noise_floor_rel >= 0.0) || !(self.front_rel_tol > 0.0) {
            return Err(Error::InvalidParam("incremental config: need border_tol >= 0, widen_factor > 1, front_rel_tol > 0".into()));
        }
        Ok(())
    }
}

/// Per-field outcome.
#[derive(Debug, Clone)]
pub struct FieldReport {
    pub field: String,
    /// Cells in the final dirty region.
    pub region_cells: usize,
    /// Cells whose committed value differs (bitwise) from before.
    pub changed_cells: usize,
    /// Largest |new - old| anywhere (the global solution).
    pub peak_change: f64,
    /// Largest |new - old| on the final region's border (0 when the region is everything).
    pub border_change: f64,
    /// Solver-noise level used by the border test (0 when no feedback loop watches the field or the factor is 0).
    pub noise_abs: f64,
    /// Cells OUTSIDE the radius/widening region whose change still exceeded `max(border_tol x peak, noise_abs)` and were therefore
    /// added to the region: a far-field (global) mode the locality guess missed, e.g. a uniform shift from the global energy balance.
    pub far_field_added: usize,
    /// TRUE truncation error of what stays uncommitted: largest |new - old| outside the final region. It is <= `max(border_tol x peak, noise_abs)`
    /// by construction (cells above that threshold are added to the region).
    pub truncation_error: f64,
    pub widenings: usize,
    /// Region reached the whole mesh.
    pub whole_mesh: bool,
}

#[derive(Debug, Clone)]
pub struct IncrementalReport {
    pub commit: Option<u64>,
    pub run: RunReport,
    pub fields: Vec<FieldReport>,
    pub affected_fields: Vec<String>,
    /// Union of the per-field regions.
    pub region_cells: usize,
    pub n_cells: usize,
    /// Scheduler runs used (1: the scratch solve is global, widening re-uses it).
    pub solves: usize,
    /// Modules honouring `DirtyRegion` (none yet). When this becomes true the solver can re-run per widening.
    pub modules_honor_dirty: bool,
    pub trace: Option<Trace>,
    /// The final per-field regions (the dirty plan after widening), for the diff and explainer.
    pub regions: BTreeMap<String, Region>,
}

impl IncrementalReport {
    pub fn max_truncation_error(&self) -> f64 { self.fields.iter().map(|f| f.truncation_error).fold(0.0, f64::max) }
    pub fn field(&self, name: &str) -> Option<&FieldReport> { self.fields.iter().find(|f| f.field == name) }
    /// Total loop passes over all feedback loops of the solve (warm start shows here).
    pub fn loop_passes(&self) -> usize { self.run.loops.iter().map(|l| l.iterations).sum() }
}

/// Largest relative tolerance among the feedback loops that watch `field` (0 if none does).
fn loop_tolerance(sched: &Scheduler, field: &str) -> f64 {
    let mut tol = 0.0f64;
    for l in sched.feedback_loops() {
        let watches = if l.watch.is_empty() {
            sched.specs().iter().any(|s| l.members.contains(&s.name) && s.outputs.iter().any(|o| o == field))
        } else {
            l.watch.iter().any(|w| w == field)
        };
        if watches {
            tol = tol.max(l.tolerance);
        }
    }
    tol
}

fn delta_per_cell(old: &[f64], new: &[f64], layers: usize) -> Vec<f64> {
    (0..old.len() / layers).map(|c| (0..layers).map(|l| (new[c * layers + l] - old[c * layers + l]).abs()).fold(0.0, f64::max)).collect()
}

/// Apply `edit`, re-solve warm, commit locally. One undo group (the whole edit undoes with `fields.undo()`).
pub fn resolve_edit(sched: &mut Scheduler, domain: &Domain, fields: &mut FieldStore, edit: &Edit, cfg: &IncrementalConfig) -> Result<IncrementalReport> {
    cfg.validate()?;
    let ly = fields.layers(&edit.field)?;
    let vals = edit.expand(ly)?;
    if edit.cells.is_empty() {
        return Err(Error::InvalidParam(format!("edit '{}' has no cells", edit.label)));
    }
    let mut cells = edit.cells.clone();
    // keep the user's cell order for `values`: only dedup for the seed list
    cells.sort_unstable();
    cells.dedup();

    fields.begin_group(&format!("edit: {}", edit.label))?;
    let out = resolve_inner(sched, domain, fields, edit, &vals, &cells, cfg);
    let end = fields.end_group();
    match out {
        Ok(mut rep) => {
            rep.commit = end?;
            Ok(rep)
        }
        Err(e) => {
            if let Ok(Some(_)) = end {
                let _ = fields.undo();
            }
            Err(e)
        }
    }
}

fn resolve_inner(sched: &mut Scheduler, domain: &Domain, fields: &mut FieldStore, edit: &Edit, vals: &[f64], seeds: &[u32], cfg: &IncrementalConfig) -> Result<IncrementalReport> {
    let n = domain.n_cells();
    fields.set_cells(&edit.field, &edit.cells, vals, edit.pin)?;

    // scratch solve, warm
    let mut scratch = fields.fork();
    let affected = downstream_fields(sched.specs(), &[edit.field.as_str()]);
    let plan0 = plan_dirty(domain, fields, sched.specs(), &[edit.field.as_str()], seeds, &cfg.spec, 1.0);
    sched.set_trace(if cfg.trace { Some(TraceCfg { front_rel_tol: cfg.front_rel_tol }) } else { None });
    let run = sched.run(domain, &mut scratch, &plan0.union.to_dirty());
    let trace = sched.take_trace();
    let run = run?;

    // per-field truncation
    let mut reports = Vec::new();
    let mut regions = BTreeMap::new();
    let mut union = Region::empty(n);
    let mut affected_present = Vec::new();
    for f in &affected {
        let (Some(old), Some(new)) = (fields.get(f), scratch.get(f)) else { continue };
        if old.layers != new.layers {
            return Err(Error::InvalidParam(format!("field '{f}' changed layer count during the solve")));
        }
        affected_present.push(f.clone());
        let d = delta_per_cell(&old.data, &new.data, old.layers);
        let peak = d.iter().cloned().fold(0.0, f64::max);
        let (lo, hi) = old.data.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
        let range = if hi > lo { hi - lo } else { hi.abs().max(lo.abs()) };
        let is_edit = *f == edit.field;
        let max_abs = old.data.iter().fold(0.0f64, |a, &v| a.max(v.abs()));
        let noise_abs = cfg.loop_noise_factor * max_abs * loop_tolerance(sched, f);
        let mut scale = 1.0;
        let mut widenings = 0;
        let mut region;
        let mut border_change;
        loop {
            // regions for this field at the current scale (cheap: one Dijkstra per widening)
            let p = plan_dirty(domain, fields, sched.specs(), &[edit.field.as_str()], seeds, &cfg.spec, scale);
            region = p.per_field.get(f).cloned().unwrap_or_else(|| Region::empty(n));
            let border = region.border(domain);
            border_change = border.iter().map(|&c| d[c as usize]).fold(0.0, f64::max);
            let negligible = peak <= cfg.noise_floor_rel * range.max(1e-300);
            if is_edit || negligible || region.is_all() || border_change <= (cfg.border_tol * peak).max(noise_abs) || widenings >= cfg.max_widen {
                break;
            }
            widenings += 1;
            scale *= cfg.widen_factor;
        }
        let negligible = peak <= cfg.noise_floor_rel * range.max(1e-300);
        // far-field guard: the scratch solve is global, so we KNOW every cell's change. Anything above the threshold outside the region
        // is a signal the locality guess missed; it joins the region (this is what keeps "incremental == full within tolerance" true).
        let thr = (cfg.border_tol * peak).max(noise_abs);
        let mut far_added = 0usize;
        if cfg.far_field_guard && !negligible && !is_edit {
            let far: Vec<usize> = (0..n).filter(|&c| !region.dist[c].is_finite() && d[c] > thr).collect();
            if !far.is_empty() {
                let all = crate::dirty::grow_region(domain, seeds, f64::INFINITY);
                for &c in &far {
                    region.dist[c] = all.dist[c];
                }
                region.cells = (0..n as u32).filter(|&c| region.dist[c as usize].is_finite()).collect();
                far_added = far.len();
            }
        }
        let mask = region.mask();
        let truncation = (0..n).filter(|&c| !mask[c]).map(|c| d[c]).fold(0.0, f64::max);
        let mut changed = 0usize;
        // The edited field is committed over the seeds too: for an input that is the edit itself; for a field some module produces, an
        // UNPINNED edit is only a warm initial condition and the producer's answer wins (pin it to override the physics).
        if !negligible {
            let ly = old.layers;
            let newd = new.data.clone();
            fields.write(f, |dst| {
                for &c in &region.cells {
                    let b = c as usize * ly;
                    dst[b..b + ly].copy_from_slice(&newd[b..b + ly]);
                }
            })?;
            changed = region.cells.iter().filter(|&&c| d[c as usize] > 0.0).count();
        }
        union = union.union(&region);
        reports.push(FieldReport {
            field: f.clone(),
            region_cells: region.len(),
            changed_cells: changed,
            peak_change: peak,
            border_change,
            noise_abs,
            far_field_added: far_added,
            truncation_error: truncation,
            widenings,
            whole_mesh: region.is_all(),
        });
        regions.insert(f.clone(), region);
    }
    // fields the solve created that did not exist before (first run): copy whole
    for name in scratch.names() {
        if !fields.has(&name) {
            let src = scratch.get(&name).unwrap();
            fields.put(&name, src.layers, src.data.clone())?;
        }
    }
    Ok(IncrementalReport {
        commit: None,
        run,
        fields: reports,
        affected_fields: affected_present,
        region_cells: union.len(),
        n_cells: n,
        solves: 1,
        modules_honor_dirty: false,
        trace,
        regions,
    })
}

/// Reference for tests and tools: apply the same edit to `fields` and solve the WHOLE domain (no truncation). Run it on `fields.fork()` to
/// compare with `resolve_edit` on the original.
pub fn solve_full(sched: &mut Scheduler, domain: &Domain, fields: &mut FieldStore, edit: &Edit) -> Result<RunReport> {
    let ly = fields.layers(&edit.field)?;
    let vals = edit.expand(ly)?;
    fields.set_cells(&edit.field, &edit.cells, &vals, edit.pin)?;
    sched.set_trace(None);
    sched.run(domain, fields, &crate::module::DirtyRegion::All)
}

/// Lock helper for painting a pin without changing values.
pub fn pin_cells(fields: &mut FieldStore, field: &str, cells: &[u32]) -> Result<()> { fields.lock_cells(field, cells) }
