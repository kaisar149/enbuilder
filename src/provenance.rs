//! Part 6, step 4: provenance and the "why did this cell change?" explainer.
//!
//! PROVENANCE (`Provenance`): per field and cell, a bitmask over a table of at most 64 named contributors. Three kinds of contributor:
//!   * `module:<name>`  the module that produces the field (static, `from_producers`), plus any module that actually wrote the cell
//!                      while it deviated from its pre-edit value (dynamic, `record_solve`, from the scheduler `Trace`);
//!   * `edit:<label>`   a user edit on that field and cell (a warm initial condition when the field has a producer);
//!   * `pin:<label>`    a PINNED user edit (an override of a module-produced field: the explainer stops there);
//!   * `anomaly:<name>` a magic/anomaly source (Part 8 will call `mark_anomaly`).
//!
//! EXPLAINER (`explain`): walks the module dependency graph BACKWARD from (field, cell). At each step it looks at the producer's inputs,
//! finds the input change that best explains the effect (largest relative change, nearest to the cell; surface distance on the mesh),
//! and continues from there until it reaches a recorded origin (an edit or anomaly). Every link is a real measured change from the
//! `ChangeSet` plus a real dependency from the module specs, but the choice between several candidate causes is a RANKING, not a proof;
//! `Hop::Global` marks a hop that had no nearby cause and fell back to a far change (global coupling such as the energy balance).
use crate::diff::ChangeSet;
use crate::dirty::grow_region;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::incremental::{Edit, IncrementalReport};
use crate::scheduler::Trace;
use crate::module::ModuleSpec;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CONTRIBUTORS: usize = 64;

#[derive(Debug, Clone, Default)]
pub struct Provenance {
    n_cells: usize,
    table: Vec<String>,
    masks: BTreeMap<String, Vec<u64>>,
}

impl Provenance {
    pub fn new(n_cells: usize) -> Self { Provenance { n_cells, table: Vec::new(), masks: BTreeMap::new() } }

    /// Static provenance: every cell of every module output gets `module:<producer>`.
    pub fn from_producers(n_cells: usize, specs: &[ModuleSpec]) -> Result<Self> {
        let mut p = Self::new(n_cells);
        let all: Vec<u32> = (0..n_cells as u32).collect();
        for s in specs {
            for o in &s.outputs {
                p.mark(o, &all, &format!("module:{}", s.name))?;
            }
        }
        Ok(p)
    }

    fn id(&mut self, name: &str) -> Result<usize> {
        if let Some(i) = self.table.iter().position(|t| t == name) {
            return Ok(i);
        }
        if self.table.len() >= MAX_CONTRIBUTORS {
            return Err(Error::InvalidParam(format!("provenance: more than {MAX_CONTRIBUTORS} distinct contributors")));
        }
        self.table.push(name.to_string());
        Ok(self.table.len() - 1)
    }

    /// Add `contributor` (full name, e.g. `edit:ice patch`) to `field` at `cells`.
    pub fn mark(&mut self, field: &str, cells: &[u32], contributor: &str) -> Result<()> {
        let id = self.id(contributor)?;
        let n = self.n_cells;
        let m = self.masks.entry(field.to_string()).or_insert_with(|| vec![0u64; n]);
        for &c in cells {
            if (c as usize) < n {
                m[c as usize] |= 1u64 << id;
            }
        }
        Ok(())
    }

    /// Add `anomaly:<name>` (Part 8 calls this for magic/anomaly sources).
    pub fn mark_anomaly(&mut self, field: &str, cells: &[u32], name: &str) -> Result<()> { self.mark(field, cells, &format!("anomaly:{name}")) }

    /// Record a user edit: `pin:<label>` when pinned, else `edit:<label>`.
    pub fn mark_edit(&mut self, edit: &Edit) -> Result<()> {
        let kind = if edit.pin { "pin" } else { "edit" };
        self.mark(&edit.field, &edit.cells, &format!("{kind}:{}", edit.label))
    }

    /// Dynamic module contributors from a traced solve: `module:<m>` on every cell that module wrote with a changed value while the
    /// cell deviated from its pre-edit value.
    pub fn record_solve(&mut self, trace: &Trace) -> Result<()> {
        for ((module, field), flags) in &trace.touched {
            let cells: Vec<u32> = flags.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i as u32).collect();
            if !cells.is_empty() {
                self.mark(field, &cells, &format!("module:{module}"))?;
            }
        }
        Ok(())
    }

    /// Edit + solve in one call: `mark_edit`, then `record_solve` when the report carries a trace.
    pub fn record_edit(&mut self, edit: &Edit, report: &IncrementalReport) -> Result<()> {
        self.mark_edit(edit)?;
        if let Some(t) = &report.trace {
            self.record_solve(t)?;
        }
        Ok(())
    }

    /// Remove a contributor everywhere (e.g. `edit:ice patch` after the edit was undone). Its table slot stays reserved.
    pub fn forget(&mut self, contributor: &str) {
        if let Some(id) = self.table.iter().position(|t| t == contributor) {
            for m in self.masks.values_mut() {
                for w in m.iter_mut() {
                    *w &= !(1u64 << id);
                }
            }
        }
    }

    pub fn n_cells(&self) -> usize { self.n_cells }
    pub fn table(&self) -> &[String] { &self.table }
    pub fn fields(&self) -> Vec<String> { self.masks.keys().cloned().collect() }

    /// Contributors of (field, cell), in table order.
    pub fn contributors(&self, field: &str, cell: usize) -> Vec<String> {
        let w = self.masks.get(field).and_then(|m| m.get(cell)).copied().unwrap_or(0);
        self.table.iter().enumerate().filter(|(i, _)| w >> i & 1 == 1).map(|(_, t)| t.clone()).collect()
    }
    pub fn has(&self, field: &str, cell: usize, contributor: &str) -> bool { self.contributors(field, cell).iter().any(|c| c == contributor) }
    /// Cells of `field` carrying `contributor`.
    pub fn cells_with(&self, field: &str, contributor: &str) -> Vec<u32> {
        let (Some(id), Some(m)) = (self.table.iter().position(|t| t == contributor), self.masks.get(field)) else { return Vec::new() };
        m.iter().enumerate().filter(|(_, w)| **w >> id & 1 == 1).map(|(i, _)| i as u32).collect()
    }

    /// Binary format `ENPROV01`: magic, n_cells u64, table (count u32, then len u32 + utf8 each), masks (count u32, then name as above, then
    /// n_cells x u64), all little-endian. Round-trips exactly.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"ENPROV01".to_vec();
        b.extend((self.n_cells as u64).to_le_bytes());
        b.extend((self.table.len() as u32).to_le_bytes());
        for t in &self.table {
            b.extend((t.len() as u32).to_le_bytes());
            b.extend(t.as_bytes());
        }
        b.extend((self.masks.len() as u32).to_le_bytes());
        for (name, m) in &self.masks {
            b.extend((name.len() as u32).to_le_bytes());
            b.extend(name.as_bytes());
            for w in m {
                b.extend(w.to_le_bytes());
            }
        }
        b
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Provenance> {
        fn bad() -> Error { Error::InvalidParam("provenance: corrupt or truncated data".into()) }
        struct R<'a>(&'a [u8], usize);
        impl<'a> R<'a> {
            fn take(&mut self, k: usize) -> Result<&'a [u8]> {
                let end = self.1.checked_add(k).filter(|&e| e <= self.0.len()).ok_or_else(bad)?;
                let s = &self.0[self.1..end];
                self.1 = end;
                Ok(s)
            }
            fn u32(&mut self) -> Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
            fn u64(&mut self) -> Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
            fn string(&mut self) -> Result<String> {
                let k = self.u32()? as usize;
                String::from_utf8(self.take(k)?.to_vec()).map_err(|_| bad())
            }
        }
        let mut r = R(bytes, 0);
        if r.take(8)? != b"ENPROV01" {
            return Err(bad());
        }
        let n_cells = r.u64()? as usize;
        let nt = r.u32()? as usize;
        if nt > MAX_CONTRIBUTORS {
            return Err(bad());
        }
        let mut table = Vec::new();
        for _ in 0..nt {
            table.push(r.string()?);
        }
        let nm = r.u32()? as usize;
        let mut masks = BTreeMap::new();
        for _ in 0..nm {
            let name = r.string()?;
            let raw = r.take(n_cells.checked_mul(8).ok_or_else(bad)?)?;
            masks.insert(name, raw.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect());
        }
        if r.1 != bytes.len() {
            return Err(bad());
        }
        Ok(Provenance { n_cells, table, masks })
    }
}

// ------------------------------------------------------------------ explainer

#[derive(Debug, Clone)]
pub struct ExplainOptions {
    /// A cause within this surface distance counts as LOCAL; otherwise the hop is `Hop::Global`. Default: 6 x the mean centre distance
    /// of neighbouring cells (set by `ExplainOptions::for_domain`).
    pub local_radius_m: f64,
    /// Distance scale of the proximity weight `1 / (1 + d / proximity_m)`. Default 3 x the mean centre distance.
    pub proximity_m: f64,
    /// Candidate causes must change by at least `min_rel` x the peak change of their field. Default 0.05.
    pub min_rel: f64,
    pub max_depth: usize,
    /// Alternatives are reported when their score >= `alt_frac` x the best score. Default 0.5.
    pub alt_frac: f64,
}

impl ExplainOptions {
    pub fn for_domain(d: &Domain) -> Self {
        let m = &d.mesh;
        let mean = if m.edge_dist.is_empty() { 1.0 } else { m.edge_dist.iter().sum::<f64>() / m.edge_dist.len() as f64 };
        ExplainOptions { local_radius_m: 6.0 * mean, proximity_m: 3.0 * mean, min_rel: 0.05, max_depth: 12, alt_frac: 0.5 }
    }
}

/// How the NEXT link (the cause) was chosen relative to this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hop {
    /// The cause is within `local_radius_m` of this cell.
    Local,
    /// No cause nearby: the best-ranked change anywhere (global coupling, e.g. the energy balance).
    Global,
    /// This link IS the origin (an edit, pin or anomaly, or an external input with no producer). No next link.
    Origin,
    /// The chain stopped without finding an origin (see `Explanation::stopped`).
    End,
}

#[derive(Debug, Clone)]
pub struct Link {
    pub field: String,
    pub cell: u32,
    pub old: f64,
    pub new: f64,
    /// Module that produced this value from the next link (None for an origin).
    pub module: Option<String>,
    pub hop: Hop,
    /// Surface distance from this cell to the next link's cell (0 for an origin).
    pub dist_to_cause_m: f64,
    pub contributors: Vec<String>,
    /// Other changed inputs of the same module that ranked almost as high as the chosen cause (score >= `alt_frac` x best): the cause is
    /// often several inputs at once, and the ranking cannot tell them apart. (field, cell).
    pub also: Vec<(String, u32)>,
    /// Inputs of the module that ALSO changed near this cell but are already on the chain: the hop closes a feedback loop through them.
    pub via_feedback: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Explanation {
    pub field: String,
    pub cell: u32,
    /// Effect first, deepest cause last. Empty when the cell did not change.
    pub chain: Vec<Link>,
    /// Contributor names of the origin (`edit:...`, `pin:...`, `anomaly:...`) or `external:<field>`.
    pub origin: Vec<String>,
    /// Why the chain ends without an origin, if it does.
    pub stopped: Option<String>,
}

impl Explanation {
    pub fn fields(&self) -> Vec<&str> { self.chain.iter().map(|l| l.field.as_str()).collect() }
    pub fn found_origin(&self) -> bool { self.chain.last().map(|l| l.hop == Hop::Origin).unwrap_or(false) }
    pub fn has_global_hop(&self) -> bool { self.chain.iter().any(|l| l.hop == Hop::Global) }

    /// Human-readable chain, effect first.
    pub fn text(&self) -> String {
        if self.chain.is_empty() {
            return format!("{} at cell {} did not change.", self.field, self.cell);
        }
        let mut s = String::new();
        for (i, l) in self.chain.iter().enumerate() {
            let d = l.new - l.old;
            s += &format!("{}{} @cell {}: {:.4} -> {:.4} ({:+.4})", if i == 0 { "" } else { "\n" }, l.field, l.cell, l.old, l.new, d);
            match (l.hop, &l.module) {
                (Hop::Local, Some(m)) => s += &format!("  <- {m}, input changed {:.1} km away", l.dist_to_cause_m / 1e3),
                (Hop::Global, Some(m)) => s += &format!("  <- {m}, GLOBAL coupling (no nearby cause; best ranked cause {:.1} km away)", l.dist_to_cause_m / 1e3),
                (Hop::Origin, _) => s += &format!("  [origin: {}]", if self.origin.is_empty() { "?".to_string() } else { self.origin.join(", ") }),
                _ => {}
            }
            if !l.also.is_empty() {
                s += &format!("; also: {}", l.also.iter().map(|(f, c)| format!("{f}@{c}")).collect::<Vec<_>>().join(", "));
            }
            if !l.via_feedback.is_empty() {
                s += &format!("; feedback loop through {}", l.via_feedback.join(", "));
            }
        }
        if let Some(r) = &self.stopped {
            s += &format!("\n(chain stops: {r})");
        }
        s
    }
}

fn producer<'a>(specs: &'a [ModuleSpec], field: &str) -> Option<&'a ModuleSpec> { specs.iter().find(|s| s.outputs.iter().any(|o| o == field)) }

/// Contributors of (field, cell) that END the walk: pinned edits, anomalies, and edits on fields no module produces.
fn origin_names(prov: &Provenance, specs: &[ModuleSpec], field: &str, cell: usize) -> Vec<String> {
    let produced = producer(specs, field).is_some();
    prov.contributors(field, cell).into_iter().filter(|c| c.starts_with("pin:") || c.starts_with("anomaly:") || (!produced && c.starts_with("edit:"))).collect()
}

/// "Why did `field` change at `cell`?" Walks the module graph backward (see the module doc for what the ranking does and does not prove).
pub fn explain(domain: &Domain, specs: &[ModuleSpec], cs: &ChangeSet, prov: &Provenance, field: &str, cell: usize, opts: &ExplainOptions) -> Result<Explanation> {
    let n = domain.n_cells();
    if cell >= n {
        return Err(Error::InvalidParam(format!("explain: cell {cell} out of range ({n} cells)")));
    }
    let mut ex = Explanation { field: field.into(), cell: cell as u32, chain: Vec::new(), origin: Vec::new(), stopped: None };
    let Some((mut old, mut new)) = cs.cell_change(field, cell) else { return Ok(ex) };
    let (mut f, mut c) = (field.to_string(), cell);
    let mut visited: BTreeSet<String> = BTreeSet::new();
    for _ in 0..opts.max_depth.max(1) {
        visited.insert(f.clone());
        let contributors = prov.contributors(&f, c);
        let origin = origin_names(prov, specs, &f, c);
        let spec = producer(specs, &f);
        if !origin.is_empty() || spec.is_none() {
            ex.origin = if origin.is_empty() { vec![format!("external:{f}")] } else { origin };
            ex.chain.push(Link { field: f, cell: c as u32, old, new, module: None, hop: Hop::Origin, dist_to_cause_m: 0.0, contributors, also: Vec::new(), via_feedback: Vec::new() });
            return Ok(ex);
        }
        let spec = spec.unwrap();
        // rank the changed inputs of the producer: per input field its best local and best overall candidate
        let dist = grow_region(domain, &[c as u32], f64::INFINITY).dist;
        type Cand = (f64, String, usize, f64, bool); // score, field, cell, dist, carries an origin contributor
        let mut locals: Vec<Cand> = Vec::new();
        let mut anys: Vec<Cand> = Vec::new();
        let mut loop_inputs: Vec<String> = Vec::new();
        for g in spec.inputs.iter().filter(|g| **g != f) {
            let Some(fc) = cs.fields.get(g) else { continue };
            let on_chain = visited.contains(g);
            let rows = fc.per_cell();
            let peak = rows.iter().map(|r| (r.3 - r.2).abs()).fold(0.0, f64::max);
            if peak <= 0.0 {
                continue;
            }
            let (mut bl, mut ba): (Option<Cand>, Option<Cand>) = (None, None);
            for (cc, _, o, nw) in rows {
                let rel = (nw - o).abs() / peak;
                let d = dist[cc as usize];
                if rel < opts.min_rel || !d.is_finite() {
                    continue;
                }
                let score = rel / (1.0 + d / opts.proximity_m);
                let is_origin = !origin_names(prov, specs, g, cc as usize).is_empty();
                let cand: Cand = (score, g.clone(), cc as usize, d, is_origin);
                let better = |b: &Option<Cand>| b.as_ref().map(|b| score > b.0 * (1.0 + 1e-9) || (score >= b.0 * (1.0 - 1e-9) && is_origin && !b.4)).unwrap_or(true);
                if d <= opts.local_radius_m && better(&bl) {
                    bl = Some(cand.clone());
                }
                if better(&ba) {
                    ba = Some(cand);
                }
            }
            if on_chain {
                if bl.is_some() {
                    loop_inputs.push(g.clone());
                }
                continue; // an input already on the chain cannot be its own cause
            }
            locals.extend(bl);
            anys.extend(ba);
        }
        let rank = |v: &mut Vec<Cand>| v.sort_by(|a, b| (b.0 * (1.0 + 1e-9)).partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(b.4.cmp(&a.4)).then(a.1.cmp(&b.1)));
        rank(&mut locals);
        rank(&mut anys);
        let (hop, ranked) = if !locals.is_empty() { (Hop::Local, locals) } else { (Hop::Global, anys) };
        if ranked.is_empty() {
            ex.chain.push(Link { field: f, cell: c as u32, old, new, module: Some(spec.name.clone()), hop: Hop::End, dist_to_cause_m: 0.0, contributors, also: Vec::new(), via_feedback: loop_inputs });
            ex.stopped = Some(format!("no changed input of module '{}' that was not already on the chain (feedback loop or sub-threshold cause)", spec.name));
            return Ok(ex);
        }
        let top = ranked[0].0;
        let also: Vec<(String, u32)> = ranked[1..].iter().filter(|r| r.0 >= opts.alt_frac * top).map(|r| (r.1.clone(), r.2 as u32)).collect();
        let pick = ranked[0].clone();
        ex.chain.push(Link { field: f.clone(), cell: c as u32, old, new, module: Some(spec.name.clone()), hop, dist_to_cause_m: pick.3, contributors, also, via_feedback: loop_inputs });
        let (_, g, cc, _, _) = pick;
        let (o2, n2) = cs.cell_change(&g, cc).expect("candidate came from the change set");
        f = g;
        c = cc;
        old = o2;
        new = n2;
    }
    ex.stopped = Some(format!("chain longer than max_depth = {}", opts.max_depth));
    Ok(ex)
}
