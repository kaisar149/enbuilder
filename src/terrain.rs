//! Stage A.2: terrain shaping.
//!
//! elevation = tectonic base + fBm noise, then thermal erosion (mass-conserving
//! slope limiting) and a smooth gravity-dependent cap on peak height / trench depth.
//!
//! Gravity dependence (all per cell, so anomalies that change g act automatically):
//!   noise amplitude  ~ g_ref / g
//!   max relief       = rock_strength * peak_ref_m * g_ref / g     (depth: depth_ref_m * g_ref / g)
//!   max slope        = max_slope * (g_ref / g_pair) * strength_pair
//! so high gravity gives lower, flatter, more eroded relief.
//!
//! `max_slope` is a slope per mesh edge, so it is resolution dependent (open decision).
use crate::consts::G_EARTH;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::noise::fbm3;

const PARAMS: [(&str, f64); 10] = [
    ("noise_amp_m", 1000.0),
    ("noise_wavelength_m", 0.0), // 0 = auto: largest domain extent / 5
    ("noise_octaves", 5.0),
    ("max_slope", 0.05),
    ("thermal_iters", 8.0),
    ("thermal_rate", 0.15),
    ("peak_ref_m", 9000.0),
    ("depth_ref_m", 11000.0),
    ("cap_knee", 0.6),
    ("seed_offset", 0.0),
];

pub struct TerrainShaping { params: ParamSet }

impl Default for TerrainShaping {
    fn default() -> Self { Self::new() }
}

impl TerrainShaping {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        TerrainShaping { params }
    }
}

/// Identity below knee*h, then smoothly saturating toward h (C1 at the knee).
pub fn soft_cap(e: f64, h: f64, knee: f64) -> f64 {
    let k = knee * h;
    if e <= k { e } else { k + (h - k) * ((e - k) / (h - k)).tanh() }
}

impl Module for TerrainShaping {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "terrain".into(),
            inputs: vec!["tectonic_elevation".into(), "rock_strength".into()],
            outputs: vec!["elevation".into(), "slope".into()],
            anomaly_hooks: vec!["gravity".into(), "elevation".into(), "rock_strength".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) {
            return Err(Error::InvalidParam(format!("terrain: unknown parameter '{name}'")));
        }
        let bad = !value.is_finite()
            || (name == "noise_amp_m" && value < 0.0)
            || (name == "noise_wavelength_m" && value < 0.0)
            || (name == "noise_octaves" && value < 1.0)
            || (name == "max_slope" && value <= 0.0)
            || (name == "thermal_iters" && value < 0.0)
            || (name == "thermal_rate" && !(value > 0.0 && value <= 0.2))
            || ((name == "peak_ref_m" || name == "depth_ref_m") && value <= 0.0)
            || (name == "cap_knee" && !(value > 0.0 && value < 1.0));
        if bad {
            return Err(Error::InvalidParam(format!("terrain: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _dirty: &DirtyRegion) -> Result<()> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let p = |k: &str| self.params.get_or(k, 0.0);
        let tect = fields.data("tectonic_elevation")?.to_vec();
        let strength = fields.data("rock_strength")?.to_vec();
        let g: Vec<f64> = (0..n).map(|i| domain.gravity_magnitude(i)).collect();
        if g.iter().any(|&x| !(x > 0.0)) {
            return Err(Error::InvalidParam("terrain: gravity must be positive in every cell".into()));
        }
        let mut e = tect;

        // ---- noise ----
        let amp = p("noise_amp_m");
        if amp > 0.0 {
            let mut lo = [f64::MAX; 3];
            let mut hi = [f64::MIN; 3];
            for c in &mesh.centroid {
                for a in 0..3 {
                    lo[a] = lo[a].min(c[a]);
                    hi[a] = hi[a].max(c[a]);
                }
            }
            let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
            let wl = if p("noise_wavelength_m") > 0.0 { p("noise_wavelength_m") } else { (extent / 5.0).max(1e-9) };
            let seed = domain.seed ^ (p("seed_offset") as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x4E01_5E00_7E44_A1;
            let oct = p("noise_octaves") as u32;
            for i in 0..n {
                let c = mesh.centroid[i];
                let q = [c[0] / wl, c[1] / wl, c[2] / wl];
                e[i] += amp * fbm3(q, oct, seed) * G_EARTH / g[i];
            }
        }

        // ---- thermal erosion: volume-conserving slope limiting ----
        let rate = p("thermal_rate");
        let smax = p("max_slope");
        let mut delta = vec![0.0; n];
        for _ in 0..(p("thermal_iters") as usize) {
            delta.iter_mut().for_each(|d| *d = 0.0);
            for i in 0..n {
                let nbrs = mesh.neighbors(i);
                let ds = mesh.edge_dists(i);
                for q in 0..nbrs.len() {
                    let j = nbrs[q] as usize;
                    let gp = 0.5 * (g[i] + g[j]);
                    let sp = 0.5 * (strength[i] + strength[j]);
                    let lim = smax * (G_EARTH / gp) * sp * ds[q];
                    let diff = e[i] - e[j];
                    if diff > lim {
                        let amt = rate * (diff - lim);
                        delta[i] -= amt;
                        delta[j] += amt * mesh.area[i] / mesh.area[j];
                    }
                }
            }
            for i in 0..n {
                e[i] += delta[i];
            }
        }

        // ---- gravity-dependent cap ----
        let knee = p("cap_knee");
        for i in 0..n {
            let h_up = strength[i] * p("peak_ref_m") * G_EARTH / g[i];
            let h_dn = p("depth_ref_m") * G_EARTH / g[i];
            e[i] = if e[i] >= 0.0 { soft_cap(e[i], h_up, knee) } else { -soft_cap(-e[i], h_dn, knee) };
        }

        // ---- slope diagnostic: steepest neighbour gradient ----
        let mut slope = vec![0.0; n];
        for i in 0..n {
            let nbrs = mesh.neighbors(i);
            let ds = mesh.edge_dists(i);
            for q in 0..nbrs.len() {
                slope[i] = f64::max(slope[i], (e[i] - e[nbrs[q] as usize]).abs() / ds[q]);
            }
        }
        fields.put("elevation", 1, e)?;
        fields.put("slope", 1, slope)?;
        Ok(())
    }
}
