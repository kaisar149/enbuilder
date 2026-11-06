//! Part 4a-1: shared sea level, ocean mask, wind stress. Pure setup, no solver.
//!
//! Modules and fields
//!   sea_level  : (none)                          -> sea_level (m, per cell, uniform unless an anomaly edits it)
//!   ocean_mask : elevation, sea_level            -> ocean_mask (0/1), ocean_component (id, -1 off-ocean),
//!                                                   coast_distance (m), depth (m, >= 0)
//!   wind_stress: wind, ocean_mask, ice_fraction  -> wind_stress (3 layers, N/m^2)
//!
//! Rules (recorded in ARCHITECTURE.md)
//!   * Below sea level  = `elevation < sea_level` (strict; a cell exactly at sea level is land).
//!   * Ocean vs lake    = a connected component (mesh adjacency) of below-sea-level cells whose total area is
//!                        `>= min_ocean_area_m2` is ocean; any smaller one is a lake (Part 4b). `min_ocean_area_m2 = 0`
//!                        means auto: 1% of the total domain area.
//!   * Coast            = an ocean cell with a non-ocean neighbour (land or lake), or with a wall edge
//!                        (`boundary_len > 0` on a `Wall`/`Closed` domain). Void edges are not coasts (water leaves).
//!   * coast_distance   = multi-source Dijkstra over ocean cells only, edge weight = `edge_dist`; 0 on coast cells,
//!                        0 on non-ocean cells. An ocean component with no coast at all gets `sqrt(total_area)` (finite sentinel).
//!   * Stress           = `rho_air * cd * |u| u * (1 - ice_fraction)` on ocean cells, exactly 0 elsewhere.
use crate::domain::{BoundaryKind, Domain};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::{dot, length, scale, sub};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// Label connected components of `below[i] == true` cells. Returns `(component id per cell or -1, areas per id)`.
/// Ids are assigned in order of the lowest cell index of each component (deterministic).
pub fn label_components(domain: &Domain, below: &[bool]) -> (Vec<i64>, Vec<f64>) {
    let mesh = &domain.mesh;
    let n = mesh.n_cells;
    let mut comp = vec![-1i64; n];
    let mut areas: Vec<f64> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for s in 0..n {
        if !below[s] || comp[s] >= 0 {
            continue;
        }
        let id = areas.len() as i64;
        let mut a = 0.0;
        comp[s] = id;
        stack.push(s);
        while let Some(i) = stack.pop() {
            a += mesh.area[i];
            for &j in mesh.neighbors(i) {
                let j = j as usize;
                if below[j] && comp[j] < 0 {
                    comp[j] = id;
                    stack.push(j);
                }
            }
        }
        areas.push(a);
    }
    (comp, areas)
}

/// Ocean/lake split. Returns `(ocean flag per cell, ocean component id per cell (-1 off-ocean, ids renumbered 0..))`.
/// `min_area_m2 <= 0` means auto (1% of total area).
pub fn ocean_cells(domain: &Domain, elevation: &[f64], sea_level: &[f64], min_area_m2: f64) -> (Vec<bool>, Vec<i64>) {
    let n = domain.n_cells();
    let thr = if min_area_m2 > 0.0 { min_area_m2 } else { 0.01 * domain.mesh.total_area() };
    let below: Vec<bool> = (0..n).map(|i| elevation[i] < sea_level[i]).collect();
    let (comp, areas) = label_components(domain, &below);
    // Renumber the surviving components so ocean ids are dense and ordered by lowest cell index.
    let mut remap = vec![-1i64; areas.len()];
    let mut next = 0i64;
    let mut ocean = vec![false; n];
    let mut id = vec![-1i64; n];
    for i in 0..n {
        let c = comp[i];
        if c < 0 || areas[c as usize] < thr {
            continue;
        }
        if remap[c as usize] < 0 {
            remap[c as usize] = next;
            next += 1;
        }
        ocean[i] = true;
        id[i] = remap[c as usize];
    }
    (ocean, id)
}

/// Graph distance (m) from each ocean cell to the nearest coast cell, through ocean cells only. Non-ocean cells get 0.
pub fn coast_distance(domain: &Domain, ocean: &[bool], component: &[i64]) -> Vec<f64> {
    let mesh = &domain.mesh;
    let n = mesh.n_cells;
    let wall_is_coast = matches!(domain.boundary, BoundaryKind::Wall | BoundaryKind::Closed);
    let mut dist = vec![f64::INFINITY; n];
    let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
    for i in 0..n {
        if !ocean[i] {
            continue;
        }
        let touches_land = mesh.neighbors(i).iter().any(|&j| !ocean[j as usize]);
        let touches_wall = wall_is_coast && mesh.boundary_len[i] > 0.0;
        if touches_land || touches_wall {
            dist[i] = 0.0;
            heap.push(Reverse((0f64.to_bits(), i as u32)));
        }
    }
    while let Some(Reverse((db, iu))) = heap.pop() {
        let i = iu as usize;
        let d = f64::from_bits(db);
        if d > dist[i] {
            continue; // stale entry
        }
        let nbrs = mesh.neighbors(i);
        let ds = mesh.edge_dists(i);
        for q in 0..nbrs.len() {
            let j = nbrs[q] as usize;
            if !ocean[j] {
                continue;
            }
            let nd = d + ds[q];
            if nd < dist[j] {
                dist[j] = nd;
                heap.push(Reverse((nd.to_bits(), j as u32)));
            }
        }
    }
    // Sentinel for coastless components (e.g. an all-ocean sphere), and 0 off the ocean.
    let sentinel = mesh.total_area().sqrt();
    let _ = component;
    for i in 0..n {
        if !ocean[i] {
            dist[i] = 0.0;
        } else if !dist[i].is_finite() {
            dist[i] = sentinel;
        }
    }
    dist
}

/// `rho_air * cd * |u| u * (1 - ice)` for a surface wind vector. `ice` is clamped to 0..1.
pub fn stress_vector(u: [f64; 3], rho_air: f64, cd: f64, ice: f64) -> [f64; 3] {
    let k = rho_air * cd * length(u) * (1.0 - ice.clamp(0.0, 1.0));
    scale(u, k)
}

// ---------------------------------------------------------------------------------------------
// Module: sea_level (the one place sea level is defined)
// ---------------------------------------------------------------------------------------------

const SL_PARAMS: [(&str, f64); 1] = [("sea_level", 0.0)];

/// Writes the shared field `sea_level` (m). Every module that needs sea level should read this field instead of
/// carrying its own `sea_level` parameter. Per-cell so a later "sea level" anomaly can edit it locally.
pub struct SeaLevel { params: ParamSet }

impl Default for SeaLevel {
    fn default() -> Self { Self::new() }
}

impl SeaLevel {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in SL_PARAMS.iter() {
            params.set(k, *v);
        }
        SeaLevel { params }
    }
}

impl Module for SeaLevel {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "sea_level".into(), inputs: vec![], outputs: names(&["sea_level"]), anomaly_hooks: names(&["sea_level"]) }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "sea_level" || !value.is_finite() {
            return Err(Error::InvalidParam(format!("sea_level: bad parameter '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        fields.put("sea_level", 1, vec![self.params.get_or("sea_level", 0.0); domain.n_cells()])
    }
}

// ---------------------------------------------------------------------------------------------
// Module: ocean_mask
// ---------------------------------------------------------------------------------------------

const OM_PARAMS: [(&str, f64); 1] = [("min_ocean_area_m2", 0.0)]; // 0 = auto (1% of total area)

pub struct OceanMask { params: ParamSet }

impl Default for OceanMask {
    fn default() -> Self { Self::new() }
}

impl OceanMask {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in OM_PARAMS.iter() {
            params.set(k, *v);
        }
        OceanMask { params }
    }
}

impl Module for OceanMask {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "ocean_mask".into(),
            inputs: names(&["elevation", "sea_level"]),
            outputs: names(&["ocean_mask", "ocean_component", "coast_distance", "depth"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "min_ocean_area_m2" || !value.is_finite() || value < 0.0 {
            return Err(Error::InvalidParam(format!("ocean_mask: bad parameter '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let z = fields.data("elevation")?.to_vec();
        let sl = fields.data("sea_level")?.to_vec();
        let (ocean, comp) = ocean_cells(domain, &z, &sl, self.params.get_or("min_ocean_area_m2", 0.0));
        let cd = coast_distance(domain, &ocean, &comp);
        let mask: Vec<f64> = ocean.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect();
        let cid: Vec<f64> = comp.iter().map(|&c| c as f64).collect();
        let depth: Vec<f64> = (0..n).map(|i| if ocean[i] { (sl[i] - z[i]).max(0.0) } else { 0.0 }).collect();
        fields.put("ocean_mask", 1, mask)?;
        fields.put("ocean_component", 1, cid)?;
        fields.put("coast_distance", 1, cd)?;
        fields.put("depth", 1, depth)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Module: wind_stress
// ---------------------------------------------------------------------------------------------

const WS_PARAMS: [(&str, f64); 2] = [("rho_air", 1.2), ("cd_stress", 1.3e-3)];

pub struct WindStress { params: ParamSet }

impl Default for WindStress {
    fn default() -> Self { Self::new() }
}

impl WindStress {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in WS_PARAMS.iter() {
            params.set(k, *v);
        }
        WindStress { params }
    }
}

impl Module for WindStress {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "wind_stress".into(),
            inputs: names(&["wind", "ocean_mask", "ice_fraction"]),
            outputs: names(&["wind_stress"]),
            anomaly_hooks: names(&["wind_forcing"]),
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !WS_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() || value < 0.0 {
            return Err(Error::InvalidParam(format!("wind_stress: bad parameter '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let (rho, cd) = (self.params.get_or("rho_air", 1.2), self.params.get_or("cd_stress", 1.3e-3));
        let w = fields.data("wind")?;
        let mask = fields.data("ocean_mask")?;
        let ice = fields.data("ice_fraction")?;
        if w.len() != 3 * n {
            return Err(Error::InvalidParam("wind_stress: 'wind' must have 3 layers".into()));
        }
        let mut out = vec![0.0; 3 * n];
        for i in 0..n {
            if mask[i] < 0.5 {
                continue;
            }
            let u = [w[3 * i], w[3 * i + 1], w[3 * i + 2]];
            // Keep the stress tangent: wind is tangent already, this guards against rounding drift.
            let up = domain.mesh.up[i];
            let u = sub(u, scale(up, dot(u, up)));
            let t = stress_vector(u, rho, cd, ice[i]);
            out[3 * i..3 * i + 3].copy_from_slice(&t);
        }
        fields.put("wind_stress", 3, out)
    }
}

// ---------------------------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------------------------

/// Registers `sea_level` + `ocean_mask` (needs `elevation`). Works with no atmosphere.
pub fn add_ocean_mask_stage(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(SeaLevel::new()))?;
    s.register(Box::new(OceanMask::new()))
}

/// Registers `sea_level`, `ocean_mask` and `wind_stress` (needs `elevation`, `wind`, `ice_fraction`).
pub fn add_ocean_stage(s: &mut Scheduler) -> Result<()> {
    add_ocean_mask_stage(s)?;
    s.register(Box::new(WindStress::new()))
}
