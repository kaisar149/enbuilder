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
        }
    }
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

#[derive(Debug, Clone)]
enum Step { Module(usize), Loop { loop_idx: usize, members: Vec<usize> } }

pub struct Scheduler {
    modules: Vec<Box<dyn Module>>,
    specs: Vec<ModuleSpec>,
    loops: Vec<FeedbackLoop>,
    plan: Option<Vec<Step>>,
    external_inputs: BTreeSet<String>,
}

impl Default for Scheduler {
    fn default() -> Self { Self::new() }
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler { modules: Vec::new(), specs: Vec::new(), loops: Vec::new(), plan: None, external_inputs: BTreeSet::new() }
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
        self.modules[mi].run(domain, fields, dirty)?;
        for f in &self.specs[mi].outputs {
            if !fields.has(f) {
                return Err(Error::MissingOutput { module: self.specs[mi].name.clone(), field: f.clone() });
            }
        }
        Ok(())
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
                        let om = cfg.relaxation;
                        let mut maxd = 0.0f64;
                        let mut maxv = 0.0f64;
                        let mut relaxed = Vec::with_capacity(new.len());
                        for (o, nw) in old.iter().zip(new.iter()) {
                            let r = if om >= 1.0 { *nw } else { o + om * (nw - o) };
                            maxd = maxd.max((r - o).abs());
                            maxv = maxv.max(r.abs());
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
            if have_all && res <= cfg.tolerance {
                return Ok(LoopReport { name: cfg.name, iterations: iter, residual, converged: true });
            }
        }
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
