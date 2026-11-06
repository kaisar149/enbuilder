//! Stage A.1: stylised tectonics.
//!
//! Plates are grown from random seed cells by multi-source Dijkstra over mesh
//! adjacency (works on any topology). Each plate moves with a 3D velocity that
//! is projected into every cell's tangent plane. Along each plate boundary the
//! closing speed along the cell-to-cell direction classifies the boundary:
//! collision, subduction, rift or transform. Boundary effects spread into the
//! plate interior with exponential decay (max-product label setting).
//!
//! Outputs (all single-layer): plate_id, crust_continental (0/1),
//! boundary_type (0 none, 1 collision, 2 subduction, 3 rift, 4 transform),
//! tectonic_elevation (m), rock_strength (relative, granite ~ 1), volcanism (0..1).
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::*;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::rng::Rng;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

#[derive(Debug, Clone)]
pub struct PlateDef {
    /// Reference velocity (any consistent units; compared with `plate_speed`).
    /// Projected into each cell's tangent plane, so it need not be tangent itself.
    pub velocity: Vec3,
    pub continental: bool,
}

/// User-painted plates: one plate index per cell plus plate definitions.
#[derive(Debug, Clone)]
pub struct PaintedPlates {
    pub plate_of_cell: Vec<u32>,
    pub plates: Vec<PlateDef>,
}

const PARAMS: [(&str, f64); 14] = [
    ("n_plates", 12.0),
    ("continental_fraction", 0.4),
    ("plate_speed", 1.0),
    ("boundary_width_m", 0.0), // 0 = auto: 3 x mean centre distance
    ("hotspots", 3.0),
    ("seed_offset", 0.0),
    ("base_continental_m", 400.0),
    ("base_oceanic_m", -3800.0),
    ("max_uplift_m", 8000.0),
    ("max_trench_m", 6000.0),
    ("rift_depth_m", 1500.0),
    ("ridge_height_m", 2000.0),
    ("hotspot_uplift_m", 1200.0),
    ("transform_threshold", 0.15),
];

pub struct Tectonics {
    params: ParamSet,
    painted: Option<PaintedPlates>,
}

impl Default for Tectonics {
    fn default() -> Self { Self::new() }
}

impl Tectonics {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        Tectonics { params, painted: None }
    }
    pub fn with_painted(mut self, painted: PaintedPlates) -> Self {
        self.painted = Some(painted);
        self
    }
}

fn proj(v: Vec3, up: Vec3) -> Vec3 { sub(v, scale(up, dot(v, up))) }

/// Max-product spreading: value decays by exp(-dist/l) per edge, never leaves its plate.
fn propagate(domain: &Domain, seed: &[f64], plate: &[u32], l: f64) -> Vec<f64> {
    let mesh = &domain.mesh;
    let mut val = seed.to_vec();
    let mut heap: BinaryHeap<(u64, u32)> = BinaryHeap::new();
    for (i, &v) in seed.iter().enumerate() {
        if v > 0.0 {
            heap.push((v.to_bits(), i as u32));
        }
    }
    let mut done = vec![false; seed.len()];
    while let Some((vb, iu)) = heap.pop() {
        let i = iu as usize;
        if done[i] {
            continue;
        }
        done[i] = true;
        let v = f64::from_bits(vb);
        let nbrs = mesh.neighbors(i);
        let ds = mesh.edge_dists(i);
        for q in 0..nbrs.len() {
            let j = nbrs[q] as usize;
            if plate[j] != plate[i] {
                continue;
            }
            let nv = v * (-ds[q] / l).exp();
            if nv > 1e-3 && nv > val[j] {
                val[j] = nv;
                heap.push((nv.to_bits(), j as u32));
            }
        }
    }
    val
}

impl Module for Tectonics {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "tectonics".into(),
            inputs: vec![],
            outputs: ["plate_id", "crust_continental", "boundary_type", "tectonic_elevation", "rock_strength", "volcanism"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            anomaly_hooks: vec!["elevation".into(), "rock_strength".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) {
            return Err(Error::InvalidParam(format!("tectonics: unknown parameter '{name}'")));
        }
        if !value.is_finite() {
            return Err(Error::InvalidParam(format!("tectonics: '{name}' must be finite")));
        }
        if (name == "n_plates" && value < 1.0)
            || (name == "continental_fraction" && !(0.0..=1.0).contains(&value))
            || (name == "plate_speed" && value <= 0.0)
            || (name == "hotspots" && value < 0.0)
            || (name == "boundary_width_m" && value < 0.0)
        {
            return Err(Error::InvalidParam(format!("tectonics: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _dirty: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let p = |k: &str| self.params.get_or(k, 0.0);
        let speed = p("plate_speed");

        let mut rng = Rng::new(
            domain.seed ^ (p("seed_offset") as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x7EC7_0A1C_5EED_0001,
        );

        // ---- 1. plates ----
        let (plate, plate_vel, cont): (Vec<u32>, Vec<Vec3>, Vec<bool>) = match &self.painted {
            Some(pp) => {
                if pp.plate_of_cell.len() != n || pp.plates.is_empty() {
                    return Err(Error::InvalidParam("painted plates: wrong length or no plates".into()));
                }
                if pp.plate_of_cell.iter().any(|&k| k as usize >= pp.plates.len()) {
                    return Err(Error::InvalidParam("painted plates: plate index out of range".into()));
                }
                (
                    pp.plate_of_cell.clone(),
                    pp.plates.iter().map(|d| d.velocity).collect(),
                    pp.plates.iter().map(|d| d.continental).collect(),
                )
            }
            None => {
                let np = (p("n_plates") as usize).clamp(1, n);
                let seeds = rng.choose_distinct(n, np);
                let growth: Vec<f64> = (0..np).map(|_| rng.range(0.5, 1.5)).collect();
                let mut plate = vec![u32::MAX; n];
                let mut heap: BinaryHeap<Reverse<(u64, u32, u32)>> = BinaryHeap::new();
                for (k, &s) in seeds.iter().enumerate() {
                    heap.push(Reverse((0.0f64.to_bits(), s as u32, k as u32)));
                }
                while let Some(Reverse((db, iu, k))) = heap.pop() {
                    let i = iu as usize;
                    if plate[i] != u32::MAX {
                        continue;
                    }
                    plate[i] = k;
                    let d = f64::from_bits(db);
                    let nbrs = mesh.neighbors(i);
                    let ds = mesh.edge_dists(i);
                    for q in 0..nbrs.len() {
                        let j = nbrs[q] as usize;
                        if plate[j] == u32::MAX {
                            let nd = d + ds[q] / growth[k as usize];
                            heap.push(Reverse((nd.to_bits(), j as u32, k)));
                        }
                    }
                }
                if plate.iter().any(|&k| k == u32::MAX) {
                    return Err(Error::InvalidMesh("tectonics: mesh is not connected".into()));
                }
                let frac = p("continental_fraction");
                let cont: Vec<bool> = (0..np).map(|_| rng.next_f64() < frac).collect();
                let vel: Vec<Vec3> = seeds
                    .iter()
                    .map(|&s| {
                        let t = proj(rng.unit_vec3(), mesh.up[s]);
                        let mag = speed * rng.range(0.3, 1.0);
                        scale(normalize(t), mag)
                    })
                    .collect();
                (plate, vel, cont)
            }
        };

        // ---- 2. per-cell velocity, boundary classification ----
        let vel: Vec<Vec3> = (0..n).map(|i| proj(plate_vel[plate[i] as usize], mesh.up[i])).collect();
        let mean_d = {
            let (mut s, mut c) = (0.0, 0usize);
            for &d in &mesh.edge_dist {
                s += d;
                c += 1;
            }
            if c > 0 { s / c as f64 } else { 1.0 }
        };
        let width = if p("boundary_width_m") > 0.0 { p("boundary_width_m") } else { 3.0 * mean_d };
        let thr = p("transform_threshold") * speed;

        let mut up_s = vec![0.0f64; n];
        let mut tr_s = vec![0.0f64; n];
        let mut vo_s = vec![0.0f64; n];
        let mut rf_s = vec![0.0f64; n];
        let mut rd_s = vec![0.0f64; n];
        let mut btype = vec![0.0f64; n];
        let mut best = vec![0.0f64; n];
        for i in 0..n {
            let nbrs = mesh.neighbors(i);
            for &ju in nbrs {
                let j = ju as usize;
                if plate[i] == plate[j] {
                    continue;
                }
                let nrm = normalize(proj(sub(mesh.centroid[j], mesh.centroid[i]), mesh.up[i]));
                let c = dot(sub(vel[i], vel[j]), nrm);
                let s = (c.abs() / (1.5 * speed)).min(1.0);
                let (ci, cj) = (cont[plate[i] as usize], cont[plate[j] as usize]);
                let code;
                if c.abs() < thr {
                    code = 4.0;
                } else if c > 0.0 {
                    code = if ci && cj {
                        up_s[i] = up_s[i].max(s);
                        1.0
                    } else {
                        if ci {
                            up_s[i] = up_s[i].max(0.5 * s);
                            vo_s[i] = vo_s[i].max(s);
                        } else if cj {
                            tr_s[i] = tr_s[i].max(s);
                        } else if plate[i] < plate[j] {
                            up_s[i] = up_s[i].max(0.6 * s);
                            vo_s[i] = vo_s[i].max(s);
                        } else {
                            tr_s[i] = tr_s[i].max(s);
                        }
                        2.0
                    };
                } else {
                    code = 3.0;
                    if ci {
                        rf_s[i] = rf_s[i].max(s);
                        vo_s[i] = vo_s[i].max(0.3 * s);
                    } else {
                        rd_s[i] = rd_s[i].max(s);
                        vo_s[i] = vo_s[i].max(0.2 * s);
                    }
                }
                if c.abs() >= best[i] {
                    best[i] = c.abs();
                    btype[i] = code;
                }
            }
        }

        // ---- 3. spread boundary effects into plate interiors ----
        let up_f = propagate(domain, &up_s, &plate, width);
        let tr_f = propagate(domain, &tr_s, &plate, 0.5 * width);
        let rf_f = propagate(domain, &rf_s, &plate, width);
        let rd_f = propagate(domain, &rd_s, &plate, width);
        let vo_f = propagate(domain, &vo_s, &plate, 0.5 * width);
        let mut hs = vec![0.0; n];
        for c in rng.choose_distinct(n, p("hotspots") as usize) {
            hs[c] = 1.0;
        }
        let hs_f = propagate(domain, &hs, &plate, 0.7 * width);

        // ---- 4. assemble fields ----
        let (mut elev, mut strength, mut volc, mut pid, mut cc) =
            (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            let is_cont = cont[plate[i] as usize];
            let base = if is_cont { p("base_continental_m") } else { p("base_oceanic_m") };
            elev[i] = base + up_f[i] * p("max_uplift_m") - tr_f[i] * p("max_trench_m") - rf_f[i] * p("rift_depth_m")
                + rd_f[i] * p("ridge_height_m")
                + hs_f[i] * p("hotspot_uplift_m");
            let s0 = if is_cont { 1.0 } else { 1.25 };
            strength[i] = (s0 * (1.0 + 0.3 * up_f[i]) * (1.0 - 0.3 * rf_f[i])).clamp(0.3, 2.0);
            volc[i] = vo_f[i].max(hs_f[i]).min(1.0);
            pid[i] = plate[i] as f64;
            cc[i] = if is_cont { 1.0 } else { 0.0 };
        }
        fields.put("plate_id", 1, pid)?;
        fields.put("crust_continental", 1, cc)?;
        fields.put("boundary_type", 1, btype)?;
        fields.put("tectonic_elevation", 1, elev)?;
        fields.put("rock_strength", 1, strength)?;
        fields.put("volcanism", 1, volc)?;
        Ok(())
    }
}
