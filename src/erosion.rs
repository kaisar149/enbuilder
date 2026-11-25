//! Part 4c-1: hydraulic erosion by the stream power law, implicit in flow order, NO feedback yet (the loop is declared in 4c-3).
//!
//! DECISION (recorded before coding, as the PART file asks): `terrain` stays the only producer of `elevation`. `erosion` never writes
//! `elevation`; it writes `erosion_delta` (m, CUMULATIVE, negative = cut, positive = deposit; 4c-1 only cuts). `TerrainShaping::with_erosion()`
//! (see `terrain.rs`) reads `erosion_delta` and adds it to `tectonic_elevation` before noise, thermal erosion and the caps. Because the
//! two modules would form a cycle (`terrain -> elevation -> erosion -> erosion_delta -> terrain`), they must not be registered in one
//! scheduler until 4c-3 declares the loop. Until then `run_erosion_steps` stands in for `terrain`: it adds each step's change to `elevation`.
//!
//! Module "erosion"
//!   inputs : elevation, elevation_filled, flow_dir, flow_order, drainage_area, depression_depth, ocean_mask, rock_strength, erosion_delta_prev
//!   outputs: erosion_delta (m, cumulative = erosion_delta_prev + erosion_step), erosion_step (m, this step, <= 0), incision_rate (m/yr, >= 0)
//!   (`lake_depth` of the PART text is replaced by `depression_depth`: by the 4b-2 decision the lake cells are exactly the depressions, and
//!   this needs no `rivers_lakes` run. `discharge` is not needed by the area form of the law.)
//!
//! Law per land cell c with receiver r = flow_dir[c] (L = distance to r, A = drainage_area):
//!     dz/dt = - K_eff * A^m * S^n,   S = (z - z_r) / L,   K_eff = K * (g_c / g_ref)^gravity_exp / max(rock_strength_c, 0.001)
//! Implicit (backward Euler) update, receivers first: cells are visited in REVERSE `flow_order` (downhill to uphill), so z_r is already the
//! NEW value. With h = z_new - z_r_new and d = z_old - z_r_new:
//!     h + c h^n = d,   c = K_eff A^m dt / (L^n)        (n = 1: h = d / (1 + c), closed form; otherwise safeguarded Newton in (0, d])
//! The root lies in (0, d], so z_new >= z_r_new (no cell ever goes below its receiver, so no new pit) and the update is unconditionally
//! stable for any dt. A receiver that is a lake (depression) cell is not the base level: the lake SURFACE is (`elevation_filled`), so a
//! tributary stops cutting at the lake level and never drops below the spill height (which would make a new depression cell).
//! `max_incision_m_per_step` then limits the cut (this only RAISES z_new, so the ordering stays true).
//! Not incised: ocean cells (`ocean_mask > 0.5`), depression (lake) cells (`depression_depth > 0`), outlet/sink cells (`flow_dir < 0`),
//! cells whose receiver is not lower. Their `erosion_step` is 0.
//! Gravity: K scales with |g| per cell (so gravity anomalies flow through); `rock_strength` divides K.
//! Volume: with no deposition the removed volume leaves the model (a sink until 4c-2) and is reported in `eroded_volume_m3`.
//! Time handling follows 4b-3: the module reads the COMMITTED `erosion_delta_prev` and writes the end field, so reruns never advance time
//! twice; `commit_erosion_state` (once per step) copies `erosion_delta` to `erosion_delta_prev`; `seed_erosion_state` creates zeros.
//! Full recompute only (ignores `DirtyRegion`).
use crate::consts::G_EARTH;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

/// Rock strength below this is treated as this value (keeps `K_eff` finite).
pub const MIN_STRENGTH: f64 = 1.0e-3;

pub const EROSION_PARAMS: [(&str, f64); 7] = [
    ("K", 2.0e-7),                     // m^(1-2m)/yr; with m 0.5 a 1e10 m^2 river at slope 0.01 cuts 0.2 mm/yr
    ("m_exp", 0.5),
    ("n_exp", 1.0),
    ("years_per_step", 1.0e5),
    ("max_incision_m_per_step", 500.0),
    ("gravity_exp", 1.0),              // K_eff ~ (g / 9.81)^gravity_exp
    ("q_ref", 1.0e-8),                 // 4c-3, `with_discharge()` only: m^3/s per m^2; effective area = discharge / q_ref (same default as sediment.q_per_area)
];

const OUTPUT_ONLY: [&str; 4] = ["eroded_volume_m3", "max_incision_m", "n_incised_cells", "n_capped_cells"];

/// New elevation of one cell: solves `h + c h^n = d` for `h = z_new - z_r` where `d = z0 - zr`, `c >= 0`, `n > 0`. Returns `z0` when
/// `d <= 0` or `c <= 0`. The result always satisfies `zr < z_new <= z0` for `d > 0`.
pub fn incise(z0: f64, zr: f64, c: f64, n: f64) -> f64 {
    let d = z0 - zr;
    if !(d > 0.0) || !(c > 0.0) {
        return z0;
    }
    if n == 1.0 {
        return zr + d / (1.0 + c);
    }
    let (mut lo, mut hi) = (0.0f64, d);
    let mut h = if n >= 1.0 { d / (1.0 + c * d.powf(n - 1.0)) } else { 0.5 * d };
    h = h.clamp(lo, hi);
    for _ in 0..100 {
        let f = h + c * h.powf(n) - d;
        if f > 0.0 {
            hi = h;
        } else {
            lo = h;
        }
        let df = if h > 0.0 { 1.0 + c * n * h.powf(n - 1.0) } else { f64::INFINITY };
        let mut hn = if df.is_finite() { h - f / df } else { 0.5 * (lo + hi) };
        if !(hn > lo && hn < hi) {
            hn = 0.5 * (lo + hi);
        }
        if (hn - h).abs() <= 1.0e-15 * d {
            h = hn;
            break;
        }
        h = hn;
    }
    zr + h
}

/// Create `erosion_delta_prev = 0` (nothing eroded yet).
pub fn seed_erosion_state(fields: &mut FieldStore) -> Result<()> {
    let n = fields.n_cells();
    fields.put("erosion_delta_prev", 1, vec![0.0; n])
}

/// Copy `erosion_delta` into `erosion_delta_prev`: advances erosion time by one step. Call once per step.
pub fn commit_erosion_state(fields: &mut FieldStore) -> Result<()> {
    let d = fields.data("erosion_delta")?.to_vec();
    fields.put("erosion_delta_prev", 1, d)
}

/// Stream power erosion. Parameters `K`, `m_exp`, `n_exp`, `years_per_step`, `max_incision_m_per_step`, `gravity_exp`; output-only
/// (set by `run`): `eroded_volume_m3` (volume removed this step), `max_incision_m`, `n_incised_cells`, `n_capped_cells`
/// (cells where `max_incision_m_per_step` bound).
pub struct Erosion {
    params: ParamSet,
    discharge: bool,
}

impl Default for Erosion {
    fn default() -> Self { Self::new() }
}

impl Erosion {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in EROSION_PARAMS.iter() {
            params.set(k, *v);
        }
        for k in OUTPUT_ONLY.iter() {
            params.set(k, 0.0);
        }
        Erosion { params, discharge: false }
    }

    /// Part 4c-3 variant: reads `discharge` (m^3/s, from `rivers_lakes`) instead of `drainage_area`; the effective area of the law is
    /// `discharge / q_ref`. Rainfall now controls incision (wet basins cut faster), which closes the terrain-wind-rain loop. `new()` is unchanged.
    pub fn with_discharge() -> Self {
        let mut e = Self::new();
        e.discharge = true;
        e
    }
}

impl Module for Erosion {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "erosion".into(),
            inputs: names(&["elevation", "elevation_filled", "flow_dir", "flow_order", if self.discharge { "discharge" } else { "drainage_area" }, "depression_depth", "ocean_mask", "rock_strength", "erosion_delta_prev"]),
            outputs: names(&["erosion_delta", "erosion_step", "incision_rate"]),
            anomaly_hooks: names(&["gravity", "rock_strength", "time_flow_rate"]),
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !EROSION_PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("erosion: bad parameter '{name}'")));
        }
        let bad = match name {
            "K" | "max_incision_m_per_step" => value < 0.0,
            "m_exp" => value < 0.0,
            "n_exp" | "years_per_step" | "q_ref" => value <= 0.0,
            _ => false,
        };
        if bad {
            return Err(Error::InvalidParam(format!("erosion: '{name}' out of range (got {value})")));
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
        let order: Vec<usize> = fields.data("flow_order")?.iter().map(|&v| v as usize).collect();
        let area: Vec<f64> = if self.discharge {
            let q_ref = self.params.get_or("q_ref", 1.0e-8);
            fields.data("discharge")?.iter().map(|&q| q.max(0.0) / q_ref).collect()
        } else {
            fields.data("drainage_area")?.to_vec()
        };
        let dep = fields.data("depression_depth")?.to_vec();
        let ocean = fields.data("ocean_mask")?.to_vec();
        let strength = fields.data("rock_strength")?.to_vec();
        let prev = fields.data("erosion_delta_prev")?.to_vec();
        if [z.len(), filled.len(), fd.len(), order.len(), area.len(), dep.len(), ocean.len(), strength.len(), prev.len()].iter().any(|&l| l != n) {
            return Err(Error::InvalidParam("erosion: every input field must have one value per cell".into()));
        }
        for (name, v) in [("elevation", &z), ("elevation_filled", &filled), ("drainage_area", &area), ("rock_strength", &strength), ("erosion_delta_prev", &prev)] {
            if v.iter().any(|x| !x.is_finite()) {
                return Err(Error::NonFinite(name.into()));
            }
        }
        if fd.iter().any(|&r| r >= n as i64) || order.iter().any(|&c| c >= n) {
            return Err(Error::InvalidParam("erosion: flow_dir or flow_order out of range".into()));
        }
        // position in flow order: every receiver must come AFTER its donor (uphill -> downhill)
        let mut pos = vec![usize::MAX; n];
        for (k, &c) in order.iter().enumerate() {
            pos[c] = k;
        }
        for c in 0..n {
            if fd[c] >= 0 && (pos[c] == usize::MAX || pos[fd[c] as usize] == usize::MAX || pos[fd[c] as usize] <= pos[c]) {
                return Err(Error::InvalidParam("erosion: flow_order is not uphill-to-downhill for flow_dir".into()));
            }
        }
        let p = |k: &str| self.params.get_or(k, 0.0);
        let (kk, m, ne, years, maxinc, gexp) = (p("K"), p("m_exp"), p("n_exp"), p("years_per_step"), p("max_incision_m_per_step"), p("gravity_exp"));

        // Part 7: anomaly hook `time_flow_rate` (modifier on a base rate of 1): scales the erosion rate cell by cell.
        let mut tfr = vec![1.0; n];
        crate::anomaly::apply_modifier(fields, "time_flow_rate", &mut tfr)?;
        if tfr.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(Error::InvalidParam("erosion: time_flow_rate must be finite and >= 0".into()));
        }
        let mut znew = z.clone();
        let (mut vol, mut max_inc, mut n_inc, mut n_cap) = (0.0, 0.0f64, 0.0, 0.0);
        for &c in order.iter().rev() {
            if ocean[c] > 0.5 || dep[c] > 0.0 || fd[c] < 0 {
                continue;
            }
            let r = fd[c] as usize;
            let dist = match mesh.neighbors(c).iter().position(|&j| j as usize == r) {
                Some(k) => mesh.edge_dists(c)[k],
                None => continue,
            };
            let g = domain.gravity_magnitude(c);
            if !(g > 0.0) {
                return Err(Error::InvalidParam("erosion: gravity must be positive in every cell".into()));
            }
            let keff = kk * (g / G_EARTH).powf(gexp) / strength[c].max(MIN_STRENGTH) * tfr[c];
            let coef = keff * area[c].max(0.0).powf(m) * years / dist.powf(ne);
            let base = if dep[r] > 0.0 { filled[r].max(znew[r]) } else { znew[r] };
            let cut = incise(z[c], base, coef, ne);
            let limited = cut.max(z[c] - maxinc);
            if limited > cut {
                n_cap += 1.0;
            }
            znew[c] = limited;
            let dz = z[c] - limited;
            if dz > 0.0 {
                n_inc += 1.0;
                vol += mesh.area[c] * dz;
                max_inc = max_inc.max(dz);
            }
        }
        let step: Vec<f64> = (0..n).map(|c| znew[c] - z[c]).collect();
        let delta: Vec<f64> = (0..n).map(|c| prev[c] + step[c]).collect();
        let rate: Vec<f64> = step.iter().map(|s| -s / years).collect();
        self.params.set("eroded_volume_m3", vol);
        self.params.set("max_incision_m", max_inc);
        self.params.set("n_incised_cells", n_inc);
        self.params.set("n_capped_cells", n_cap);
        fields.put("erosion_delta", 1, delta)?;
        fields.put("erosion_step", 1, step)?;
        fields.put("incision_rate", 1, rate)
    }
}

/// Registers only `erosion`.
pub fn add_erosion_module(s: &mut Scheduler) -> Result<()> { s.register(Box::new(Erosion::new())) }

/// What one step of `run_erosion_steps` reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErosionStep {
    pub eroded_volume_m3: f64,
    pub max_incision_m: f64,
    pub n_incised_cells: usize,
    pub n_capped_cells: usize,
    pub n_depression_cells: usize,
}

/// Stand-in driver until the 4c-3 loop: `steps` times: run `routing` (a scheduler holding `add_flow_routing_stage`, reading the current
/// `elevation`), run `erosion`, add `erosion_step` to `elevation` (what `terrain` will do through `erosion_delta` in 4c-3), commit the state.
/// `fields` needs `elevation`, `rock_strength` and the seeded `erosion_delta_prev`.
pub fn run_erosion_steps(domain: &Domain, fields: &mut FieldStore, routing: &mut Scheduler, erosion: &mut Erosion, steps: usize) -> Result<Vec<ErosionStep>> {
    let mut out = Vec::with_capacity(steps);
    for _ in 0..steps {
        routing.run(domain, fields, &DirtyRegion::All)?;
        erosion.run(domain, fields, &DirtyRegion::All)?;
        let z = fields.data("elevation")?.to_vec();
        let st = fields.data("erosion_step")?.to_vec();
        let nz: Vec<f64> = z.iter().zip(st.iter()).map(|(a, b)| a + b).collect();
        let ndep = fields.data("depression_depth")?.iter().filter(|&&d| d > 0.0).count();
        let q = erosion.params();
        out.push(ErosionStep {
            eroded_volume_m3: q.get_or("eroded_volume_m3", 0.0),
            max_incision_m: q.get_or("max_incision_m", 0.0),
            n_incised_cells: q.get_or("n_incised_cells", 0.0) as usize,
            n_capped_cells: q.get_or("n_capped_cells", 0.0) as usize,
            n_depression_cells: ndep,
        });
        fields.put("elevation", 1, nz)?;
        commit_erosion_state(fields)?;
    }
    Ok(out)
}
