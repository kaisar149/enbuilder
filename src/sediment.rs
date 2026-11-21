//! Part 4c-2: sediment transport, deposition (floodplains, deltas, lake trapping) and thermal erosion. Volume is conserved.
//!
//! DECISIONS (recorded before coding, see ARCHITECTURE.md):
//!  * One producer per field (scheduler rule). `erosion` keeps `erosion_delta` (hydraulic CUT only, 4c-1 tests unchanged). `sediment`
//!    produces the NET carrier `terrain_delta` (m, cumulative: incision + deposition + thermal) and `terrain_step` (this step).
//!    `TerrainShaping::with_net_erosion()` reads `terrain_delta` instead of `erosion_delta`. (The PART text says "deposits raise
//!    `erosion_delta`"; that cannot hold with one producer per field, so the net field is new.)
//!  * Order inside one step: (1) eroded surface `z1 = elevation + erosion_step`; (2) sediment routed down the drainage tree with
//!    capacity-limited deposition; (3) thermal relaxation of `z2 = z1 + deposition`. `terrain_step = z3 - elevation`.
//!  * Thermal erosion is a pairwise, area-weighted, volume-conserving talus projection (Gauss-Seidel over edges until the largest
//!    excess is <= `thermal_tol_m` or `thermal_iters` sweeps). It moves material locally (talus aprons); it is NOT fed into the river
//!    flux (the PART text "plus thermal erosion" is read as: hillslope material is kept on the surface, the budget below is for the
//!    fluvial part, and the thermal part is separately zero-sum, `thermal_volume_error_m3`).
//!
//! Module "sediment"
//!   inputs : elevation, elevation_filled, flow_dir, drainage_area (or `discharge` with `Sediment::with_discharge()`), depression_depth,
//!            ocean_mask, rock_strength, erosion_step, terrain_delta_prev
//!   outputs: sediment (m^3/yr leaving the cell down `flow_dir`; at an outlet the volume leaving the model), sediment_capacity (m^3/yr),
//!            deposition (m raised this step, fluvial only, >= 0), thermal_step (m, signed), terrain_step (m), terrain_delta (m, cumulative),
//!            delta_mask (0/1), floodplain_mask (0/1)
//!
//! Law: capacity `qs_cap = Kc * Q^a * S` (m^3/yr; Q m^3/s = `q_per_area * drainage_area` or the `discharge` field; S = max(z1_c - base, 0)/L
//! to the receiver, base = lake SURFACE when the receiver is a lake cell). Load of a cell = own eroded volume + inflow. Excess over
//! `qs_cap * years_per_step` is deposited with fraction `deposit_fraction` (the rest moves on), limited by HEADROOM: a cell is never raised
//! above the lowest donor's effective height (lake cells count at their spill level), so deposition cannot create a pit.
//! Lakes (depression units from `lake_units`) trap ALL incoming sediment up to their basin volume below the spill level (water-filling from
//! the lowest floor); the remainder leaves through the exit cells (equal split). Ocean cells receiving sediment keep `deposit_fraction` of it
//! (raised at most to `sea_level`, never above their donors: a delta grows seaward), the rest is exported. Void-edge and closed-world sink
//! cells export everything.
//! Budget (m^3 per step, tested to 1e-8, observed ~1e-15):
//!   eroded = deposited_land + deposited_delta + stored_lakes + export_ocean + export_void + export_sink
//! and independently  sum(area * terrain_step) = -(export_ocean + export_void + export_sink)  (thermal is zero-sum).
//! Units are contracted to one node and ordered with Kahn's algorithm (smallest node first, as in `rivers_lakes`); a unit that would make the
//! contracted graph cyclic (tie artefact on flat terrain) is dissolved into ordinary cells and counted in `n_unresolved_lakes`.
//! Time handling as 4b-3/4c-1: reads the COMMITTED `terrain_delta_prev`; `commit_sediment_state` once per step.
//! `years_per_step` must equal the one of `erosion` (`set_surface_years`, `run_surface_steps` checks). Full recompute only.
use crate::consts::G_EARTH;
use crate::domain::{BoundaryKind, Domain};
use crate::erosion::{add_erosion_module, commit_erosion_state, seed_erosion_state};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::flow_routing::add_flow_routing_stage;
use crate::mesh::Mesh;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::rivers_lakes::lake_units;
use crate::scheduler::Scheduler;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

pub const SEDIMENT_PARAMS: [(&str, f64); 11] = [
    ("Kc", 5.0e7),                // m^3/yr per (m^3/s)^a. Derivation: with K 2e-7, m 0.5, n 1 the supply of a 1e12 m^2 basin is ~0.2*S m/yr per m^2 and the capacity Kc*q*S per m^2, equal at Kc ~ 2e7; 5e7 gives 2.5x headroom, so deposition happens at gradient breaks, lakes and mouths
    ("a_exp", 1.0),
    ("years_per_step", 1.0e5),    // must equal erosion.years_per_step
    ("q_per_area", 1.0e-8),       // m^3/s of discharge per m^2 of drainage area (area-form discharge, ~0.3 m/yr runoff)
    ("deposit_fraction", 0.5),    // share of the over-capacity excess deposited in the cell (rest moves on)
    ("talus_scale", 1.0),         // multiplies max_slope
    ("max_slope", 0.05),          // talus slope per unit distance (same meaning as terrain.max_slope), times g_ref/|g| and rock strength
    ("thermal_iters", 1000.0),    // maximum Gauss-Seidel sweeps (0 = thermal erosion off)
    ("thermal_tol_m", 1.0e-6),    // stop when no edge exceeds its limit by more than this many metres of height difference
    ("sea_level", 0.0),
    ("floodplain_slope", 0.005),  // deposits on land reaches with S <= this are floodplain
];

const OUTPUT_ONLY: [&str; 17] = [
    "eroded_volume_m3", "deposited_land_m3", "deposited_delta_m3", "stored_lakes_m3", "export_ocean_m3", "export_void_m3", "export_sink_m3",
    "balance_residual", "thermal_moved_m3", "thermal_volume_error_m3", "thermal_iters_used", "thermal_max_excess_m", "n_unresolved_lakes",
    "n_deposit_cells", "n_delta_cells", "n_floodplain_cells", "max_deposit_m",
];

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// Transport capacity `Kc * Q^a * S` (m^3/yr). Zero when any factor is not positive.
pub fn transport_capacity(kc: f64, q: f64, a_exp: f64, slope: f64) -> f64 {
    if !(kc > 0.0) || !(q > 0.0) || !(slope > 0.0) { 0.0 } else { kc * q.powf(a_exp) * slope }
}

/// Water-filling of a basin with sediment: raises the lowest floors to a common level `L <= top` holding `volume` (m^3). Returns
/// `(level, trapped)`: `trapped = min(volume, capacity)` with `capacity = sum area * max(top - floor, 0)`; cell k ends at `max(floor_k, level)`.
/// `volume <= 0` returns `(lowest floor, 0)`.
pub fn fill_basin(floors: &[f64], areas: &[f64], top: f64, volume: f64) -> (f64, f64) {
    let m = floors.len();
    if m == 0 {
        return (top, 0.0);
    }
    let cap: f64 = (0..m).map(|k| areas[k] * (top - floors[k]).max(0.0)).sum();
    let lowest = floors.iter().cloned().fold(f64::INFINITY, f64::min);
    if !(volume > 0.0) {
        return (lowest, 0.0);
    }
    if volume >= cap {
        return (top, cap);
    }
    let mut idx: Vec<usize> = (0..m).collect();
    idx.sort_by(|&a, &b| floors[a].total_cmp(&floors[b]).then(a.cmp(&b)));
    let (mut acc, mut asum) = (0.0f64, 0.0f64);
    let mut cur = floors[idx[0]];
    for (t, &k) in idx.iter().enumerate() {
        if t > 0 {
            let nxt = floors[k];
            let need = asum * (nxt - cur);
            if acc + need >= volume {
                return (cur + (volume - acc) / asum, volume);
            }
            acc += need;
            cur = nxt;
        }
        asum += areas[k];
    }
    ((cur + (volume - acc) / asum).min(top), volume)
}

/// What `thermal_relax` did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalReport {
    /// Sweeps run.
    pub iters: usize,
    /// Largest `|dz| - limit` over all edges at the end (m); <= `tol` when converged.
    pub max_excess_m: f64,
    /// Sum of the volumes moved (m^3).
    pub moved_m3: f64,
}

/// Talus relaxation, volume conserving. Edge limit `max_slope * (g_ref / g_pair) * strength_pair * edge_dist` (pair values are means).
/// Each sweep visits every edge once (cells in index order, neighbours in adjacency order, only `j > i`) and moves the volume that brings
/// that pair exactly to its limit (area-weighted: `i` loses `v/area_i`, `j` gains `v/area_j`). This is a cyclic projection onto convex sets
/// in the area-weighted norm, so it converges; it stops when the largest excess is `<= tol` or after `iters` sweeps.
pub fn thermal_relax(mesh: &Mesh, z: &mut [f64], g: &[f64], strength: &[f64], max_slope: f64, iters: usize, tol: f64) -> ThermalReport {
    let n = mesh.n_cells;
    let mut lim = vec![0.0; mesh.adj.len()];
    for i in 0..n {
        let off = mesh.adj_offsets[i] as usize;
        let nb = mesh.neighbors(i);
        let ds = mesh.edge_dists(i);
        for q in 0..nb.len() {
            let j = nb[q] as usize;
            if j > i {
                let gp = 0.5 * (g[i] + g[j]);
                let sp = 0.5 * (strength[i] + strength[j]);
                lim[off + q] = max_slope * (G_EARTH / gp) * sp * ds[q];
            }
        }
    }
    let mut it = 0usize;
    let mut moved = 0.0f64;
    let mut mx: f64;
    loop {
        mx = 0.0f64;
        for i in 0..n {
            let off = mesh.adj_offsets[i] as usize;
            let nb = mesh.neighbors(i);
            for q in 0..nb.len() {
                let j = nb[q] as usize;
                if j > i {
                    let ex = (z[i] - z[j]).abs() - lim[off + q];
                    if ex > mx {
                        mx = ex;
                    }
                }
            }
        }
        if mx <= tol || it >= iters {
            break;
        }
        for i in 0..n {
            let off = mesh.adj_offsets[i] as usize;
            let nb = mesh.neighbors(i);
            for q in 0..nb.len() {
                let j = nb[q] as usize;
                if j > i {
                    let d = z[i] - z[j];
                    let ex = d.abs() - lim[off + q];
                    if ex > 0.0 {
                        let v = ex / (1.0 / mesh.area[i] + 1.0 / mesh.area[j]);
                        if d > 0.0 {
                            z[i] -= v / mesh.area[i];
                            z[j] += v / mesh.area[j];
                        } else {
                            z[j] -= v / mesh.area[j];
                            z[i] += v / mesh.area[i];
                        }
                        moved += v;
                    }
                }
            }
        }
        it += 1;
    }
    ThermalReport { iters: it, max_excess_m: mx, moved_m3: moved }
}

/// Volume budget of one routing pass (m^3).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SedimentBudget {
    pub eroded: f64,
    pub deposited_land: f64,
    pub deposited_delta: f64,
    pub stored_lakes: f64,
    pub export_ocean: f64,
    pub export_void: f64,
    pub export_sink: f64,
}

impl SedimentBudget {
    /// `eroded - (deposits + lake storage + exports)`.
    pub fn imbalance(&self) -> f64 {
        self.eroded - (self.deposited_land + self.deposited_delta + self.stored_lakes + self.export_ocean + self.export_void + self.export_sink)
    }
    /// `|imbalance| / eroded` (0 when nothing was eroded).
    pub fn residual(&self) -> f64 {
        if self.eroded > 0.0 { self.imbalance().abs() / self.eroded } else { self.imbalance().abs() }
    }
    /// Volume leaving the model.
    pub fn exported(&self) -> f64 { self.export_ocean + self.export_void + self.export_sink }
}

/// Inputs of `route_sediment`; every slice has one value per cell.
pub struct RouteInput<'a> {
    /// Eroded surface (m).
    pub z1: &'a [f64],
    /// Eroded volume supplied by each cell (m^3, >= 0).
    pub src: &'a [f64],
    pub filled: &'a [f64],
    pub depth: &'a [f64],
    /// Receiver cell or -1.
    pub flow_dir: &'a [i64],
    pub ocean: &'a [bool],
    /// Discharge (m^3/s).
    pub q: &'a [f64],
}

#[derive(Debug, Clone, Copy)]
pub struct RouteParams {
    pub kc: f64,
    pub a_exp: f64,
    pub years: f64,
    pub deposit_fraction: f64,
    pub sea_level: f64,
    pub floodplain_slope: f64,
}

/// Result of `route_sediment`.
#[derive(Debug, Clone)]
pub struct Routed {
    /// Fluvial deposition height per cell (m, >= 0).
    pub deposit: Vec<f64>,
    /// Volume leaving each cell down `flow_dir` (m^3/yr); at outlet cells the volume leaving the model.
    pub flux: Vec<f64>,
    /// Transport capacity (m^3/yr), 0 for ocean, lake and outlet cells.
    pub capacity: Vec<f64>,
    /// Slope used for the capacity (land cells with a receiver), else 0.
    pub slope: Vec<f64>,
    pub delta_mask: Vec<bool>,
    pub floodplain_mask: Vec<bool>,
    /// Lake unit of each cell (-1 none); dissolved units are -1.
    pub lake: Vec<i32>,
    pub budget: SedimentBudget,
    pub n_unresolved_lakes: usize,
}

fn pass_down(hmin: &mut [f64], fd: &[i64], c: usize, h: f64) {
    if fd[c] >= 0 {
        let r = fd[c] as usize;
        if h < hmin[r] {
            hmin[r] = h;
        }
    }
}

/// Sediment routing on a drainage tree (see the module doc). Pure and deterministic.
pub fn route_sediment(domain: &Domain, inp: &RouteInput, p: &RouteParams) -> Result<Routed> {
    let n = domain.n_cells();
    let mesh = &domain.mesh;
    let (z1, src, filled, fd, ocean, q) = (inp.z1, inp.src, inp.filled, inp.flow_dir, inp.ocean, inp.q);
    if [z1.len(), src.len(), filled.len(), inp.depth.len(), fd.len(), ocean.len(), q.len()].iter().any(|&l| l != n) {
        return Err(Error::InvalidParam("sediment: every input must have one value per cell".into()));
    }
    let (uid, units) = lake_units(domain, filled, inp.depth);
    let mut dissolved = vec![false; units.len()];
    let (node, members, seq) = loop {
        let node: Vec<usize> = (0..n).map(|c| if uid[c] >= 0 && !dissolved[uid[c] as usize] { n + uid[c] as usize } else { c }).collect();
        let total = n + units.len();
        let mut exists = vec![false; total];
        for c in 0..n {
            exists[node[c]] = true;
        }
        let mut indeg = vec![0u32; total];
        for c in 0..n {
            if fd[c] >= 0 {
                let w = node[fd[c] as usize];
                if w != node[c] {
                    indeg[w] += 1;
                }
            }
        }
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); total];
        for c in 0..n {
            members[node[c]].push(c);
        }
        let mut heap = BinaryHeap::new();
        for v in 0..total {
            if exists[v] && indeg[v] == 0 {
                heap.push(Reverse(v));
            }
        }
        let mut seq: Vec<usize> = Vec::new();
        while let Some(Reverse(v)) = heap.pop() {
            seq.push(v);
            for &c in &members[v] {
                if fd[c] >= 0 {
                    let w = node[fd[c] as usize];
                    if w != v {
                        indeg[w] -= 1;
                        if indeg[w] == 0 {
                            heap.push(Reverse(w));
                        }
                    }
                }
            }
        }
        let n_nodes = exists.iter().filter(|&&e| e).count();
        if seq.len() == n_nodes {
            break (node, members, seq);
        }
        let mut done = vec![false; total];
        for &v in &seq {
            done[v] = true;
        }
        let mut any = false;
        for u in 0..units.len() {
            if !dissolved[u] && exists[n + u] && !done[n + u] {
                dissolved[u] = true;
                any = true;
            }
        }
        if !any {
            return Err(Error::InvalidParam("sediment: flow_dir contains a cycle".into()));
        }
    };
    let lake: Vec<i32> = (0..n).map(|c| if uid[c] >= 0 && !dissolved[uid[c] as usize] { uid[c] } else { -1 }).collect();
    let void_domain = matches!(domain.boundary, BoundaryKind::Void);

    let mut load: Vec<f64> = src.to_vec();
    let mut hmin = vec![f64::INFINITY; n];
    let mut dh = vec![0.0; n];
    let mut capacity = vec![0.0; n];
    let mut flux = vec![0.0; n];
    let mut slope = vec![0.0; n];
    let mut b = SedimentBudget { eroded: src.iter().sum(), ..Default::default() };
    let years = p.years;

    for &v in &seq {
        if v >= n {
            let mem = &members[v];
            let l: f64 = mem.iter().map(|&c| load[c]).sum();
            let floors: Vec<f64> = mem.iter().map(|&c| z1[c]).collect();
            let areas: Vec<f64> = mem.iter().map(|&c| mesh.area[c]).collect();
            let top = filled[mem[0]];
            let (lvl, trapped) = fill_basin(&floors, &areas, top, l);
            if trapped > 0.0 {
                for &c in mem.iter() {
                    dh[c] = (lvl - z1[c]).max(0.0);
                }
            }
            b.stored_lakes += trapped;
            let outv = l - trapped;
            let exits: Vec<usize> = mem.iter().cloned().filter(|&c| fd[c] >= 0 && node[fd[c] as usize] != v).collect();
            if exits.is_empty() {
                b.export_sink += outv;
            } else {
                let share = outv / exits.len() as f64;
                for &c in &exits {
                    let r = fd[c] as usize;
                    load[r] += share;
                    flux[c] = share / years;
                    pass_down(&mut hmin, fd, c, filled[c]);
                }
            }
            continue;
        }
        let c = v;
        let lc = load[c];
        if ocean[c] {
            let hr = (hmin[c].min(p.sea_level) - z1[c]).max(0.0);
            let d = (p.deposit_fraction * lc).min(hr * mesh.area[c]);
            dh[c] = d / mesh.area[c];
            b.deposited_delta += d;
            b.export_ocean += lc - d;
            flux[c] = (lc - d) / years;
            continue;
        }
        if fd[c] < 0 {
            if void_domain && mesh.boundary_len[c] > 0.0 {
                b.export_void += lc;
            } else {
                b.export_sink += lc;
            }
            flux[c] = lc / years;
            continue;
        }
        let r = fd[c] as usize;
        let dist = match mesh.neighbors(c).iter().position(|&j| j as usize == r) {
            Some(k) => mesh.edge_dists(c)[k],
            None => return Err(Error::InvalidParam("sediment: flow_dir receiver is not a neighbour".into())),
        };
        let base = if lake[r] >= 0 { filled[r] } else { z1[r] };
        let s = (z1[c] - base).max(0.0) / dist;
        slope[c] = s;
        let cap = transport_capacity(p.kc, q[c], p.a_exp, s);
        capacity[c] = cap;
        let excess = (lc - cap * years).max(0.0);
        let mut d = p.deposit_fraction * excess;
        if hmin[c].is_finite() {
            d = d.min((hmin[c] - z1[c]).max(0.0) * mesh.area[c]);
        }
        dh[c] = d / mesh.area[c];
        b.deposited_land += d;
        let outv = lc - d;
        load[r] += outv;
        flux[c] = outv / years;
        pass_down(&mut hmin, fd, c, z1[c] + dh[c]);
    }

    let mut delta_mask = vec![false; n];
    let mut floodplain_mask = vec![false; n];
    for c in 0..n {
        if !(dh[c] > 0.0) {
            continue;
        }
        if ocean[c] || (fd[c] >= 0 && ocean[fd[c] as usize]) {
            delta_mask[c] = true;
        } else if lake[c] < 0 && slope[c] <= p.floodplain_slope {
            floodplain_mask[c] = true;
        }
    }
    Ok(Routed {
        deposit: dh,
        flux,
        capacity,
        slope,
        delta_mask,
        floodplain_mask,
        lake,
        budget: b,
        n_unresolved_lakes: dissolved.iter().filter(|&&d| d).count(),
    })
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// Create `terrain_delta_prev = 0`.
pub fn seed_sediment_state(fields: &mut FieldStore) -> Result<()> {
    let n = fields.n_cells();
    fields.put("terrain_delta_prev", 1, vec![0.0; n])
}

/// Copy `terrain_delta` into `terrain_delta_prev`: advances the surface time by one step. Call once per step.
pub fn commit_sediment_state(fields: &mut FieldStore) -> Result<()> {
    let d = fields.data("terrain_delta")?.to_vec();
    fields.put("terrain_delta_prev", 1, d)
}

// ---------------------------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------------------------

/// Sediment transport, deposition and thermal erosion. Parameters: see `SEDIMENT_PARAMS`; output-only: see `OUTPUT_ONLY` in the source
/// (`eroded_volume_m3`, `deposited_land_m3`, `deposited_delta_m3`, `stored_lakes_m3`, `export_ocean_m3`, `export_void_m3`, `export_sink_m3`,
/// `balance_residual`, `thermal_moved_m3`, `thermal_volume_error_m3`, `thermal_iters_used`, `thermal_max_excess_m`, `n_unresolved_lakes`,
/// `n_deposit_cells`, `n_delta_cells`, `n_floodplain_cells`, `max_deposit_m`).
pub struct Sediment {
    params: ParamSet,
    discharge: bool,
}

impl Default for Sediment {
    fn default() -> Self { Self::new() }
}

impl Sediment {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in SEDIMENT_PARAMS.iter() {
            params.set(k, *v);
        }
        for k in OUTPUT_ONLY.iter() {
            params.set(k, 0.0);
        }
        Sediment { params, discharge: false }
    }

    /// Variant that reads the `discharge` field (m^3/s, from `rivers_lakes`) instead of `q_per_area * drainage_area`.
    pub fn with_discharge() -> Self {
        let mut s = Self::new();
        s.discharge = true;
        s
    }
}

impl Module for Sediment {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "sediment".into(),
            inputs: names(&[
                "elevation", "elevation_filled", "flow_dir", if self.discharge { "discharge" } else { "drainage_area" }, "depression_depth", "ocean_mask",
                "rock_strength", "erosion_step", "terrain_delta_prev",
            ]),
            outputs: names(&["sediment", "sediment_capacity", "deposition", "thermal_step", "terrain_step", "terrain_delta", "delta_mask", "floodplain_mask"]),
            anomaly_hooks: names(&["gravity", "rock_strength", "time_flow_rate"]),
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !SEDIMENT_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("sediment: bad parameter '{name}'")));
        }
        let bad = match name {
            "Kc" | "q_per_area" | "thermal_iters" | "thermal_tol_m" | "floodplain_slope" => value < 0.0,
            "a_exp" | "years_per_step" | "talus_scale" | "max_slope" => value <= 0.0,
            "deposit_fraction" => !(0.0..=1.0).contains(&value),
            _ => false,
        };
        if bad {
            return Err(Error::InvalidParam(format!("sediment: '{name}' out of range (got {value})")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let mesh = &domain.mesh;
        let z = fields.data("elevation")?.to_vec();
        let filled = fields.data("elevation_filled")?.to_vec();
        let fd: Vec<i64> = fields.data("flow_dir")?.iter().map(|&v| v as i64).collect();
        let area_drain = if self.discharge { fields.data("discharge")?.to_vec() } else { fields.data("drainage_area")?.to_vec() };
        let dep = fields.data("depression_depth")?.to_vec();
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        let strength = fields.data("rock_strength")?.to_vec();
        let estep = fields.data("erosion_step")?.to_vec();
        let prev = fields.data("terrain_delta_prev")?.to_vec();
        if [z.len(), filled.len(), fd.len(), area_drain.len(), dep.len(), ocean.len(), strength.len(), estep.len(), prev.len()].iter().any(|&l| l != n) {
            return Err(Error::InvalidParam("sediment: every input field must have one value per cell".into()));
        }
        for (name, v) in [("elevation", &z), ("elevation_filled", &filled), ("drainage_area", &area_drain), ("depression_depth", &dep), ("rock_strength", &strength), ("erosion_step", &estep), ("terrain_delta_prev", &prev)] {
            if v.iter().any(|x| !x.is_finite()) {
                return Err(Error::NonFinite(name.into()));
            }
        }
        if fd.iter().any(|&r| r >= n as i64) {
            return Err(Error::InvalidParam("sediment: flow_dir out of range".into()));
        }
        let g: Vec<f64> = (0..n).map(|c| domain.gravity_magnitude(c)).collect();
        if g.iter().any(|&x| !(x > 0.0)) {
            return Err(Error::InvalidParam("sediment: gravity must be positive in every cell".into()));
        }
        let p = |k: &str| self.params.get_or(k, 0.0);
        let years = p("years_per_step");
        let z1: Vec<f64> = (0..n).map(|c| z[c] + estep[c]).collect();
        let src: Vec<f64> = (0..n).map(|c| mesh.area[c] * (-estep[c]).max(0.0)).collect();
        let q: Vec<f64> = if self.discharge { area_drain.iter().map(|&d| d.max(0.0)).collect() } else { area_drain.iter().map(|&a| p("q_per_area") * a.max(0.0)).collect() };
        let rp = RouteParams { kc: p("Kc"), a_exp: p("a_exp"), years, deposit_fraction: p("deposit_fraction"), sea_level: p("sea_level"), floodplain_slope: p("floodplain_slope") };
        let routed = route_sediment(domain, &RouteInput { z1: &z1, src: &src, filled: &filled, depth: &dep, flow_dir: &fd, ocean: &ocean, q: &q }, &rp)?;

        let z2: Vec<f64> = (0..n).map(|c| z1[c] + routed.deposit[c]).collect();
        let mut z3 = z2.clone();
        let iters = p("thermal_iters") as usize;
        let rep = if iters > 0 {
            thermal_relax(mesh, &mut z3, &g, &strength, p("talus_scale") * p("max_slope"), iters, p("thermal_tol_m"))
        } else {
            ThermalReport { iters: 0, max_excess_m: 0.0, moved_m3: 0.0 }
        };
        let tstep: Vec<f64> = (0..n).map(|c| z3[c] - z2[c]).collect();
        let tvol: f64 = (0..n).map(|c| mesh.area[c] * tstep[c]).sum();
        let step: Vec<f64> = (0..n).map(|c| z3[c] - z[c]).collect();
        let delta: Vec<f64> = (0..n).map(|c| prev[c] + step[c]).collect();
        let b = routed.budget;
        let flag = |v: &Vec<bool>| -> Vec<f64> { v.iter().map(|&x| if x { 1.0 } else { 0.0 }).collect() };
        let (dm, fm) = (flag(&routed.delta_mask), flag(&routed.floodplain_mask));
        let max_dep = routed.deposit.iter().cloned().fold(0.0, f64::max);
        self.params.set("eroded_volume_m3", b.eroded);
        self.params.set("deposited_land_m3", b.deposited_land);
        self.params.set("deposited_delta_m3", b.deposited_delta);
        self.params.set("stored_lakes_m3", b.stored_lakes);
        self.params.set("export_ocean_m3", b.export_ocean);
        self.params.set("export_void_m3", b.export_void);
        self.params.set("export_sink_m3", b.export_sink);
        self.params.set("balance_residual", b.residual());
        self.params.set("thermal_moved_m3", rep.moved_m3);
        self.params.set("thermal_volume_error_m3", tvol);
        self.params.set("thermal_iters_used", rep.iters as f64);
        self.params.set("thermal_max_excess_m", rep.max_excess_m);
        self.params.set("n_unresolved_lakes", routed.n_unresolved_lakes as f64);
        self.params.set("n_deposit_cells", routed.deposit.iter().filter(|&&d| d > 0.0).count() as f64);
        self.params.set("n_delta_cells", routed.delta_mask.iter().filter(|&&d| d).count() as f64);
        self.params.set("n_floodplain_cells", routed.floodplain_mask.iter().filter(|&&d| d).count() as f64);
        self.params.set("max_deposit_m", max_dep);
        fields.put("sediment", 1, routed.flux)?;
        fields.put("sediment_capacity", 1, routed.capacity)?;
        fields.put("deposition", 1, routed.deposit)?;
        fields.put("thermal_step", 1, tstep)?;
        fields.put("terrain_step", 1, step)?;
        fields.put("terrain_delta", 1, delta)?;
        fields.put("delta_mask", 1, dm)?;
        fields.put("floodplain_mask", 1, fm)
    }
}

/// Registers only `sediment`.
pub fn add_sediment_module(s: &mut Scheduler) -> Result<()> { s.register(Box::new(Sediment::new())) }

/// Registers `flow_routing` (with the ocean mask), `erosion` and `sediment`. The scheduler orders `flow_routing -> erosion -> sediment`.
pub fn add_surface_stage(s: &mut Scheduler) -> Result<()> {
    add_flow_routing_stage(s)?;
    add_erosion_module(s)?;
    add_sediment_module(s)
}

/// Sets `years_per_step` on both `erosion` and `sediment` (they must agree).
pub fn set_surface_years(s: &mut Scheduler, years: f64) -> Result<()> {
    s.set_param("erosion", "years_per_step", years)?;
    s.set_param("sediment", "years_per_step", years)
}

/// `erosion_delta_prev` and `terrain_delta_prev` = 0.
pub fn seed_surface_state(fields: &mut FieldStore) -> Result<()> {
    seed_erosion_state(fields)?;
    seed_sediment_state(fields)
}

/// What one step of `run_surface_steps` reports (m^3 unless noted).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceStep {
    pub eroded_volume_m3: f64,
    pub deposited_m3: f64,
    pub stored_lakes_m3: f64,
    pub exported_m3: f64,
    pub balance_residual: f64,
    pub thermal_max_excess_m: f64,
    pub n_depression_cells: usize,
    pub n_delta_cells: usize,
    pub n_floodplain_cells: usize,
}

/// Stand-in driver until the 4c-3 loop: `steps` times run the scheduler `s` (`add_surface_stage`: routing, erosion, sediment on the
/// current `elevation`), add `terrain_step` to `elevation` (what `terrain::with_net_erosion` will do through `terrain_delta` in 4c-3) and
/// commit both states. `fields` needs `elevation`, `rock_strength` and the seeded state (`seed_surface_state`).
pub fn run_surface_steps(domain: &Domain, fields: &mut FieldStore, s: &mut Scheduler, steps: usize) -> Result<Vec<SurfaceStep>> {
    let ye = s.module_param("erosion", "years_per_step");
    let ys = s.module_param("sediment", "years_per_step");
    if ye.is_none() || ys.is_none() || ye != ys {
        return Err(Error::InvalidParam("run_surface_steps: erosion and sediment must both exist and share years_per_step".into()));
    }
    let mut out = Vec::with_capacity(steps);
    for _ in 0..steps {
        s.run(domain, fields, &DirtyRegion::All)?;
        let z = fields.data("elevation")?.to_vec();
        let st = fields.data("terrain_step")?.to_vec();
        let nz: Vec<f64> = z.iter().zip(st.iter()).map(|(a, b)| a + b).collect();
        let ndep = fields.data("depression_depth")?.iter().filter(|&&d| d > 0.0).count();
        let m = |k: &str| s.module_param("sediment", k).unwrap_or(0.0);
        out.push(SurfaceStep {
            eroded_volume_m3: m("eroded_volume_m3"),
            deposited_m3: m("deposited_land_m3") + m("deposited_delta_m3"),
            stored_lakes_m3: m("stored_lakes_m3"),
            exported_m3: m("export_ocean_m3") + m("export_void_m3") + m("export_sink_m3"),
            balance_residual: m("balance_residual"),
            thermal_max_excess_m: m("thermal_max_excess_m"),
            n_depression_cells: ndep,
            n_delta_cells: m("n_delta_cells") as usize,
            n_floodplain_cells: m("n_floodplain_cells") as usize,
        });
        fields.put("elevation", 1, nz)?;
        commit_erosion_state(fields)?;
        commit_sediment_state(fields)?;
    }
    Ok(out)
}
