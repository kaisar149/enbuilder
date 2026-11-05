//! Throwaway modules used to prove scheduling and iteration. Not physics.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

fn no_param(name: &str) -> Error { Error::InvalidParam(format!("unknown parameter '{name}'")) }

/// Writes a constant per-cell value into `heat_source`.
pub struct ConstantSource { params: ParamSet, output: String }

impl ConstantSource {
    pub fn new(output: &str, value: f64) -> Self {
        ConstantSource { params: ParamSet::new().with("value", value), output: output.to_string() }
    }
}

impl Module for ConstantSource {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "constant_source".into(), inputs: vec![], outputs: vec![self.output.clone()], anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "value" { return Err(no_param(name)); }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let v = self.params.get_or("value", 0.0);
        fields.put(&self.output, 1, vec![v; domain.n_cells()])
    }
}

/// Conservative explicit diffusion in flux form:
/// dT_i/dt = (1/A_i) * sum_j k * (len_ij / dist_ij) * (T_j - T_i)  [+ source_i].
/// Wall/void/closed boundaries carry no flux, so sum(T*A) is conserved
/// (when there is no source).
pub struct Diffusion { params: ParamSet, field: String, source: Option<String> }

impl Diffusion {
    pub fn new(field: &str) -> Self {
        Diffusion { params: ParamSet::new().with("diffusivity", 1.0).with("steps", 1.0), field: field.to_string(), source: None }
    }
    /// Add a per-cell source term (units of field per time) read from `source_field`.
    pub fn with_source(mut self, source_field: &str) -> Self { self.source = Some(source_field.to_string()); self }
}

fn stable_dt(mesh: &Mesh, k: f64) -> f64 {
    let mut dt = f64::INFINITY;
    for i in 0..mesh.n_cells {
        let coef: f64 = mesh.edge_lens(i).iter().zip(mesh.edge_dists(i)).map(|(l, d)| k * l / d).sum();
        if coef > 0.0 {
            dt = dt.min(0.4 * mesh.area[i] / coef);
        }
    }
    dt
}

impl Module for Diffusion {
    fn spec(&self) -> ModuleSpec {
        let mut inputs = vec![self.field.clone()];
        if let Some(s) = &self.source { inputs.push(s.clone()); }
        ModuleSpec { name: "diffusion".into(), inputs, outputs: vec![self.field.clone()], anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        match name {
            "diffusivity" if value >= 0.0 => {}
            "steps" if value >= 1.0 => {}
            "diffusivity" | "steps" => return Err(Error::InvalidParam(format!("bad value for '{name}'"))),
            _ => return Err(no_param(name)),
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let k = self.params.get_or("diffusivity", 1.0);
        let steps = self.params.get_or("steps", 1.0) as usize;
        if fields.layers(&self.field)? != 1 {
            return Err(Error::InvalidParam("diffusion test module supports single-layer fields only".into()));
        }
        let src: Option<Vec<f64>> = match &self.source {
            Some(s) => Some(fields.data(s)?.to_vec()),
            None => None,
        };
        let dt = stable_dt(mesh, k);
        if !dt.is_finite() || k == 0.0 {
            return Ok(());
        }
        let n = mesh.n_cells;
        fields.write(&self.field, |t| {
            let mut next = vec![0.0; n];
            for _ in 0..steps {
                for i in 0..n {
                    let nbrs = mesh.neighbors(i);
                    let ls = mesh.edge_lens(i);
                    let ds = mesh.edge_dists(i);
                    let mut flux = 0.0;
                    for q in 0..nbrs.len() {
                        let j = nbrs[q] as usize;
                        flux += k * ls[q] / ds[q] * (t[j] - t[i]);
                    }
                    let mut v = t[i] + dt * flux / mesh.area[i];
                    if let Some(s) = &src {
                        v += dt * s[i];
                    }
                    next[i] = v;
                }
                t.copy_from_slice(&next);
            }
        })
    }
}

/// Thermostat: heat_source = gain * (target - temperature). Closes a feedback
/// loop with `Diffusion::with_source("heat_source")`.
pub struct Thermostat { params: ParamSet, input: String, output: String }

impl Thermostat {
    pub fn new(input: &str, output: &str, target: f64, gain: f64) -> Self {
        Thermostat { params: ParamSet::new().with("target", target).with("gain", gain), input: input.into(), output: output.into() }
    }
}

impl Module for Thermostat {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "thermostat".into(), inputs: vec![self.input.clone()], outputs: vec![self.output.clone()], anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "target" && name != "gain" { return Err(no_param(name)); }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, _domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let (target, gain) = (self.params.get_or("target", 0.0), self.params.get_or("gain", 0.0));
        let src: Vec<f64> = fields.data(&self.input)?.iter().map(|t| gain * (target - t)).collect();
        fields.put(&self.output, 1, src)
    }
}

/// Declares arbitrary inputs/outputs and fills missing outputs with zeros.
/// Used to test graph construction (ordering, cycles, duplicate producers).
pub struct Noop { params: ParamSet, name: String, inputs: Vec<String>, outputs: Vec<String> }

impl Noop {
    pub fn new(name: &str, inputs: &[&str], outputs: &[&str]) -> Self {
        Noop {
            params: ParamSet::new(),
            name: name.into(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Module for Noop {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: self.name.clone(), inputs: self.inputs.clone(), outputs: self.outputs.clone(), anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, _v: f64) -> Result<()> { Err(no_param(name)) }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        for o in &self.outputs {
            if !fields.has(o) {
                fields.put(o, 1, vec![0.0; domain.n_cells()])?;
            }
        }
        Ok(())
    }
}
