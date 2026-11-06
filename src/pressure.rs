//! Stage C.5a: diagnostic pressure.
//!
//! Inputs : temperature_sl, temperature, elevation
//! Outputs: pressure_sl (Pa, reduced to sea level), pressure (Pa, at the surface),
//!          pressure_upper (Pa, ~500 hPa analogue), air_density (kg/m^3, at the surface),
//!          coriolis_f (1/s), band_coord (|distance from dynamical equator| / L_H, -1 if undefined)
//!
//! Sea-level pressure = p0 + thermal part + circulation-band part, with the area mean forced to p0:
//!  * thermal:  p_th = -thermal_coeff * p0 * (T_sl - Tbar) / Tbar            (warm = low, cold = high)
//!  * bands:    p_dyn = -A cos(pi d / L_H)                                    (Hadley-style cells)
//!      d   = geodesic distance (over mesh edges) from the dynamical equator, i.e. the contour where
//!            f = 2 Omega (spin_axis . up) changes sign (linear interpolation across the edge);
//!      L_H = sqrt((5/3) hadley_scale R dT) / Omega_eff  (Held-Hou scaling, independent of planet size;
//!            dT = contrast_k * area-weighted std of T_sl, Omega_eff = max|f| / 2)
//!      A   = band_coeff * p0 * dT / Tbar.
//!    Fast rotation -> small L_H -> many bands; slow rotation -> one cell; no f -> no bands.
//! Surface pressure is hydrostatic: p = p_sl exp(-g z / (R T_col)), g = local |gravity|, ocean cells at z <= sea_level.
//! pressure_upper = 0.5 p0 - upper_ratio * (anomaly): the baroclinic reversal aloft (high over warm columns).
//! The band term is a diagnostic stand-in for the stepped solver of deliverable 3, not emergent dynamics.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::dot;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const PARAMS: [(&str, f64); 11] = [
    ("rotation_period_s", 86164.0), // sidereal day; 0 = no rotation (f = 0)
    ("coriolis_scale", 1.0),        // 0 disables Coriolis, 1 = physical
    ("coriolis_const", 0.0),        // nonzero: constant f everywhere (f-plane), overrides spin axis
    ("sea_level_pressure_pa", 101325.0),
    ("gas_constant", 287.05),
    ("thermal_coeff", 0.04),
    ("band_coeff", 0.05),
    ("hadley_scale", 2.5),
    ("contrast_k", 3.36), // dT_eq-pole = contrast_k * std(T_sl), exact for T = T0 - dT sin^2(lat)
    ("upper_ratio", 0.7),
    ("sea_level", 0.0),
];

/// Reference atmosphere density that `sea_level_pressure_pa` corresponds to (Media::earth_like).
const REF_DENSITY: f64 = 1.2;

pub struct PressureModel { params: ParamSet }

impl Default for PressureModel {
    fn default() -> Self { Self::new() }
}

impl PressureModel {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        PressureModel { params }
    }
}

impl Module for PressureModel {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "pressure".into(),
            inputs: vec!["temperature_sl".into(), "temperature".into(), "elevation".into()],
            outputs: ["pressure_sl", "pressure", "pressure_upper", "air_density", "coriolis_f", "band_coord"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            anomaly_hooks: vec!["gravity".into(), "pressure".into(), "temperature_source".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("pressure: bad parameter '{name}'")));
        }
        let positive = matches!(name, "sea_level_pressure_pa" | "gas_constant" | "hadley_scale");
        let nonneg = matches!(name, "rotation_period_s" | "coriolis_scale" | "thermal_coeff" | "band_coeff" | "contrast_k" | "upper_ratio");
        if (positive && value <= 0.0) || (nonneg && value < 0.0) {
            return Err(Error::InvalidParam(format!("pressure: '{name}' out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let n = domain.n_cells();
        let mesh = &domain.mesh;

        let atm = match &domain.media.atmosphere {
            Some(a) => a.clone(),
            None => {
                // No atmosphere: no pressure, no density, no wind (wind module sees zeros).
                for name in ["pressure_sl", "pressure", "pressure_upper", "air_density", "coriolis_f"] {
                    fields.put(name, 1, vec![0.0; n])?;
                }
                fields.put("band_coord", 1, vec![-1.0; n])?;
                return Ok(());
            }
        };
        let tsl = fields.data("temperature_sl")?.to_vec();
        let tsurf = fields.data("temperature")?.to_vec();
        let z = fields.data("elevation")?.to_vec();

        let r_gas = p("gas_constant");
        let p0 = p("sea_level_pressure_pa") * atm.density / REF_DENSITY;
        let wsum: f64 = mesh.area.iter().sum();
        let tbar = (0..n).map(|i| tsl[i] * mesh.area[i]).sum::<f64>() / wsum;
        let sd = ((0..n).map(|i| mesh.area[i] * (tsl[i] - tbar) * (tsl[i] - tbar)).sum::<f64>() / wsum).sqrt();
        let d_t = p("contrast_k") * sd;

        // thermal part
        let pth: Vec<f64> = (0..n).map(|i| -p("thermal_coeff") * p0 * (tsl[i] - tbar) / tbar).collect();

        // Coriolis parameter from the spin axis (rotation-aware north), never from latitude
        let period = p("rotation_period_s");
        let omega = if period > 0.0 { 2.0 * std::f64::consts::PI / period } else { 0.0 };
        let fconst = p("coriolis_const");
        let f: Vec<f64> = (0..n)
            .map(|i| {
                let base = if fconst != 0.0 { fconst } else { 2.0 * omega * dot(domain.spin_axis, mesh.up[i]) };
                p("coriolis_scale") * base
            })
            .collect();

        // circulation bands
        let mut pdyn = vec![0.0; n];
        let mut band = vec![-1.0; n];
        let fmax = f.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        if d_t > 0.0 && fmax > 0.0 {
            let mut d = vec![f64::INFINITY; n];
            for i in 0..n {
                if f[i] == 0.0 {
                    d[i] = 0.0;
                }
                let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
                for k in a..b {
                    let j = mesh.adj[k] as usize;
                    if f[i] * f[j] < 0.0 {
                        let cand = mesh.edge_dist[k] * f[i].abs() / (f[i].abs() + f[j].abs());
                        if cand < d[i] {
                            d[i] = cand;
                        }
                    }
                }
            }
            let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
            for i in 0..n {
                if d[i].is_finite() {
                    heap.push(Reverse((d[i].to_bits(), i)));
                }
            }
            while let Some(Reverse((bits, i))) = heap.pop() {
                let di = f64::from_bits(bits);
                if di > d[i] {
                    continue;
                }
                let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
                for k in a..b {
                    let j = mesh.adj[k] as usize;
                    let nd = di + mesh.edge_dist[k];
                    if nd < d[j] {
                        d[j] = nd;
                        heap.push(Reverse((nd.to_bits(), j)));
                    }
                }
            }
            let l_h = ((5.0 / 3.0) * p("hadley_scale") * r_gas * d_t).sqrt() / (fmax / 2.0);
            let amp = p("band_coeff") * p0 * d_t / tbar;
            for i in 0..n {
                if d[i].is_finite() {
                    band[i] = d[i] / l_h;
                    pdyn[i] = -amp * (std::f64::consts::PI * band[i]).cos();
                }
            }
        }

        // anomaly with zero area mean (mass-like conservation of the sea-level field)
        let mut anom: Vec<f64> = (0..n).map(|i| pth[i] + pdyn[i]).collect();
        let mean = (0..n).map(|i| anom[i] * mesh.area[i]).sum::<f64>() / wsum;
        for a in anom.iter_mut() {
            *a -= mean;
        }
        let psl: Vec<f64> = anom.iter().map(|a| p0 + a).collect();
        let sea = p("sea_level");
        let mut ps = vec![0.0; n];
        let mut rho = vec![0.0; n];
        for i in 0..n {
            let zs = z[i].max(sea);
            let tcol = 0.5 * (tsl[i] + tsurf[i]);
            ps[i] = psl[i] * (-domain.gravity_magnitude(i) * zs / (r_gas * tcol)).exp();
            rho[i] = ps[i] / (r_gas * tsurf[i]);
        }
        let pup: Vec<f64> = anom.iter().map(|a| 0.5 * p0 - p("upper_ratio") * a).collect();

        fields.put("pressure_sl", 1, psl)?;
        fields.put("pressure", 1, ps)?;
        fields.put("pressure_upper", 1, pup)?;
        fields.put("air_density", 1, rho)?;
        fields.put("coriolis_f", 1, f)?;
        fields.put("band_coord", 1, band)?;
        Ok(())
    }
}

/// Registers pressure + wind (the scheduler orders them from their field dependencies).
/// Expects `temperature_sl`, `temperature` and `elevation` to exist (from Part 2 or supplied).
pub fn add_atmosphere_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(PressureModel::new()))?;
    s.register(Box::new(crate::wind::DiagnosticWind::new()))
}
