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
// Part 4a-2a: ocean subgraph, operator and linear solver (no physics)
// ---------------------------------------------------------------------------------------------
//
// Operator (per ocean cell i, local indices):
//     (L psi)_i = sum_k w_ik (psi_k - psi_i),   w_ik = (edge_len / edge_dist)_k / area_i,
// summed over ALL mesh neighbours k. A land (non-ocean) neighbour has psi = 0 (Dirichlet), so it contributes
// `w_ik * (0 - psi_i)`. Edges that leave the domain (wall/void) carry no term (natural, zero-flux).
//
// Symmetry: with c_ik = edge_len/edge_dist (symmetric) and A = diag(area), L = -A^-1 C. L is therefore symmetric in
// the AREA-WEIGHTED inner product <x,y>_A = sum_i area_i x_i y_i (and in the plain inner product when all areas are
// equal, e.g. the plane), and negative semi-definite there. The solver works on the equivalent Euclidean-SPD system
//     (C + sigma A) psi = A b      <=>      (-L + sigma I) psi = b
// with Jacobi-preconditioned CG, one independent solve per connected ocean component.

/// Iteration controls for the ocean CG. Defaults: relative residual 1e-10, 20000 iterations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CgConfig {
    /// Stop when `||b - K psi|| <= tol * ||b||` (2-norm of the symmetrised system's residual and right-hand side).
    pub tol: f64,
    /// Hard guard: exceeding it returns `Error::NoConvergence` instead of looping.
    pub max_iters: usize,
}

impl Default for CgConfig {
    fn default() -> Self { CgConfig { tol: 1e-10, max_iters: 20_000 } }
}

/// Per-component solve report.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSolve {
    /// Value from the `ocean_component` field.
    pub id: i64,
    pub n_cells: usize,
    pub iterations: usize,
    /// Final `||r|| / ||rhs||` (0 for a zero right-hand side).
    pub rel_residual: f64,
    /// True when the component has no Dirichlet anchor and `sigma == 0` (operator singular): the right-hand side
    /// had its area-weighted mean removed and the solution is normalised to area-weighted mean zero.
    pub singular: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OceanSolution {
    /// Solution per OCEAN-LOCAL cell (same order as `OceanGraph::cells`).
    pub psi: Vec<f64>,
    pub components: Vec<ComponentSolve>,
    pub total_iterations: usize,
}

/// The induced subgraph of `ocean_mask == 1` cells.
#[derive(Debug, Clone)]
pub struct OceanGraph {
    /// Number of ocean cells.
    pub n: usize,
    /// Local index -> global cell index (ascending).
    pub cells: Vec<u32>,
    /// Global cell index -> local index, or -1 off-ocean.
    pub local_of: Vec<i64>,
    /// CSR over local indices, ocean neighbours only (same relative order as `mesh.adj`).
    pub offsets: Vec<u32>,
    pub adj: Vec<u32>,
    /// `edge_len / edge_dist` parallel to `adj` (the Part 1 flux coefficient, not yet divided by area).
    pub coef: Vec<f64>,
    /// Cell area (m^2) per local cell.
    pub area: Vec<f64>,
    /// Sum of `edge_len/edge_dist` over non-ocean neighbours (the Dirichlet psi = 0 diagonal term).
    pub land_coef: Vec<f64>,
    /// At least one land (non-ocean) neighbour. Walls/void edges do NOT count (they are not land).
    pub coast_adjacent: Vec<bool>,
    /// Component id per local cell (from `ocean_component`).
    pub component: Vec<i64>,
    /// Distinct component ids, ascending.
    pub comp_ids: Vec<i64>,
    /// Local cell indices of each component (ascending), parallel to `comp_ids`.
    pub comp_cells: Vec<Vec<u32>>,
    /// Position of each local cell inside its component's list.
    comp_pos: Vec<u32>,
}

impl OceanGraph {
    /// Build from an ocean flag per cell and the `ocean_component` ids (as returned by `ocean_cells`).
    /// Errors if two adjacent ocean cells carry different component ids or an ocean cell has no id.
    pub fn new(domain: &Domain, ocean: &[bool], component: &[i64]) -> Result<Self> {
        let mesh = &domain.mesh;
        let nc = mesh.n_cells;
        if ocean.len() != nc || component.len() != nc {
            return Err(Error::InvalidParam("OceanGraph: ocean/component length does not match the mesh".into()));
        }
        let mut local_of = vec![-1i64; nc];
        let mut cells: Vec<u32> = Vec::new();
        for i in 0..nc {
            if ocean[i] {
                if component[i] < 0 {
                    return Err(Error::InvalidParam(format!("OceanGraph: ocean cell {i} has no component id")));
                }
                local_of[i] = cells.len() as i64;
                cells.push(i as u32);
            }
        }
        let n = cells.len();
        let mut offsets = Vec::with_capacity(n + 1);
        let mut adj: Vec<u32> = Vec::new();
        let mut coef: Vec<f64> = Vec::new();
        let mut area = Vec::with_capacity(n);
        let mut land_coef = Vec::with_capacity(n);
        let mut coast = Vec::with_capacity(n);
        offsets.push(0u32);
        for &g in &cells {
            let g = g as usize;
            let nbrs = mesh.neighbors(g);
            let ls = mesh.edge_lens(g);
            let ds = mesh.edge_dists(g);
            let mut lc = 0.0;
            let mut touches_land = false;
            for q in 0..nbrs.len() {
                let j = nbrs[q] as usize;
                let c = ls[q] / ds[q];
                if ocean[j] {
                    if component[j] != component[g] {
                        return Err(Error::InvalidParam(format!("OceanGraph: adjacent ocean cells {g} and {j} are in different components")));
                    }
                    adj.push(local_of[j] as u32);
                    coef.push(c);
                } else {
                    lc += c;
                    touches_land = true;
                }
            }
            offsets.push(adj.len() as u32);
            area.push(mesh.area[g]);
            land_coef.push(lc);
            coast.push(touches_land);
        }
        let comp_local: Vec<i64> = cells.iter().map(|&g| component[g as usize]).collect();
        let mut comp_ids: Vec<i64> = comp_local.clone();
        comp_ids.sort_unstable();
        comp_ids.dedup();
        let mut comp_cells: Vec<Vec<u32>> = vec![Vec::new(); comp_ids.len()];
        let mut comp_pos = vec![0u32; n];
        for (l, &c) in comp_local.iter().enumerate() {
            let k = comp_ids.binary_search(&c).expect("id present");
            comp_pos[l] = comp_cells[k].len() as u32;
            comp_cells[k].push(l as u32);
        }
        Ok(OceanGraph { n, cells, local_of, offsets, adj, coef, area, land_coef, coast_adjacent: coast, component: comp_local, comp_ids, comp_cells, comp_pos })
    }

    /// Build from the fields written by `ocean_mask` (`ocean_mask`, `ocean_component`).
    pub fn from_fields(domain: &Domain, fields: &FieldStore) -> Result<Self> {
        let m = fields.data("ocean_mask")?;
        let c = fields.data("ocean_component")?;
        let ocean: Vec<bool> = m.iter().map(|&v| v > 0.5).collect();
        let comp: Vec<i64> = c.iter().map(|&v| v.round() as i64).collect();
        Self::new(domain, &ocean, &comp)
    }

    pub fn n_components(&self) -> usize { self.comp_ids.len() }

    /// Local neighbours of local cell `i`.
    pub fn neighbors(&self, i: usize) -> &[u32] { &self.adj[self.offsets[i] as usize..self.offsets[i + 1] as usize] }

    /// Operator weight `w_ik` for the `q`-th ocean neighbour of local cell `i`.
    pub fn weight(&self, i: usize, q: usize) -> f64 { self.coef[self.offsets[i] as usize + q] / self.area[i] }

    /// `L psi` on local vectors (Dirichlet psi = 0 on land neighbours).
    pub fn apply_l(&self, x: &[f64]) -> Vec<f64> {
        assert_eq!(x.len(), self.n, "apply_l: length mismatch");
        let mut out = vec![0.0; self.n];
        for i in 0..self.n {
            let (a, b) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
            let mut s = 0.0;
            for q in a..b {
                s += self.coef[q] * (x[self.adj[q] as usize] - x[i]);
            }
            s -= self.land_coef[i] * x[i];
            out[i] = s / self.area[i];
        }
        out
    }

    /// `(-L + sigma I) x`.
    pub fn apply_shifted(&self, sigma: f64, x: &[f64]) -> Vec<f64> {
        let l = self.apply_l(x);
        (0..self.n).map(|i| sigma * x[i] - l[i]).collect()
    }

    /// Area-weighted inner product `sum_i area_i x_i y_i` (the inner product in which `L` is symmetric).
    pub fn dot_area(&self, x: &[f64], y: &[f64]) -> f64 {
        (0..self.n).map(|i| self.area[i] * x[i] * y[i]).sum()
    }

    /// Scatter a local vector to a global one (0 off-ocean).
    pub fn to_global(&self, local: &[f64]) -> Vec<f64> {
        let mut g = vec![0.0; self.local_of.len()];
        for (l, &c) in self.cells.iter().enumerate() {
            g[c as usize] = local[l];
        }
        g
    }

    /// Gather the ocean cells of a global vector.
    pub fn to_local(&self, global: &[f64]) -> Vec<f64> {
        self.cells.iter().map(|&c| global[c as usize]).collect()
    }

    /// Split a local right-hand side per component, in `comp_ids` order and ascending local index inside each.
    /// An empty ocean gives an empty list; a one-cell component gives a one-element vector.
    pub fn split_by_component(&self, b: &[f64]) -> Vec<Vec<f64>> {
        assert_eq!(b.len(), self.n, "split_by_component: length mismatch");
        self.comp_cells.iter().map(|cells| cells.iter().map(|&l| b[l as usize]).collect()).collect()
    }

    /// Inverse of `split_by_component`.
    pub fn merge_by_component(&self, parts: &[Vec<f64>]) -> Vec<f64> {
        let mut out = vec![0.0; self.n];
        for (cells, p) in self.comp_cells.iter().zip(parts) {
            for (&l, &v) in cells.iter().zip(p) {
                out[l as usize] = v;
            }
        }
        out
    }

    /// Solve `(-L + sigma I) psi = b` (local vectors), one CG per component. `sigma >= 0`.
    /// Returns `Error::NoConvergence` if any component exceeds `cfg.max_iters`.
    pub fn solve(&self, sigma: f64, b: &[f64], cfg: &CgConfig) -> Result<OceanSolution> {
        if !sigma.is_finite() || sigma < 0.0 {
            return Err(Error::InvalidParam("ocean solve: sigma must be finite and >= 0".into()));
        }
        if !(cfg.tol > 0.0) || !cfg.tol.is_finite() {
            return Err(Error::InvalidParam("ocean solve: tol must be > 0".into()));
        }
        if b.len() != self.n {
            return Err(Error::InvalidParam(format!("ocean solve: rhs length {} != {} ocean cells", b.len(), self.n)));
        }
        if b.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("ocean solve rhs".into()));
        }
        let parts = self.split_by_component(b);
        let mut sols: Vec<Vec<f64>> = Vec::with_capacity(parts.len());
        let mut reports = Vec::with_capacity(parts.len());
        let mut total = 0usize;
        for (k, rhs) in parts.iter().enumerate() {
            let (x, rep) = self.solve_component(k, sigma, rhs, cfg)?;
            total += rep.iterations;
            sols.push(x);
            reports.push(rep);
        }
        Ok(OceanSolution { psi: self.merge_by_component(&sols), components: reports, total_iterations: total })
    }

    /// `K x` for the symmetrised system on component `k` (component-local vectors).
    fn k_mul(&self, cells: &[u32], sigma: f64, x: &[f64], out: &mut [f64]) {
        for (p, &l) in cells.iter().enumerate() {
            let i = l as usize;
            let (a, b) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
            let mut s = (self.land_coef[i] + sigma * self.area[i]) * x[p];
            for q in a..b {
                s += self.coef[q] * (x[p] - x[self.comp_pos[self.adj[q] as usize] as usize]);
            }
            out[p] = s;
        }
    }

    fn solve_component(&self, k: usize, sigma: f64, b: &[f64], cfg: &CgConfig) -> Result<(Vec<f64>, ComponentSolve)> {
        let cells = &self.comp_cells[k];
        let m = cells.len();
        let id = self.comp_ids[k];
        let singular = sigma == 0.0 && !cells.iter().any(|&l| self.coast_adjacent[l as usize]);
        let area: Vec<f64> = cells.iter().map(|&l| self.area[l as usize]).collect();
        // Symmetrised right-hand side A b (mean removed first when the operator is singular).
        let mut rhs: Vec<f64> = (0..m).map(|p| area[p] * b[p]).collect();
        if singular {
            let sa: f64 = area.iter().sum();
            let mean_b = rhs.iter().sum::<f64>() / sa; // area-weighted mean of b
            for p in 0..m {
                rhs[p] -= area[p] * mean_b;
            }
        }
        let bnorm = rhs.iter().map(|v| v * v).sum::<f64>().sqrt();
        let mut rep = ComponentSolve { id, n_cells: m, iterations: 0, rel_residual: 0.0, singular };
        if bnorm == 0.0 {
            return Ok((vec![0.0; m], rep));
        }
        // Jacobi preconditioner: 1 / diag(K).
        let dinv: Vec<f64> = cells
            .iter()
            .map(|&l| {
                let i = l as usize;
                let (a, b2) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
                let d = self.coef[a..b2].iter().sum::<f64>() + self.land_coef[i] + sigma * self.area[i];
                1.0 / d
            })
            .collect();
        let mut x = vec![0.0; m];
        let mut r = rhs.clone();
        let mut z: Vec<f64> = (0..m).map(|p| dinv[p] * r[p]).collect();
        let mut pdir = z.clone();
        let mut rz: f64 = (0..m).map(|p| r[p] * z[p]).sum();
        let mut kp = vec![0.0; m];
        let mut rel = 1.0;
        let mut it = 0usize;
        while rel > cfg.tol {
            if it >= cfg.max_iters {
                return Err(Error::NoConvergence { solver: "ocean_pcg".into(), iterations: it, residual: rel });
            }
            self.k_mul(cells, sigma, &pdir, &mut kp);
            let pkp: f64 = (0..m).map(|p| pdir[p] * kp[p]).sum();
            if !(pkp > 0.0) || !pkp.is_finite() {
                return Err(Error::NoConvergence { solver: "ocean_pcg (non-positive curvature)".into(), iterations: it, residual: rel });
            }
            let alpha = rz / pkp;
            for p in 0..m {
                x[p] += alpha * pdir[p];
                r[p] -= alpha * kp[p];
            }
            it += 1;
            rel = r.iter().map(|v| v * v).sum::<f64>().sqrt() / bnorm;
            for p in 0..m {
                z[p] = dinv[p] * r[p];
            }
            let rz_new: f64 = (0..m).map(|p| r[p] * z[p]).sum();
            let beta = rz_new / rz;
            rz = rz_new;
            for p in 0..m {
                pdir[p] = z[p] + beta * pdir[p];
            }
        }
        if singular {
            let sa: f64 = area.iter().sum();
            let mean = (0..m).map(|p| area[p] * x[p]).sum::<f64>() / sa;
            for v in x.iter_mut() {
                *v -= mean;
            }
        }
        rep.iterations = it;
        rep.rel_residual = rel;
        Ok((x, rep))
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
