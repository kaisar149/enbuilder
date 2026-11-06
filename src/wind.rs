//! Stage C.5b: diagnostic wind (surface and upper level).
//!
//! Inputs : pressure_sl, pressure_upper, air_density, coriolis_f, elevation
//! Outputs: wind (3 layers = world-space tangent vector, m/s, surface), wind_upper (3 layers),
//!          wind_speed, wind_east, wind_north (surface, m/s)
//!
//! Steady force balance per cell with linear drag r and Coriolis f (k = local up):
//!     0 = a - r u - f k x u,   a = -grad(p)/rho
//!     u = (r a - f k x a) / (r^2 + f^2)
//! which is geostrophic where |f| >> r (wind along isobars, low pressure on the left in the
//! northern hemisphere) and a down-gradient drift where f ~ 0 (equator, tidally locked, flat worlds).
//!  * Friction r = drag_ocean or drag_land, times (1 + roughness / drag_rough_ref_m); roughness is the
//!    mean |dz| to neighbours (land only; the sea floor does not roughen the surface).
//!  * Mountain deflection: the up-slope component of the wind is removed by the fraction
//!    b = gain * rise / (rise + |u| / N), rise = |grad z| * sqrt(area) (height gained across one cell) and
//!    N = stability_n, so strong winds climb and weak winds are steered along the contours.
//!  * Upper level: same balance with pressure_upper, density upper_density_ratio * air_density, weak
//!    drag, no blocking; it reverses the thermal flow (day-to-night aloft on a tidally locked world).
//!  * |u| is soft-limited to max_wind with tanh.
//! Gradients come from `GradientOp` (domain-agnostic least squares), so any mesh works. If the domain
//! has no atmosphere all wind fields are zero.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::gradient::GradientOp;
use crate::math::{cross, dot, length, scale, sub, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

pub(crate) const PARAMS: [(&str, f64); 9] = [
    ("drag_ocean", 1.5e-5),   // 1/s
    ("drag_land", 3.0e-5),    // 1/s
    ("drag_rough_ref_m", 500.0),
    ("drag_upper", 8.0e-6),   // 1/s, free atmosphere
    ("upper_density_ratio", 0.5),
    ("stability_n", 0.01),    // Brunt-Vaisala frequency, 1/s
    ("blocking_gain", 1.0),   // 0 disables mountain deflection
    ("max_wind", 80.0),       // m/s soft cap
    ("sea_level", 0.0),
];

pub struct DiagnosticWind { params: ParamSet }

impl Default for DiagnosticWind {
    fn default() -> Self { Self::new() }
}

impl DiagnosticWind {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        DiagnosticWind { params }
    }
}

/// Surface friction rate r (1/s): water vs land, raised by local roughness (mean |dz| to neighbours,
/// land only; the sea floor does not roughen the surface). Shared by `DiagnosticWind` and `SteppedWind`.
pub(crate) fn surface_friction(domain: &Domain, z: &[f64], sea: f64, drag_ocean: f64, drag_land: f64, rough_ref: f64) -> Vec<f64> {
    let mesh = &domain.mesh;
    let n = domain.n_cells();
    let zc: Vec<f64> = z.iter().map(|&v| v.max(sea)).collect();
    let mut r = vec![0.0; n];
    for i in 0..n {
        let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
        let mut rough = 0.0;
        if b > a {
            for k in a..b {
                rough += (zc[mesh.adj[k] as usize] - zc[i]).abs();
            }
            rough /= (b - a) as f64;
        }
        let base = if z[i] < sea { drag_ocean } else { drag_land };
        r[i] = base * (1.0 + rough / rough_ref);
    }
    r
}

/// u = (r a - f k x a) / (r^2 + f^2) with a = -grad(p) / rho.
pub(crate) fn balance(domain: &Domain, grad_p: &[Vec3], rho: &[f64], r: &[f64], f: &[f64]) -> Vec<Vec3> {
    (0..domain.n_cells())
        .map(|i| {
            let a = scale(grad_p[i], -1.0 / rho[i].max(1e-9));
            let kxa = cross(domain.mesh.up[i], a);
            let den = r[i] * r[i] + f[i] * f[i];
            scale(sub(scale(a, r[i]), scale(kxa, f[i])), 1.0 / den)
        })
        .collect()
}

pub(crate) fn soft_cap(u: &mut [Vec3], max_wind: f64) {
    for v in u.iter_mut() {
        let s = length(*v);
        if s > 1e-12 {
            *v = scale(*v, (s / max_wind).tanh() * max_wind / s);
        }
    }
}

impl Module for DiagnosticWind {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "wind".into(),
            inputs: ["pressure_sl", "pressure_upper", "air_density", "coriolis_f", "elevation"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            outputs: ["wind", "wind_upper", "wind_speed", "wind_east", "wind_north"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            anomaly_hooks: vec!["wind_forcing".into(), "roughness".into(), "gravity".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("wind: bad parameter '{name}'")));
        }
        let positive = matches!(
            name,
            "drag_ocean" | "drag_land" | "drag_rough_ref_m" | "drag_upper" | "upper_density_ratio" | "stability_n" | "max_wind"
        );
        if (positive && value <= 0.0) || (name == "blocking_gain" && value < 0.0) {
            return Err(Error::InvalidParam(format!("wind: '{name}' out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let n = domain.n_cells();
        let mesh = &domain.mesh;

        if domain.media.atmosphere.is_none() {
            fields.put("wind", 3, vec![0.0; 3 * n])?;
            fields.put("wind_upper", 3, vec![0.0; 3 * n])?;
            for name in ["wind_speed", "wind_east", "wind_north"] {
                fields.put(name, 1, vec![0.0; n])?;
            }
            return Ok(());
        }

        let psl = fields.data("pressure_sl")?.to_vec();
        let pup = fields.data("pressure_upper")?.to_vec();
        let rho = fields.data("air_density")?.to_vec();
        let f = fields.data("coriolis_f")?.to_vec();
        let z = fields.data("elevation")?.to_vec();

        let op = GradientOp::new(domain);
        let sea = p("sea_level");
        let zc: Vec<f64> = z.iter().map(|&v| v.max(sea)).collect();

        let r = surface_friction(domain, &z, sea, p("drag_ocean"), p("drag_land"), p("drag_rough_ref_m"));

        let mut u = balance(domain, &op.gradient(domain, &psl), &rho, &r, &f);

        // mountain deflection
        let gain = p("blocking_gain");
        if gain > 0.0 {
            let gz = op.gradient(domain, &zc);
            for i in 0..n {
                let s = length(gz[i]);
                if s > 1e-9 {
                    let gh = scale(gz[i], 1.0 / s);
                    let w = dot(u[i], gh);
                    if w > 0.0 {
                        let rise = s * mesh.area[i].sqrt();
                        let b = (gain * rise / (rise + length(u[i]) / p("stability_n") + 1e-12)).min(1.0);
                        u[i] = sub(u[i], scale(gh, b * w));
                    }
                }
            }
        }
        soft_cap(&mut u, p("max_wind"));

        // upper level
        let rho_up: Vec<f64> = rho.iter().map(|d| d * p("upper_density_ratio")).collect();
        let r_up = vec![p("drag_upper"); n];
        let mut uu = balance(domain, &op.gradient(domain, &pup), &rho_up, &r_up, &f);
        soft_cap(&mut uu, p("max_wind"));

        let mut w3 = Vec::with_capacity(3 * n);
        let mut u3 = Vec::with_capacity(3 * n);
        let (mut spd, mut we, mut wn) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            w3.extend_from_slice(&u[i]);
            u3.extend_from_slice(&uu[i]);
            spd[i] = length(u[i]);
            we[i] = dot(u[i], domain.frames[i].east);
            wn[i] = dot(u[i], domain.frames[i].north);
        }
        fields.put("wind", 3, w3)?;
        fields.put("wind_upper", 3, u3)?;
        fields.put("wind_speed", 1, spd)?;
        fields.put("wind_east", 1, we)?;
        fields.put("wind_north", 1, wn)?;
        Ok(())
    }
}
