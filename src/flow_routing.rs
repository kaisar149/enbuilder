//! Part 4b-1: depression handling, flow direction and drainage area on any mesh. NO rivers, NO lakes yet (4b-2).
//!
//! Module and fields
//!   flow_routing : elevation, ocean_mask
//!                  -> elevation_filled (m), depression_depth (m, >= 0), flow_dir (receiver cell index, -1 = outlet),
//!                     flow_order (cell indices, uphill -> downhill), drainage_area (m^2)
//!
//! Rules (all decided in 4b-1, see ARCHITECTURE.md):
//!  * Outlets (flow_dir = -1): every ocean cell; every cell with `boundary_len > 0` when the domain boundary is `Void`
//!    (water leaves: the waterfall; switch `void_outflow`); `Wall` stays closed. Routing height is `elevation` only
//!    (not geopotential, so on a cube the potential difference between face centres and edges is ignored).
//!  * Priority-flood (Barnes): heap key = (filled height, hops along the plateau, cell index), so it is deterministic and
//!    flats are resolved breadth-first from their spill point instead of by cell index.
//!  * `flow_dir` = steepest descent (drop / centre distance) among neighbours with strictly lower FILLED height; on a
//!    flat the flood parent. A strictly lower filled height was popped earlier, so following `flow_dir` can never cycle.
//!  * `flow_order` = reverse flood pop order: every cell comes before its receiver. All `n_cells` cells appear, layout
//!    `data[k] = cell index` (NOT a per-cell value).
//!  * `drainage_area` counts LAND area only (cells with `ocean_mask == 0`, lakes included). An ocean outlet therefore
//!    holds the land area draining into it, and the sum over all roots equals the total land area exactly.
//!  * No outlet (closed Wall world, or a component of the mesh that has none): the lowest cell of that component becomes a
//!    terminal sink (flow_dir -1, counted in `n_sinks`). `require_outlet = 1` turns that into an error instead.
//!  * Full recompute only (ignores `DirtyRegion`).
use crate::domain::{BoundaryKind, Domain};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// Heap entry. `Ord` is reversed so that `BinaryHeap` (a max-heap) pops the smallest key first.
#[derive(Clone, Copy, Debug)]
struct Item {
    h: f64,
    hops: u32,
    idx: u32,
}

impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool { self.cmp(o) == Ordering::Equal }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.h.total_cmp(&self.h).then_with(|| o.hops.cmp(&self.hops)).then_with(|| o.idx.cmp(&self.idx))
    }
}

/// Result of the priority flood.
#[derive(Debug, Clone)]
pub struct Flood {
    /// Filled height per cell (>= elevation; equal to elevation outside depressions and on outlets).
    pub filled: Vec<f64>,
    /// Cell that discovered this cell during the flood; -1 for roots (outlets and sinks).
    pub parent: Vec<i32>,
    /// Cells in the order the flood popped them (non-decreasing filled height). Always all `n_cells`.
    pub pop_order: Vec<u32>,
    /// True for outlets and terminal sinks.
    pub root: Vec<bool>,
    /// Terminal sinks added because a component had no outlet (ascending discovery order).
    pub sinks: Vec<u32>,
}

/// Priority-flood with plateau hop counts on the CSR adjacency. `outlet[i]` marks the seeds. Any component of the mesh
/// without a seed gets its lowest cell (ties: lowest index) as a terminal sink.
pub fn priority_flood(mesh: &Mesh, elevation: &[f64], outlet: &[bool]) -> Result<Flood> {
    let n = mesh.n_cells;
    if elevation.len() != n || outlet.len() != n {
        return Err(Error::InvalidParam(format!(
            "priority_flood: expected {n} cells, got {} elevations and {} outlet flags",
            elevation.len(),
            outlet.len()
        )));
    }
    if elevation.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite("elevation".into()));
    }
    // `+ 0.0` turns -0.0 into 0.0 so that `==` and `total_cmp` agree on every height.
    let z: Vec<f64> = elevation.iter().map(|&v| v + 0.0).collect();
    let mut filled = z.clone();
    let mut hops = vec![0u32; n];
    let mut parent = vec![-1i32; n];
    let mut visited = vec![false; n];
    let mut root = vec![false; n];
    let mut sinks: Vec<u32> = Vec::new();
    let mut pop_order: Vec<u32> = Vec::with_capacity(n);
    let mut heap: BinaryHeap<Item> = BinaryHeap::new();
    for i in 0..n {
        if outlet[i] {
            visited[i] = true;
            root[i] = true;
            heap.push(Item { h: z[i], hops: 0, idx: i as u32 });
        }
    }
    let mut by_height: Option<Vec<u32>> = None;
    let mut cursor = 0usize;
    loop {
        while let Some(it) = heap.pop() {
            let c = it.idx as usize;
            pop_order.push(it.idx);
            let fc = filled[c];
            let hc = hops[c];
            for &j in mesh.neighbors(c) {
                let j = j as usize;
                if visited[j] {
                    continue;
                }
                visited[j] = true;
                parent[j] = c as i32;
                if z[j] <= fc {
                    filled[j] = fc; // inside a depression or on a plateau: raised to the level of the cell that reached it
                    hops[j] = hc + 1;
                } else {
                    filled[j] = z[j];
                    hops[j] = 0;
                }
                heap.push(Item { h: filled[j], hops: hops[j], idx: j as u32 });
            }
        }
        if pop_order.len() == n {
            break;
        }
        // Some component has no outlet: its lowest cell becomes a terminal sink.
        let order = by_height.get_or_insert_with(|| {
            let mut v: Vec<u32> = (0..n as u32).collect();
            v.sort_by(|&a, &b| z[a as usize].total_cmp(&z[b as usize]).then(a.cmp(&b)));
            v
        });
        while visited[order[cursor] as usize] {
            cursor += 1;
        }
        let s = order[cursor] as usize;
        visited[s] = true;
        root[s] = true;
        sinks.push(s as u32);
        heap.push(Item { h: z[s], hops: 0, idx: s as u32 });
    }
    Ok(Flood { filled, parent, pop_order, root, sinks })
}

/// Receiver per cell: steepest descent on the filled surface among strictly lower neighbours, the flood parent on flats,
/// -1 on roots. Ties in slope go to the lower neighbour index.
pub fn flow_directions(mesh: &Mesh, fl: &Flood) -> Vec<i32> {
    let n = mesh.n_cells;
    let mut fd = vec![-1i32; n];
    for c in 0..n {
        if fl.root[c] {
            continue;
        }
        let fc = fl.filled[c];
        let nb = mesh.neighbors(c);
        let ed = mesh.edge_dists(c);
        let mut best = -1i32;
        let mut best_slope = 0.0f64;
        for (k, &j) in nb.iter().enumerate() {
            let fj = fl.filled[j as usize];
            if fj < fc {
                let slope = (fc - fj) / ed[k];
                if best < 0 || slope > best_slope {
                    best = j as i32;
                    best_slope = slope;
                }
            }
        }
        fd[c] = if best >= 0 { best } else { fl.parent[c] };
    }
    fd
}

/// Cells from uphill to downhill: every cell appears before its receiver.
pub fn flow_order(fl: &Flood) -> Vec<u32> {
    fl.pop_order.iter().rev().copied().collect()
}

/// Land-area accumulation along `order`. `land[i]` says whether cell `i` contributes its own area.
pub fn accumulate_area(mesh: &Mesh, flow_dir: &[i32], order: &[u32], land: &[bool]) -> Vec<f64> {
    let mut acc: Vec<f64> = (0..mesh.n_cells).map(|i| if land[i] { mesh.area[i] } else { 0.0 }).collect();
    for &c in order {
        let c = c as usize;
        let r = flow_dir[c];
        if r >= 0 {
            let v = acc[c];
            acc[r as usize] += v;
        }
    }
    acc
}

/// Root (outlet or sink) each cell drains to, following `flow_dir` from the downhill end of `order`.
pub fn root_of(flow_dir: &[i32], order: &[u32]) -> Vec<i32> {
    let mut root = vec![-1i32; flow_dir.len()];
    for &c in order.iter().rev() {
        let c = c as usize;
        root[c] = if flow_dir[c] < 0 { c as i32 } else { root[flow_dir[c] as usize] };
    }
    root
}

/// Outlet cells: ocean cells, plus (when `void_outflow` and the domain boundary is `Void`) cells with `boundary_len > 0`.
pub fn outlet_mask(domain: &Domain, ocean: &[bool], void_outflow: bool) -> Vec<bool> {
    let void = void_outflow && matches!(domain.boundary, BoundaryKind::Void);
    (0..domain.n_cells()).map(|i| ocean[i] || (void && domain.mesh.boundary_len[i] > 0.0)).collect()
}

// ---------------------------------------------------------------------------------------------
// Module: flow_routing
// ---------------------------------------------------------------------------------------------

/// Parameters: `void_outflow` (0/1, default 1), `require_outlet` (0/1, default 0).
/// Output-only (set by `run`): `n_outlets`, `n_void_outlets`, `n_sinks`, `n_depression_cells`, `max_depression_depth_m`.
pub struct FlowRouting {
    params: ParamSet,
}

impl Default for FlowRouting {
    fn default() -> Self { Self::new() }
}

impl FlowRouting {
    pub fn new() -> Self {
        FlowRouting {
            params: ParamSet::new()
                .with("void_outflow", 1.0)
                .with("require_outlet", 0.0)
                .with("n_outlets", 0.0)
                .with("n_void_outlets", 0.0)
                .with("n_sinks", 0.0)
                .with("n_depression_cells", 0.0)
                .with("max_depression_depth_m", 0.0),
        }
    }
}

impl Module for FlowRouting {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "flow_routing".into(),
            inputs: names(&["elevation", "ocean_mask"]),
            outputs: names(&["elevation_filled", "depression_depth", "flow_dir", "flow_order", "drainage_area"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        match name {
            "void_outflow" | "require_outlet" => {
                if value != 0.0 && value != 1.0 {
                    return Err(Error::InvalidParam(format!("flow_routing.{name} must be 0 or 1 (got {value})")));
                }
                self.params.set(name, value);
                Ok(())
            }
            _ => Err(Error::InvalidParam(format!("flow_routing: bad parameter '{name}'"))),
        }
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let z = fields.data("elevation")?.to_vec();
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        if z.len() != n || ocean.len() != n {
            return Err(Error::InvalidParam("flow_routing: elevation and ocean_mask must have one value per cell".into()));
        }
        let void_outflow = self.params.get_or("void_outflow", 1.0) > 0.5;
        let outlet = outlet_mask(domain, &ocean, void_outflow);
        let n_outlets = outlet.iter().filter(|&&o| o).count();
        let n_void = (0..n).filter(|&i| outlet[i] && !ocean[i]).count();

        let fl = priority_flood(&domain.mesh, &z, &outlet)?;
        if !fl.sinks.is_empty() && self.params.get_or("require_outlet", 0.0) > 0.5 {
            return Err(Error::InvalidParam(format!(
                "flow_routing: {} mesh component(s) have no outlet (no ocean cell, no Void edge) and require_outlet = 1",
                fl.sinks.len()
            )));
        }
        let fd = flow_directions(&domain.mesh, &fl);
        let order = flow_order(&fl);
        let land: Vec<bool> = ocean.iter().map(|&o| !o).collect();
        let area = accumulate_area(&domain.mesh, &fd, &order, &land);
        let depth: Vec<f64> = (0..n).map(|i| fl.filled[i] - z[i]).collect();

        self.params.set("n_outlets", n_outlets as f64);
        self.params.set("n_void_outlets", n_void as f64);
        self.params.set("n_sinks", fl.sinks.len() as f64);
        self.params.set("n_depression_cells", depth.iter().filter(|&&d| d > 0.0).count() as f64);
        self.params.set("max_depression_depth_m", depth.iter().cloned().fold(0.0, f64::max));

        fields.put("elevation_filled", 1, fl.filled)?;
        fields.put("depression_depth", 1, depth)?;
        fields.put("flow_dir", 1, fd.iter().map(|&v| v as f64).collect())?;
        fields.put("flow_order", 1, order.iter().map(|&v| v as f64).collect())?;
        fields.put("drainage_area", 1, area)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------------------------

/// Registers only `flow_routing` (needs `elevation` and `ocean_mask` from elsewhere, e.g. `add_ocean_stage`).
pub fn add_flow_routing_module(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(FlowRouting::new()))
}

/// Registers `sea_level`, `ocean_mask` and `flow_routing`; needs only `elevation`. Do not combine with
/// `add_ocean_mask_stage` / `add_ocean_stage` (duplicate modules); use `add_flow_routing_module` there.
pub fn add_flow_routing_stage(s: &mut Scheduler) -> Result<()> {
    crate::ocean::add_ocean_mask_stage(s)?;
    add_flow_routing_module(s)
}
