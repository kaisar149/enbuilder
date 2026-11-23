//! Throwaway toy pipeline for the Part 6 tests (like `testmods`): small modules whose causal structure is known exactly, so propagation,
//! diff and explainer results can be checked against ground truth.
//!
//! ```text
//! surface --surface_albedo--> albedo --heat--> temperature --humidity--> humidity
//! lake_source, wind --plume--> moisture          (a sweep along +x: carries lake_source downwind)
//! bedrock --hardness--> rock_hardness            (unrelated chain: must never change when the surface is edited)
//! ```
//! `heat` solves  lambda (T - T_eq) - k * laplacian(T) = 0  with  T_eq = 300 - 40 albedo  by Jacobi iteration, so a change decays over
//! about sqrt(k/lambda) cells. It holds LOCKED cells of `temperature` fixed during its own iteration (a strong Dirichlet condition),
//! which is the pattern real modules can follow with `FieldStore::is_locked`.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;

fn spec(name: &str, i: &[&str], o: &[&str]) -> ModuleSpec {
    ModuleSpec { name: name.into(), inputs: i.iter().map(|s| s.to_string()).collect(), outputs: o.iter().map(|s| s.to_string()).collect(), anomaly_hooks: vec![] }
}
fn bad(name: &str) -> Error { Error::InvalidParam(format!("unknown parameter '{name}'")) }

macro_rules! no_params {
    () => {
        fn params(&self) -> &ParamSet { &self.params }
        fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
            if self.params.contains(name) && value.is_finite() {
                self.params.set(name, value);
                Ok(())
            } else {
                Err(bad(name))
            }
        }
    };
}

pub struct SurfaceAlbedo { params: ParamSet }
impl SurfaceAlbedo {
    pub fn new() -> Self { SurfaceAlbedo { params: ParamSet::new().with("base", 0.2).with("gain", 0.5) } }
}
impl Module for SurfaceAlbedo {
    fn spec(&self) -> ModuleSpec { spec("surface_albedo", &["surface"], &["albedo"]) }
    no_params!();
    fn run(&mut self, _d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        let (b, g) = (self.params.get_or("base", 0.2), self.params.get_or("gain", 0.5));
        let a: Vec<f64> = f.data("surface")?.iter().map(|s| b + g * s).collect();
        f.put("albedo", 1, a)
    }
}

pub struct Heat { params: ParamSet }
impl Heat {
    pub fn new() -> Self { Heat { params: ParamSet::new().with("k", 1.0).with("lambda", 0.1).with("iters", 800.0) } }
}
impl Module for Heat {
    fn spec(&self) -> ModuleSpec { spec("heat", &["albedo"], &["temperature"]) }
    no_params!();
    fn run(&mut self, d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        let m = &d.mesh;
        let n = m.n_cells;
        let (k, lam, iters) = (self.params.get_or("k", 1.0), self.params.get_or("lambda", 0.1), self.params.get_or("iters", 800.0) as usize);
        let teq: Vec<f64> = f.data("albedo")?.iter().map(|a| 300.0 - 40.0 * a).collect();
        let mut t: Vec<f64> = if f.has("temperature") { f.data("temperature")?.to_vec() } else { teq.clone() };
        let locked: Vec<bool> = (0..n).map(|i| f.is_locked("temperature", i)).collect();
        let mut next = t.clone();
        for _ in 0..iters {
            for i in 0..n {
                if locked[i] {
                    next[i] = t[i];
                    continue;
                }
                let nb = m.neighbors(i);
                let s: f64 = nb.iter().map(|&j| t[j as usize]).sum();
                next[i] = (lam * teq[i] + k * s) / (lam + k * nb.len() as f64);
            }
            std::mem::swap(&mut t, &mut next);
        }
        f.put("temperature", 1, t)
    }
}

pub struct Humidity { params: ParamSet }
impl Humidity {
    pub fn new() -> Self { Humidity { params: ParamSet::new().with("slope", 0.01) } }
}
impl Module for Humidity {
    fn spec(&self) -> ModuleSpec { spec("humidity", &["temperature"], &["humidity"]) }
    no_params!();
    fn run(&mut self, _d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        let s = self.params.get_or("slope", 0.01);
        let h: Vec<f64> = f.data("temperature")?.iter().map(|t| 0.5 + s * (t - 290.0)).collect();
        f.put("humidity", 1, h)
    }
}

/// Sweeps along +x on a RECTANGULAR plane (cell index `j * nx + i`): `m[i] = decay * m[i-1] + source[i]`. Needs `nx`.
pub struct Plume { params: ParamSet, nx: usize }
impl Plume {
    pub fn new(nx: usize) -> Self { Plume { params: ParamSet::new().with("decay", 0.9), nx } }
}
impl Module for Plume {
    fn spec(&self) -> ModuleSpec { spec("plume", &["lake_source", "wind"], &["moisture"]) }
    no_params!();
    fn run(&mut self, d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        let n = d.n_cells();
        let a = self.params.get_or("decay", 0.9);
        let src = f.data("lake_source")?.to_vec();
        let mut m = vec![0.0; n];
        for row in 0..n / self.nx {
            let mut prev = 0.0;
            for i in 0..self.nx {
                let c = row * self.nx + i;
                prev = a * prev + src[c];
                m[c] = prev;
            }
        }
        f.put("moisture", 1, m)
    }
}

pub struct Hardness { params: ParamSet }
impl Hardness {
    pub fn new() -> Self { Hardness { params: ParamSet::new().with("scale", 2.0) } }
}
impl Module for Hardness {
    fn spec(&self) -> ModuleSpec { spec("hardness", &["bedrock"], &["rock_hardness"]) }
    no_params!();
    fn run(&mut self, _d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        let s = self.params.get_or("scale", 2.0);
        let h: Vec<f64> = f.data("bedrock")?.iter().map(|b| s * b).collect();
        f.put("rock_hardness", 1, h)
    }
}

/// Fails on its first run after `arm` is called (to test rollback). Reads `albedo`, writes `boom`.
pub struct Bomb { params: ParamSet }
impl Bomb {
    pub fn new() -> Self { Bomb { params: ParamSet::new().with("armed", 0.0) } }
}
impl Module for Bomb {
    fn spec(&self) -> ModuleSpec { spec("bomb", &["albedo"], &["boom"]) }
    no_params!();
    fn run(&mut self, _d: &Domain, f: &mut FieldStore, _r: &DirtyRegion) -> Result<()> {
        if self.params.get_or("armed", 0.0) > 0.5 {
            return Err(Error::InvalidParam("bomb went off".into()));
        }
        let n = f.n_cells();
        f.put("boom", 1, vec![0.0; n])
    }
}

/// Scheduler with the whole toy pipeline for a rectangular plane `nx` wide.
pub fn toy_scheduler(nx: usize, with_bomb: bool) -> Result<Scheduler> {
    let mut s = Scheduler::new();
    s.register(Box::new(SurfaceAlbedo::new()))?;
    s.register(Box::new(Heat::new()))?;
    s.register(Box::new(Humidity::new()))?;
    s.register(Box::new(Plume::new(nx)))?;
    s.register(Box::new(Hardness::new()))?;
    if with_bomb {
        s.register(Box::new(Bomb::new()))?;
    }
    Ok(s)
}

/// External inputs of the toy pipeline: desert surface (0), no lake source, a steady +x wind of 5 m/s, a bedrock field.
pub fn toy_fields(n: usize) -> Result<FieldStore> {
    let mut f = FieldStore::new(n);
    f.put("surface", 1, vec![0.0; n])?;
    f.put("lake_source", 1, vec![0.0; n])?;
    let mut w = vec![0.0; 3 * n];
    for c in 0..n {
        w[3 * c] = 5.0;
    }
    f.put("wind", 3, w)?;
    f.put("bedrock", 1, (0..n).map(|c| 1.0 + (c % 7) as f64).collect())?;
    Ok(f)
}
