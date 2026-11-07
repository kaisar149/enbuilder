//! Part 4b-2: runoff, discharge, rivers and fill-and-spill lakes on top of the 4b-1 drainage tree.
//!
//! Module and fields
//!   rivers_lakes : elevation, ocean_mask, elevation_filled, depression_depth, flow_dir, flow_order, precipitation, evaporation
//!                  -> runoff (kg/m^2/s), discharge (m^3/s), river_mask (0/1), lake_id (-1 none), lake_depth (m),
//!                     lake_volume (m^3 per cell), lake_outflow (m^3/s, on lake cells), salt_flat (0/1)
//!
//! Rules (decided in 4b-2, see ARCHITECTURE.md):
//!  * Runoff on land = `runoff_coeff * max(precipitation, 0)`; volume flux = runoff * area / `rho_water`. The rest is "land loss".
//!  * A lake UNIT is a connected set of cells with `depression_depth > 0` and equal `elevation_filled` (the merged basin of 4b-1,
//!    so adjacent basins that fill to one level are one lake). Its spill level is that filled height. Units are numbered from 0 by
//!    lowest cell index; `lake_id` is set on every unit cell, wet or dry.
//!  * Steady-state balance per unit: inflow `Qin` = own runoff + everything draining in (after upstream lake losses). Evaporation of
//!    the full surface `Efull = sum(area * lake_evap_factor * evaporation / rho_water)`.
//!    `Qin > Efull`: full to the spill level, outflow `Qin - Efull` leaves through the unit's exit cells (equal split) and continues
//!    down `flow_dir`. Otherwise endorheic: no outflow, cells are wetted from the lowest upwards until evaporation equals `Qin`
//!    (the last cell partially; level = floor of the first cell not fully wetted, or the spill level if all are wet).
//!    No inflow = dry. Cells of an endorheic unit with depth < `min_lake_depth_m` are `salt_flat`.
//!  * Accumulation contracts each unit to one node and runs Kahn's algorithm (smallest node first, deterministic). If a tie in an
//!    exactly flat world makes the contracted graph cyclic, the leftover cells are accumulated with no lake loss ("pass-through")
//!    and counted in `n_unresolved_lakes`; their lake fields stay zero.
//!  * River mask: `discharge > river_threshold_m3s` on non-lake land, forward-filled downstream (a lake loss never makes a gap)
//!    and carried through spilling lakes.
//!  * Conservation (steady state, lake storage change = 0), m^3/s:
//!    land precipitation = land loss + lake evaporation + ocean discharge + Void outflow + terminal-sink inflow.
//!    `balance_residual` (relative) is exposed and tested to 1e-8.
//!  * Full recompute only. Lake direct rainfall is not split out (a lake cell's rain runs off like any land cell).
use crate::domain::{BoundaryKind, Domain};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// Outcome of the steady-state balance of one lake unit.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeSolve {
    /// Water depth per cell (m), aligned with the inputs, >= 0.
    pub depth: Vec<f64>,
    /// Outflow over the spill (m^3/s), > 0 only when the lake is full and still has inflow left after evaporation.
    pub q_out: f64,
    /// Evaporation from the lake surface (m^3/s).
    pub evap: f64,
    /// All cells wet up to the spill level.
    pub full: bool,
}

/// Steady-state fill-and-spill of one basin. `level` = spill height, `z` = cell floors (< level), `area` (m^2),
/// `ev` = evaporated water depth rate per cell (m/s, already scaled by the lake factor), `q_in` = inflow (m^3/s).
/// Conserves water exactly: `q_in = q_out + evap`.
pub fn solve_lake(level: f64, z: &[f64], area: &[f64], ev: &[f64], q_in: f64) -> LakeSolve {
    let m = z.len();
    if !(q_in > 0.0) {
        return LakeSolve { depth: vec![0.0; m], q_out: 0.0, evap: 0.0, full: false };
    }
    let e_full: f64 = (0..m).map(|i| area[i] * ev[i]).sum();
    if q_in > e_full {
        let depth: Vec<f64> = (0..m).map(|i| (level - z[i]).max(0.0)).collect();
        return LakeSolve { depth, q_out: q_in - e_full, evap: e_full, full: true };
    }
    let mut idx: Vec<usize> = (0..m).collect();
    idx.sort_by(|&a, &b| z[a].total_cmp(&z[b]).then(a.cmp(&b)));
    let mut cum = 0.0;
    let mut k = 0usize;
    while k < m {
        let ek = area[idx[k]] * ev[idx[k]];
        if cum + ek <= q_in {
            cum += ek;
            k += 1;
        } else {
            break;
        }
    }
    let h = if k < m { z[idx[k]] } else { level };
    let mut depth = vec![0.0; m];
    for &i in &idx[..k] {
        depth[i] = (h - z[i]).max(0.0);
    }
    LakeSolve { depth, q_out: 0.0, evap: q_in, full: k == m }
}

/// Connected components of cells with `depth > 0` and equal filled height. Returns (`unit_id` per cell, -1 outside; cells per unit,
/// ascending). Units are numbered by their lowest cell index.
pub fn lake_units(domain: &Domain, filled: &[f64], depth: &[f64]) -> (Vec<i32>, Vec<Vec<usize>>) {
    let n = domain.n_cells();
    let mut id = vec![-1i32; n];
    let mut units: Vec<Vec<usize>> = Vec::new();
    for s in 0..n {
        if id[s] >= 0 || !(depth[s] > 0.0) {
            continue;
        }
        let u = units.len() as i32;
        id[s] = u;
        let mut stack = vec![s];
        let mut members = Vec::new();
        while let Some(c) = stack.pop() {
            members.push(c);
            for &j in domain.mesh.neighbors(c) {
                let j = j as usize;
                if id[j] < 0 && depth[j] > 0.0 && filled[j] == filled[s] {
                    id[j] = u;
                    stack.push(j);
                }
            }
        }
        members.sort_unstable();
        units.push(members);
    }
    (id, units)
}

// ---------------------------------------------------------------------------------------------
// Module: rivers_lakes
// ---------------------------------------------------------------------------------------------

/// Parameters: `runoff_coeff` 0.4 (0..1), `river_threshold_m3s` 100, `lake_evap_factor` 3 (open water relative to the land
/// `evaporation` field, whose default land availability is 0.3), `min_lake_depth_m` 0.5, `rho_water` 1000.
/// Output-only (set by `run`, all m^3/s unless noted): `precip_land_m3s`, `land_loss_m3s`, `lake_evap_m3s`, `ocean_discharge_m3s`,
/// `void_outflow_m3s`, `sink_inflow_m3s`, `lake_storage_change_m3s` (0, steady state), `balance_residual` (relative),
/// `n_lakes`, `n_lakes_spilling`, `n_lakes_endorheic`, `n_lakes_dry`, `n_unresolved_lakes`, `river_cells`, `lake_area_m2`, `lake_volume_m3`.
pub struct RiversLakes {
    params: ParamSet,
}

impl Default for RiversLakes {
    fn default() -> Self { Self::new() }
}

const OUTPUT_ONLY: [&str; 16] = [
    "precip_land_m3s",
    "land_loss_m3s",
    "lake_evap_m3s",
    "ocean_discharge_m3s",
    "void_outflow_m3s",
    "sink_inflow_m3s",
    "lake_storage_change_m3s",
    "balance_residual",
    "n_lakes",
    "n_lakes_spilling",
    "n_lakes_endorheic",
    "n_lakes_dry",
    "n_unresolved_lakes",
    "river_cells",
    "lake_area_m2",
    "lake_volume_m3",
];

impl RiversLakes {
    pub fn new() -> Self {
        let mut params = ParamSet::new()
            .with("runoff_coeff", 0.4)
            .with("river_threshold_m3s", 100.0)
            .with("lake_evap_factor", 3.0)
            .with("min_lake_depth_m", 0.5)
            .with("rho_water", 1000.0);
        for k in OUTPUT_ONLY.iter() {
            params.set(k, 0.0);
        }
        RiversLakes { params }
    }
}

impl Module for RiversLakes {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "rivers_lakes".into(),
            inputs: names(&["elevation", "ocean_mask", "elevation_filled", "depression_depth", "flow_dir", "flow_order", "precipitation", "evaporation"]),
            outputs: names(&["runoff", "discharge", "river_mask", "lake_id", "lake_depth", "lake_volume", "lake_outflow", "salt_flat"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let bad = |why: &str| -> Result<()> { Err(Error::InvalidParam(format!("rivers_lakes.{name} {why} (got {value})"))) };
        match name {
            "runoff_coeff" => {
                if !(0.0..=1.0).contains(&value) { return bad("must be in 0..1"); }
            }
            "river_threshold_m3s" | "min_lake_depth_m" | "lake_evap_factor" => {
                if !value.is_finite() || value < 0.0 { return bad("must be finite and >= 0"); }
            }
            "rho_water" => {
                if !value.is_finite() || value <= 0.0 { return bad("must be finite and > 0"); }
            }
            _ => return Err(Error::InvalidParam(format!("rivers_lakes: bad parameter '{name}'"))),
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let z = fields.data("elevation")?.to_vec();
        let ocean: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        let filled = fields.data("elevation_filled")?.to_vec();
        let dep = fields.data("depression_depth")?.to_vec();
        let fd: Vec<i64> = fields.data("flow_dir")?.iter().map(|&v| v as i64).collect();
        let order: Vec<usize> = fields.data("flow_order")?.iter().map(|&v| v as usize).collect();
        let precip = fields.data("precipitation")?.to_vec();
        let evap = fields.data("evaporation")?.to_vec();
        if [z.len(), ocean.len(), filled.len(), dep.len(), fd.len(), order.len(), precip.len(), evap.len()].iter().any(|&l| l != n) {
            return Err(Error::InvalidParam("rivers_lakes: every input field must have one value per cell".into()));
        }
        for (name, v) in [("precipitation", &precip), ("evaporation", &evap)] {
            if v.iter().any(|x| !x.is_finite()) { return Err(Error::NonFinite(name.into())); }
        }
        let coeff = self.params.get_or("runoff_coeff", 0.4);
        let thr = self.params.get_or("river_threshold_m3s", 100.0);
        let lake_factor = self.params.get_or("lake_evap_factor", 3.0);
        let min_depth = self.params.get_or("min_lake_depth_m", 0.5);
        let rho = self.params.get_or("rho_water", 1000.0);
        let area = &domain.mesh.area;

        // runoff and volume fluxes
        let mut runoff = vec![0.0; n];
        let mut rq = vec![0.0; n]; // m^3/s
        let (mut precip_land, mut land_loss) = (0.0, 0.0);
        for c in 0..n {
            if ocean[c] {
                continue;
            }
            let p = precip[c].max(0.0);
            runoff[c] = coeff * p;
            rq[c] = runoff[c] * area[c] / rho;
            precip_land += p * area[c] / rho;
            land_loss += (1.0 - coeff) * p * area[c] / rho;
        }

        // lake units and the contracted graph
        let (unit_id, units) = lake_units(domain, &filled, &dep);
        let n_units = units.len();
        let mut node_of: Vec<usize> = (0..n).collect();
        for u in &units {
            for &c in u {
                node_of[c] = u[0];
            }
        }
        let mut indeg = vec![0usize; n];
        for c in 0..n {
            if fd[c] >= 0 {
                let r = fd[c] as usize;
                if r >= n { return Err(Error::InvalidParam("rivers_lakes: flow_dir out of range".into())); }
                if node_of[c] != node_of[r] { indeg[node_of[r]] += 1; }
            }
        }
        let mut ready: BinaryHeap<Reverse<u32>> = BinaryHeap::new();
        for x in 0..n {
            if node_of[x] == x && indeg[x] == 0 {
                ready.push(Reverse(x as u32));
            }
        }
        let mut acc = vec![0.0; n];
        let mut discharge = vec![0.0; n];
        let mut lake_depth = vec![0.0; n];
        let mut lake_outflow = vec![0.0; n];
        let mut q_out_u = vec![0.0; n_units];
        let mut done = vec![false; n];
        let (mut lake_evap, mut n_spill, mut n_endo, mut n_dry) = (0.0, 0usize, 0usize, 0usize);
        let mut unit_full = vec![false; n_units];
        let mut unit_state = vec![0u8; n_units]; // 0 unresolved, 1 spilling, 2 endorheic, 3 dry
        while let Some(Reverse(x)) = ready.pop() {
            let x = x as usize;
            done[x] = true;
            if unit_id[x] < 0 {
                let total = acc[x] + rq[x];
                discharge[x] = total;
                if fd[x] >= 0 {
                    let r = fd[x] as usize;
                    acc[r] += total;
                    indeg[node_of[r]] -= 1;
                    if indeg[node_of[r]] == 0 { ready.push(Reverse(node_of[r] as u32)); }
                }
            } else {
                let u = unit_id[x] as usize;
                let cells = &units[u];
                let q_in: f64 = cells.iter().map(|&c| acc[c] + rq[c]).sum();
                let zs: Vec<f64> = cells.iter().map(|&c| z[c]).collect();
                let ar: Vec<f64> = cells.iter().map(|&c| area[c]).collect();
                let ev: Vec<f64> = cells.iter().map(|&c| lake_factor * evap[c].max(0.0) / rho).collect();
                let sol = solve_lake(filled[cells[0]], &zs, &ar, &ev, q_in);
                let exits: Vec<usize> = cells.iter().copied().filter(|&c| fd[c] >= 0 && node_of[fd[c] as usize] != x).collect();
                if exits.is_empty() {
                    return Err(Error::InvalidParam("rivers_lakes: a depression has no exit; flow_dir is inconsistent with depression_depth".into()));
                }
                lake_evap += sol.evap;
                q_out_u[u] = sol.q_out;
                unit_full[u] = sol.full;
                unit_state[u] = if sol.q_out > 0.0 { 1 } else if sol.evap > 0.0 { 2 } else { 3 };
                match unit_state[u] {
                    1 => n_spill += 1,
                    2 => n_endo += 1,
                    _ => n_dry += 1,
                }
                for (k, &c) in cells.iter().enumerate() {
                    lake_depth[c] = sol.depth[k];
                    lake_outflow[c] = sol.q_out;
                }
                let share = sol.q_out / exits.len() as f64;
                for &c in &exits {
                    let r = fd[c] as usize;
                    acc[r] += share;
                    indeg[node_of[r]] -= 1;
                    if indeg[node_of[r]] == 0 { ready.push(Reverse(node_of[r] as u32)); }
                }
            }
        }
        // leftovers (tie-induced cycles in the contracted graph): plain accumulation, no lake loss
        let mut unresolved = vec![false; n_units];
        for &c in &order {
            if done[node_of[c]] {
                continue;
            }
            if unit_id[c] >= 0 { unresolved[unit_id[c] as usize] = true; }
            let total = acc[c] + rq[c];
            discharge[c] = total;
            if fd[c] >= 0 { acc[fd[c] as usize] += total; }
        }
        let n_unresolved = unresolved.iter().filter(|&&b| b).count();

        // lake presentation: min depth, salt flats, volumes
        let mut salt = vec![0.0; n];
        let mut lake_volume = vec![0.0; n];
        let (mut lake_area, mut lake_vol_total) = (0.0, 0.0);
        for (u, cells) in units.iter().enumerate() {
            if unresolved[u] {
                continue;
            }
            for &c in cells {
                if lake_depth[c] < min_depth {
                    lake_depth[c] = 0.0;
                    if q_out_u[u] <= 0.0 && !unit_full[u] { salt[c] = 1.0; }
                }
                if lake_depth[c] > 0.0 {
                    lake_volume[c] = area[c] * lake_depth[c];
                    lake_area += area[c];
                    lake_vol_total += lake_volume[c];
                }
            }
        }
        let lake_id: Vec<f64> = unit_id.iter().map(|&u| u as f64).collect();

        // rivers: threshold, forward fill, carried through spilling lakes
        let mut river = vec![false; n];
        for c in 0..n {
            if !ocean[c] && unit_id[c] < 0 && discharge[c] > thr { river[c] = true; }
        }
        let mut fed = vec![false; n_units];
        loop {
            let mut changed = false;
            for &c in &order {
                if fd[c] < 0 {
                    continue;
                }
                let r = fd[c] as usize;
                if unit_id[c] < 0 {
                    if river[c] {
                        if unit_id[r] >= 0 {
                            let u = unit_id[r] as usize;
                            if !fed[u] {
                                fed[u] = true;
                                changed = true;
                            }
                        } else if !ocean[r] && discharge[r] > 0.0 && !river[r] {
                            river[r] = true;
                            changed = true;
                        }
                    }
                } else {
                    let u = unit_id[c] as usize;
                    if fed[u] && q_out_u[u] > 0.0 {
                        if unit_id[r] < 0 {
                            if !ocean[r] && !river[r] {
                                river[r] = true;
                                changed = true;
                            }
                        } else {
                            // spilling straight into another lake: that lake is fed too
                            let u2 = unit_id[r] as usize;
                            if u2 != u && !fed[u2] {
                                fed[u2] = true;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // balance
        let void_edge = matches!(domain.boundary, BoundaryKind::Void);
        let (mut ocean_q, mut void_q, mut sink_q) = (0.0, 0.0, 0.0);
        for c in 0..n {
            if fd[c] >= 0 {
                continue;
            }
            if ocean[c] {
                ocean_q += discharge[c];
            } else if void_edge && domain.mesh.boundary_len[c] > 0.0 {
                void_q += discharge[c];
            } else {
                sink_q += discharge[c];
            }
        }
        let residual = (precip_land - land_loss - lake_evap - ocean_q - void_q - sink_q) / precip_land.max(1e-300);
        let residual = if precip_land > 0.0 { residual } else { 0.0 };

        let p = &mut self.params;
        p.set("precip_land_m3s", precip_land);
        p.set("land_loss_m3s", land_loss);
        p.set("lake_evap_m3s", lake_evap);
        p.set("ocean_discharge_m3s", ocean_q);
        p.set("void_outflow_m3s", void_q);
        p.set("sink_inflow_m3s", sink_q);
        p.set("lake_storage_change_m3s", 0.0);
        p.set("balance_residual", residual);
        p.set("n_lakes", n_units as f64);
        p.set("n_lakes_spilling", n_spill as f64);
        p.set("n_lakes_endorheic", n_endo as f64);
        p.set("n_lakes_dry", n_dry as f64);
        p.set("n_unresolved_lakes", n_unresolved as f64);
        p.set("river_cells", river.iter().filter(|&&b| b).count() as f64);
        p.set("lake_area_m2", lake_area);
        p.set("lake_volume_m3", lake_vol_total);

        fields.put("runoff", 1, runoff)?;
        fields.put("discharge", 1, discharge)?;
        fields.put("river_mask", 1, river.iter().map(|&b| if b { 1.0 } else { 0.0 }).collect())?;
        fields.put("lake_id", 1, lake_id)?;
        fields.put("lake_depth", 1, lake_depth)?;
        fields.put("lake_volume", 1, lake_volume)?;
        fields.put("lake_outflow", 1, lake_outflow)?;
        fields.put("salt_flat", 1, salt)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------------------------

/// Registers only `rivers_lakes` (needs the 4b-1 fields plus `precipitation` and `evaporation`).
pub fn add_rivers_lakes_module(s: &mut Scheduler) -> Result<()> {
    s.register(Box::new(RiversLakes::new()))
}

/// Registers `sea_level`, `ocean_mask`, `flow_routing` and `rivers_lakes`; needs `elevation`, `precipitation`, `evaporation`.
/// Where the ocean stage or `flow_routing` are already registered use `add_rivers_lakes_module`.
pub fn add_rivers_lakes_stage(s: &mut Scheduler) -> Result<()> {
    crate::flow_routing::add_flow_routing_stage(s)?;
    add_rivers_lakes_module(s)
}
