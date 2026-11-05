use crate::domain::Domain;
use crate::error::Result;
use crate::fields::FieldStore;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamSet(BTreeMap<String, f64>);

impl ParamSet {
    pub fn new() -> Self { Self::default() }
    pub fn with(mut self, k: &str, v: f64) -> Self { self.0.insert(k.to_string(), v); self }
    pub fn set(&mut self, k: &str, v: f64) { self.0.insert(k.to_string(), v); }
    pub fn get(&self, k: &str) -> Option<f64> { self.0.get(k).copied() }
    pub fn get_or(&self, k: &str, default: f64) -> f64 { self.0.get(k).copied().unwrap_or(default) }
    pub fn contains(&self, k: &str) -> bool { self.0.contains_key(k) }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &f64)> { self.0.iter() }
}

/// Static description the scheduler uses to build the dependency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleSpec {
    pub name: String,
    pub inputs: Vec<String>,
    /// Fields this module writes. A field listed as both input and output is
    /// an in-place update and does not create a dependency on itself.
    pub outputs: Vec<String>,
    /// Anomaly hook names this module responds to (used from Part 8 on).
    pub anomaly_hooks: Vec<String>,
}

/// Which cells need recomputing. Modules may ignore it in Part 1 (treat as All)
/// but must accept it so incremental solving can be added without API changes.
#[derive(Debug, Clone, PartialEq)]
pub enum DirtyRegion {
    All,
    /// Sorted, deduplicated cell indices.
    Cells(Vec<u32>),
}

impl DirtyRegion {
    pub fn cells(mut v: Vec<u32>) -> Self {
        v.sort_unstable();
        v.dedup();
        DirtyRegion::Cells(v)
    }
    pub fn is_all(&self) -> bool { matches!(self, DirtyRegion::All) }
    pub fn contains(&self, i: usize) -> bool {
        match self {
            DirtyRegion::All => true,
            DirtyRegion::Cells(v) => v.binary_search(&(i as u32)).is_ok(),
        }
    }
}

pub trait Module: Send {
    fn spec(&self) -> ModuleSpec;
    fn params(&self) -> &ParamSet;
    fn set_param(&mut self, name: &str, value: f64) -> Result<()>;
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, dirty: &DirtyRegion) -> Result<()>;
}
