//! Dependency-DAG scheduler with declared feedback loops.
//!
//! * Edge P -> C exists when C lists as input a field that P outputs.
//! * Two modules may not output the same field.
//! * Modules inside a declared `FeedbackLoop` are collapsed into one node and
//!   iterated; any cycle not covered by a declaration is rejected (`Error::Cycle`).
//! * A whole `run` is one undo group on the FieldStore; if it fails, its
//!   changes are rolled back.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct FeedbackLoop {
    pub name: String,
    /// Module names, executed in this order each pass.
    pub members: Vec<String>,
    /// Fields relaxed and tested for convergence. Empty = all member outputs.
    pub watch: Vec<String>,
    /// Weight of the new value per pass, in (0, 1]. 1.0 = no relaxation.
    pub relaxation: f64,
    /// Converged when max over watched fields of
    /// max|change| / max(max|value|, 1e-12) <= tolerance.
    pub tolerance: f64,
    pub max_iters: usize,
    /// Part 4c-3: cells where this field is > 0.5 are still relaxed but do not count in the convergence test (e.g. `ocean_mask`:
    /// river-mouth distributary switching is a discrete process that never settles). `None` = every cell counts.
    pub exclude_where: Option<String>,
    /// Part 4c-3: after pass `decay_after`, the relaxation weight is multiplied by `relax_decay` once per further pass
    /// (`om_k = relaxation * relax_decay^(k - decay_after)`). 1.0 = off (default). A loop whose map has DISCRETE switches (receiver
    /// changes, lakes appearing) chatters at a constant amplitude under fixed relaxation; a shrinking weight forces the iterates to settle.
    pub relax_decay: f64,
    pub decay_after: usize,
}

impl FeedbackLoop {
    pub fn new(name: &str, members: &[&str]) -> Self {
        FeedbackLoop {
            name: name.to_string(),
            members: members.iter().map(|s| s.to_string()).collect(),
            watch: Vec::new(),
            relaxation: 0.5,
            tolerance: 1e-6,
            max_iters: 200,
            exclude_where: None,
            relax_decay: 1.0,
            decay_after: 8,
        }
    }
    pub fn relax_decay(mut self, decay: f64, after: usize) -> Self { self.relax_decay = decay; self.decay_after = after; self }
    pub fn exclude_where(mut self, field: &str) -> Self { self.exclude_where = Some(field.to_string()); self }
    pub fn watch(mut self, fields: &[&str]) -> Self { self.watch = fields.iter().map(|s| s.to_string()).collect(); self }
    pub fn relaxation(mut self, w: f64) -> Self { self.relaxation = w; self }
    pub fn tolerance(mut self, t: f64) -> Self { self.tolerance = t; self }
    pub fn max_iters(mut self, n: usize) -> Self { self.max_iters = n; self }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopReport { pub name: String, pub iterations: usize, pub residual: f64, pub converged: bool }

#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    /// Module names in plan order (loop members in declared order).
    pub order: Vec<String>,
    pub loops: Vec<LoopReport>,
    /// FieldStore commit id for this run, if anything changed.
    pub commit: Option<u64>,
}

/// Part 6: tracing configuration (`Scheduler::set_trace`).
#[derive(Debug, Clone)]
pub struct TraceCfg {
    /// A cell "deviates" from the run-start baseline when any layer differs by more than `front_rel_tol * range(field at run start)`
    /// (range = max - min, or max |x| when the field is constant).
    pub front_rel_tol: f64,
}

/// One module execution that changed one output field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleEvent {
    pub tick: u32,
    pub module: String,
    pub field: String,
    /// Loop iteration (1-based) when the module ran inside a feedback loop, else 0.
    pub loop_iter: u32,
    pub changed_cells: usize,
    pub max_abs: f64,
}

/// What a traced `run` observed. `u32::MAX` in `front` = the cell never deviated from its baseline.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    /// Number of module executions (the clock of `front`). Every `run_module` call is one tick.
    pub ticks: u32,
    /// Per field and cell: the first tick at which the cell deviated from the run-start value by more than the tolerance.
    pub front: BTreeMap<String, Vec<u32>>,
    /// Per (module, field): cells that the module wrote with a changed value while the cell deviated from baseline.
    pub touched: BTreeMap<(String, String), Vec<bool>>,
    pub events: Vec<ModuleEvent>,
    pub abs_tol: BTreeMap<String, f64>,
    baseline: BTreeMap<String, Vec<f64>>,
    layers: BTreeMap<String, usize>,
}

impl Trace {
    pub fn baseline(&self, field: &str) -> Option<&[f64]> { self.baseline.get(field).map(|v| v.as_slice()) }
}

#[derive(Debug, Clone)]
enum Step { Module(usize), Loop { loop_idx: usize, members: Vec<usize> } }

pub struct Scheduler {
    modules: Vec<Box<dyn Module>>,
    specs: Vec<ModuleSpec>,
    loops: Vec<FeedbackLoop>,
    plan: Option<Vec<Step>>,
    external_inputs: BTreeSet<String>,
    trace_cfg: Option<TraceCfg>,
    trace: Option<Trace>,
    cur_iter: u32,
}

impl Default for Scheduler {
    fn default() -> Self { Self::new() }
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler { modules: Vec::new(), specs: Vec::new(), loops: Vec::new(), plan: None, external_inputs: BTreeSet::new(), trace_cfg: None, trace: None, cur_iter: 0 }
    }

    pub fn register(&mut self, module: Box<dyn Module>) -> Result<()> {
        let spec = module.spec();
        if self.specs.iter().any(|s| s.name == spec.name) {
            return Err(Error::DuplicateModule(spec.name));
        }
        self.specs.push(spec);
        self.modules.push(module);
        self.plan = None;
        Ok(())
    }

    pub fn add_feedback_loop(&mut self, l: FeedbackLoop) -> Result<()> {
        if !(l.relax_decay > 0.0 && l.relax_decay <= 1.0) {
            return Err(Error::InvalidParam("relax_decay must be in (0, 1]".into()));
        }
        if !(l.relaxation > 0.0 && l.relaxation <= 1.0) {
            return Err(Error::InvalidParam("relaxation must be in (0, 1]".into()));
        }
        if !(l.tolerance > 0.0) || l.max_iters == 0 || l.members.is_empty() {
            return Err(Error::InvalidParam("loop needs tolerance > 0, max_iters >= 1 and members".into()));
        }
        if self.loops.iter().any(|x| x.name == l.name) {
            return Err(Error::InvalidParam(format!("loop '{}' already declared", l.name)));
        }
        self.loops.push(l);
        self.plan = None;
        Ok(())
    }

    pub fn set_param(&mut self, module: &str, name: &str, value: f64) -> Result<()> {
        let i = self.specs.iter().position(|s| s.name == module).ok_or_else(|| Error::UnknownModule(module.into()))?;
        self.modules[i].set_param(name, value)
    }

    /// Current value of a module parameter (None if the module or parameter does not exist).
    pub fn module_param(&self, module: &str, name: &str) -> Option<f64> {
        let i = self.specs.iter().position(|s| s.name == module)?;
        self.modules[i].params().get(name)
    }

    /// Module specs in registration order (Part 6: dirty-region closure and the explainer read the dependency structure from here).
    pub fn specs(&self) -> &[ModuleSpec] { &self.specs }
    pub fn feedback_loops(&self) -> &[FeedbackLoop] { &self.loops }

    /// Part 6: switch tracing on (`Some`) or off (`None`). While on, every `run` records a `Trace`, retrievable with `take_trace`.
    pub fn set_trace(&mut self, cfg: Option<TraceCfg>) { self.trace_cfg = cfg; self.trace = None; }
    pub fn take_trace(&mut self) -> Option<Trace> { self.trace.take() }

    /// Fields some module reads that no module produces: caller must supply them.
    pub fn external_inputs(&self) -> &BTreeSet<String> { &self.external_inputs }

    pub fn plan_order(&mut self) -> Result<Vec<String>> {
        if self.plan.is_none() {
            self.build()?;
        }
        Ok(self.order_names(self.plan.as_ref().unwrap()))
    }

    fn order_names(&self, plan: &[Step]) -> Vec<String> {
        let mut out = Vec::new();
        for st in plan {
            match st {
                Step::Module(i) => out.push(self.specs[*i].name.clone()),
                Step::Loop { members, .. } => {
                    for &i in members {
                        out.push(self.specs[i].name.clone());
                    }
                }
            }
        }
        out
    }

    pub fn build(&mut self) -> Result<()> {
        let m = self.modules.len();
        let name_idx: BTreeMap<String, usize> = self.specs.iter().enumerate().map(|(i, s)| (s.name.clone(), i)).collect();

        // 1. unique producers
        let mut producers: BTreeMap<String, usize> = BTreeMap::new();
        for (i, s) in self.specs.iter().enumerate() {
            for o in &s.outputs {
                if let Some(&prev) = producers.get(o) {
                    return Err(Error::MultipleProducers {
                        field: o.clone(),
                        a: self.specs[prev].name.clone(),
                        b: s.name.clone(),
                    });
                }
                producers.insert(o.clone(), i);
            }
        }

        // 2. resolve declared loops
        let mut mod_loop: Vec<Option<usize>> = vec![None; m];
        let mut loop_members: Vec<Vec<usize>> = Vec::new();
        for (li, l) in self.loops.iter().enumerate() {
            let mut mem = Vec::new();
            for name in &l.members {
                let &i = name_idx.get(name).ok_or_else(|| Error::UnknownModule(name.clone()))?;
                if mod_loop[i].is_some() {
                    return Err(Error::InvalidParam(format!("module '{name}' appears in more than one feedback loop (or twice)")));
                }
                mod_loop[i] = Some(li);
                mem.push(i);
            }
            loop_members.push(mem);
        }

        // 3. condensed nodes, numbered by first appearance in registration order
        let mut nodes: Vec<Step> = Vec::new();
        let mut loop_node: Vec<Option<usize>> = vec![None; self.loops.len()];
        let mut node_of_mod = vec![0usize; m];
        for i in 0..m {
            match mod_loop[i] {
                Some(li) => {
                    if loop_node[li].is_none() {
                        loop_node[li] = Some(nodes.len());
                        nodes.push(Step::Loop { loop_idx: li, members: loop_members[li].clone() });
                    }
                    node_of_mod[i] = loop_node[li].unwrap();
                }
                None => {
                    node_of_mod[i] = nodes.len();
                    nodes.push(Step::Module(i));
                }
            }
        }

        // 4. edges between nodes; external inputs
        let nn = nodes.len();
        let mut succ: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nn];
        let mut indeg = vec![0usize; nn];
        let mut external = BTreeSet::new();
        for (i, s) in self.specs.iter().enumerate() {
            for f in &s.inputs {
                match producers.get(f) {
                    Some(&p) => {
                        let (a, b) = (node_of_mod[p], node_of_mod[i]);
                        if a != b && succ[a].insert(b) {
                            indeg[b] += 1;
                        }
                    }
                    None => {
                        external.insert(f.clone());
                    }
                }
            }
        }

        // 5. Kahn with deterministic tie-break (lowest node id first)
        let mut ready: BTreeSet<usize> = (0..nn).filter(|&k| indeg[k] == 0).collect();
        let mut order: Vec<usize> = Vec::new();
        while let Some(k) = ready.pop_first() {
            order.push(k);
            for &s in &succ[k] {
                indeg[s] -= 1;
                if indeg[s] == 0 {
                    ready.insert(s);
                }
            }
        }
        if order.len() < nn {
            let done: BTreeSet<usize> = order.iter().copied().collect();
            let mut rem: BTreeSet<usize> = (0..nn).filter(|k| !done.contains(k)).collect();
            // prune nodes that are merely downstream of a cycle
            loop {
                let sinks: Vec<usize> = rem.iter().copied().filter(|&k| !succ[k].iter().any(|s| rem.contains(s))).collect();
                if sinks.is_empty() {
                    break;
                }
                for k in sinks {
                    rem.remove(&k);
                }
            }
            let mut names = Vec::new();
            for k in rem {
                match &nodes[k] {
                    Step::Module(i) => names.push(self.specs[*i].name.clone()),
                    Step::Loop { members, .. } => {
                        for &i in members {
                            names.push(self.specs[i].name.clone());
                        }
                    }
                }
            }
            return Err(Error::Cycle(names));
        }

        self.plan = Some(order.into_iter().map(|k| nodes[k].clone()).collect());
        self.external_inputs = external;
        Ok(())
    }

    fn run_module(&mut self, mi: usize, domain: &Domain, fields: &mut FieldStore, dirty: &DirtyRegion) -> Result<()> {
        for f in &self.specs[mi].inputs {
            if !fields.has(f) {
                return Err(Error::MissingInput { module: self.specs[mi].name.clone(), field: f.clone() });
            }
        }
        let before: Option<Vec<(String, Vec<f64>)>> = if self.trace.is_some() {
            Some(self.specs[mi].outputs.iter().filter_map(|f| fields.get(f).map(|x| (f.clone(), x.data.clone()))).collect())
        } else {
            None
        };
        self.modules[mi].run(domain, fields, dirty)?;
        if let Some(b) = before {
            self.trace_after_module(mi, &b, fields);
        }
        for f in &self.specs[mi].outputs {
            if !fields.has(f) {
                return Err(Error::MissingOutput { module: self.specs[mi].name.clone(), field: f.clone() });
            }
        }
        Ok(())
    }

    fn trace_init(&mut self, fields: &FieldStore) {
        let cfg = match &self.trace_cfg {
            Some(c) => c.clone(),
            None => {
                self.trace = None;
                return;
            }
        };
        let n = fields.n_cells();
        let mut t = Trace::default();
        let outs: BTreeSet<String> = self.specs.iter().flat_map(|s| s.outputs.iter().cloned()).collect();
        for f in outs {
            if let Some(x) = fields.get(&f) {
                let (mut lo, mut hi, mut mx) = (f64::INFINITY, f64::NEG_INFINITY, 0.0f64);
                for &v in &x.data {
                    lo = lo.min(v);
                    hi = hi.max(v);
                    mx = mx.max(v.abs());
                }
                let range = if hi > lo { hi - lo } else { mx };
                t.abs_tol.insert(f.clone(), cfg.front_rel_tol * range.max(1e-300));
                t.baseline.insert(f.clone(), x.data.clone());
                t.layers.insert(f.clone(), x.layers);
                t.front.insert(f.clone(), vec![u32::MAX; n]);
            }
        }
        self.trace = Some(t);
        self.cur_iter = 0;
    }

    /// Record deviations of `names` from the run-start baseline at the current tick.
    fn trace_mark(&mut self, fields: &FieldStore, names: &[String]) {
        let t = match self.trace.as_mut() {
            Some(t) => t,
            None => return,
        };
        let tick = t.ticks;
        for f in names {
            let (Some(base), Some(now), Some(front)) = (t.baseline.get(f), fields.get(f), t.front.get_mut(f)) else { continue };
            let ly = t.layers[f];
            let tol = t.abs_tol[f];
            for c in 0..front.len() {
                if front[c] != u32::MAX {
                    continue;
                }
                if (0..ly).any(|l| (now.data[c * ly + l] - base[c * ly + l]).abs() > tol) {
                    front[c] = tick;
                }
            }
        }
    }

    fn trace_after_module(&mut self, mi: usize, before: &[(String, Vec<f64>)], fields: &FieldStore) {
        let module = self.specs[mi].name.clone();
        let iter = self.cur_iter;
        if let Some(t) = self.trace.as_mut() {
            t.ticks += 1;
            let tick = t.ticks;
            for (f, old) in before {
                let Some(now) = fields.get(f) else { continue };
                let ly = now.layers;
                let (Some(base), tol) = (t.baseline.get(f), t.abs_tol.get(f).copied().unwrap_or(0.0)) else { continue };
                let n = old.len() / ly.max(1);
                let flags = t.touched.entry((module.clone(), f.clone())).or_insert_with(|| vec![false; n]);
                let (mut changed, mut maxd) = (0usize, 0.0f64);
                for c in 0..n {
                    let mut d = 0.0f64;
                    let mut dev = false;
                    for l in 0..ly {
                        d = d.max((now.data[c * ly + l] - old[c * ly + l]).abs());
                        if (now.data[c * ly + l] - base[c * ly + l]).abs() > tol {
                            dev = true;
                        }
                    }
                    if d > 0.0 {
                        changed += 1;
                        maxd = maxd.max(d);
                        if dev {
                            flags[c] = true;
                        }
                    }
                }
                if changed > 0 {
                    t.events.push(ModuleEvent { tick, module: module.clone(), field: f.clone(), loop_iter: iter, changed_cells: changed, max_abs: maxd });
                }
            }
        }
        let names: Vec<String> = before.iter().map(|(f, _)| f.clone()).collect();
        self.trace_mark(fields, &names);
    }

    fn run_loop(
        &mut self,
        li: usize,
        members: &[usize],
        domain: &Domain,
        fields: &mut FieldStore,
        dirty: &DirtyRegion,
    ) -> Result<LoopReport> {
        let cfg = self.loops[li].clone();
        let watch: Vec<String> = if cfg.watch.is_empty() {
            let mut set = BTreeSet::new();
            for &i in members {
                for o in &self.specs[i].outputs {
                    set.insert(o.clone());
                }
            }
            set.into_iter().collect()
        } else {
            cfg.watch.clone()
        };
        let mut residual = f64::INFINITY;
        for iter in 1..=cfg.max_iters {
            self.cur_iter = iter as u32;
            let prev: Vec<Option<Vec<f64>>> = watch.iter().map(|w| fields.get(w).map(|f| f.data.clone())).collect();
            for &mi in members {
                self.run_module(mi, domain, fields, dirty)?;
            }
            let mut res = 0.0f64;
            let mut have_all = true;
            for (wi, w) in watch.iter().enumerate() {
                let new: Vec<f64> = fields.data(w)?.to_vec();
                if new.iter().any(|x| !x.is_finite()) {
                    return Err(Error::NonFinite(w.clone()));
                }
                match &prev[wi] {
                    None => have_all = false,
                    Some(old) => {
                        if old.len() != new.len() {
                            return Err(Error::InvalidParam(format!("watched field '{w}' changed size")));
                        }
                        let om = cfg.relaxation * cfg.relax_decay.powi(iter.saturating_sub(cfg.decay_after) as i32);
                        let mut maxd = 0.0f64;
                        let mut maxv = 0.0f64;
                        let mut relaxed = Vec::with_capacity(new.len());
                        let mask: Option<&[f64]> = match &cfg.exclude_where {
                            Some(m) => Some(fields.data(m)?),
                            None => None,
                        };
                        for (k, (o, nw)) in old.iter().zip(new.iter()).enumerate() {
                            let r = if om >= 1.0 { *nw } else { o + om * (nw - o) };
                            if mask.map(|m| m[k] <= 0.5).unwrap_or(true) {
                                maxd = maxd.max((r - o).abs());
                                maxv = maxv.max(r.abs());
                            }
                            relaxed.push(r);
                        }
                        if om < 1.0 {
                            fields.write(w, |d| d.copy_from_slice(&relaxed))?;
                        }
                        res = res.max(maxd / maxv.max(1e-12));
                    }
                }
            }
            residual = if have_all { res } else { f64::INFINITY };
            self.trace_mark(fields, &watch);
            if have_all && res <= cfg.tolerance {
                self.cur_iter = 0;
                return Ok(LoopReport { name: cfg.name, iterations: iter, residual, converged: true });
            }
        }
        self.cur_iter = 0;
        Ok(LoopReport { name: cfg.name, iterations: cfg.max_iters, residual, converged: false })
    }

    fn run_plan(&mut self, plan: &[Step], domain: &Domain, fields: &mut FieldStore, dirty: &DirtyRegion) -> Result<Vec<LoopReport>> {
        let mut reports = Vec::new();
        for st in plan {
            match st {
                Step::Module(i) => self.run_module(*i, domain, fields, dirty)?,
                Step::Loop { loop_idx, members } => {
                    reports.push(self.run_loop(*loop_idx, members, domain, fields, dirty)?);
                }
            }
        }
        Ok(reports)
    }

    pub fn run(&mut self, domain: &Domain, fields: &mut FieldStore, dirty: &DirtyRegion) -> Result<RunReport> {
        if self.plan.is_none() {
            self.build()?;
        }
        if domain.n_cells() != fields.n_cells() {
            return Err(Error::InvalidParam("FieldStore size does not match Domain".into()));
        }
        let plan = self.plan.clone().unwrap();
        let order = self.order_names(&plan);
        self.trace_init(fields);
        fields.begin_group("scheduler run")?;
        let res = self.run_plan(&plan, domain, fields, dirty);
        let end = fields.end_group();
        match res {
            Ok(loops) => Ok(RunReport { order, loops, commit: end? }),
            Err(e) => {
                if let Ok(Some(_)) = end {
                    let _ = fields.undo(); // roll back the partial run
                }
                Err(e)
            }
        }
    }
}
