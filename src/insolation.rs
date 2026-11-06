//! Stage B.3: insolation from light geometry only (never from latitude).
//!
//! Directional lights: `direction` is the star direction at the equinox (season 0).
//! With `orbital_period_s > 0` the star moves around the orbit normal N, which is the
//! spin axis tilted by `axial_tilt_rad`:
//!     a = normalise(d0 - (d0.W) W)          (W = domain spin axis)
//!     N = W cos(eps) + (a x W) sin(eps)
//!     s(phi) = a cos(phi) + (N x a) sin(phi),   phi = 2 pi season
//! so declination = asin(sin(eps) sin(phi)). With `day_length_s > 0` the planet spins
//! about W and the output is the mean over `diurnal_samples` evenly spaced phases.
//! Irradiance = intensity * max(0, up . s). Point lights are static, with inverse-square
//! falloff from `reference_distance_m`. Optional terrain shadowing (directional lights):
//! a cell is dark for a sun sample if some cell within `shadow_hops` graph hops, lying
//! within `shadow_azimuth_deg` of the sun's azimuth, rises above the sun's elevation.
//!
//! Outputs: `insolation` (W/m^2, mean over the day at `season`), and optionally
//! `insolation_annual` (mean over `annual_samples` seasons).
use crate::domain::{Domain, Light, LightKind};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::*;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

const PARAMS: [(&str, f64); 4] =
    [("season", 0.0), ("diurnal_samples", 24.0), ("shadow_hops", 3.0), ("shadow_azimuth_deg", 30.0)];

pub struct Insolation {
    params: ParamSet,
    shadowing: bool,
    annual_samples: usize,
}

impl Default for Insolation {
    fn default() -> Self { Self::new() }
}

impl Insolation {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        Insolation { params, shadowing: false, annual_samples: 0 }
    }
    /// Enable terrain shadowing (adds `elevation` as an input).
    pub fn with_shadowing(mut self, on: bool) -> Self { self.shadowing = on; self }
    /// Also output `insolation_annual`, averaged over this many seasons (0 = off).
    pub fn with_annual(mut self, samples: usize) -> Self { self.annual_samples = samples; self }
}

fn rotate(v: Vec3, axis: Vec3, theta: f64) -> Vec3 {
    let (c, s) = (theta.cos(), theta.sin());
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

/// Star direction samples (unit vectors in the planet-fixed frame) for one light.
fn sun_samples(light: &Light, spin: Vec3, season: f64, k: usize) -> Result<Vec<Vec3>> {
    let d0 = match light.kind {
        LightKind::Directional { direction } => direction,
        LightKind::Point { .. } => return Ok(Vec::new()),
    };
    if length(d0) < 1e-12 {
        return Err(Error::InvalidParam("directional light has zero direction".into()));
    }
    let d0 = normalize(d0);
    let s = if light.orbital_period_s > 0.0 {
        let a_raw = sub(d0, scale(spin, dot(d0, spin)));
        if length(a_raw) < 1e-9 {
            return Err(Error::InvalidParam("light direction is parallel to the spin axis".into()));
        }
        let a = normalize(a_raw);
        let eps = light.axial_tilt_rad;
        let n = add(scale(spin, eps.cos()), scale(cross(a, spin), eps.sin()));
        let phi = std::f64::consts::TAU * season;
        add(scale(a, phi.cos()), scale(cross(n, a), phi.sin()))
    } else {
        d0
    };
    if light.day_length_s > 0.0 {
        Ok((0..k).map(|j| rotate(s, spin, -std::f64::consts::TAU * (j as f64 + 0.5) / k as f64)).collect())
    } else {
        Ok(vec![s])
    }
}

impl Module for Insolation {
    fn spec(&self) -> ModuleSpec {
        let mut outputs = vec!["insolation".to_string()];
        if self.annual_samples > 0 {
            outputs.push("insolation_annual".to_string());
        }
        ModuleSpec {
            name: "insolation".into(),
            inputs: if self.shadowing { vec!["elevation".into()] } else { vec![] },
            outputs,
            anomaly_hooks: vec!["light_intensity".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) {
            return Err(Error::InvalidParam(format!("insolation: unknown parameter '{name}'")));
        }
        if !value.is_finite() || (name == "diurnal_samples" && value < 1.0) || (name == "shadow_hops" && value < 0.0) {
            return Err(Error::InvalidParam(format!("insolation: bad value for '{name}'")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _dirty: &DirtyRegion) -> Result<()> {
        if domain.lights.is_empty() {
            return Err(Error::InvalidParam("insolation: domain has no lights".into()));
        }
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        let spin = domain.spin_axis;
        let k = self.params.get_or("diurnal_samples", 24.0) as usize;
        let season = self.params.get_or("season", 0.0);

        // Sun sample lists: (direction, weight in W/m^2) for the chosen season and for the year.
        let build = |season: f64, season_weight: f64| -> Result<Vec<(Vec3, f64)>> {
            let mut out = Vec::new();
            for l in &domain.lights {
                let dirs = sun_samples(l, spin, season, k)?;
                let w = l.intensity * season_weight / dirs.len().max(1) as f64;
                out.extend(dirs.into_iter().map(|d| (d, w)));
            }
            Ok(out)
        };
        let now = build(season, 1.0)?;
        let mut year: Vec<(Vec3, f64)> = Vec::new();
        for j in 0..self.annual_samples {
            year.extend(build(j as f64 / self.annual_samples as f64, 1.0 / self.annual_samples as f64)?);
        }

        // Static point lights contribute equally to both outputs.
        let point = |i: usize| -> f64 {
            let mut e = 0.0;
            for l in &domain.lights {
                if let LightKind::Point { position } = l.kind {
                    let d = sub(position, mesh.centroid[i]);
                    let r = length(d);
                    if r > 0.0 {
                        let c = (dot(mesh.up[i], d) / r).max(0.0);
                        let f = l.reference_distance_m / r;
                        e += l.intensity * f * f * c;
                    }
                }
            }
            e
        };
        for l in &domain.lights {
            if let LightKind::Point { .. } = l.kind {
                if !(l.reference_distance_m > 0.0) {
                    return Err(Error::InvalidParam("point light needs reference_distance_m > 0".into()));
                }
            }
        }

        let elev: Option<Vec<f64>> = if self.shadowing { Some(fields.data("elevation")?.to_vec()) } else { None };
        let hops = self.params.get_or("shadow_hops", 3.0) as u32;
        let cos_az = self.params.get_or("shadow_azimuth_deg", 30.0).to_radians().cos();

        let mut out_now = vec![0.0; n];
        let mut out_year = vec![0.0; if self.annual_samples > 0 { n } else { 0 }];
        let mut stamp = vec![0u32; n];
        let mut horizon: Vec<(Vec3, f64)> = Vec::new();

        for i in 0..n {
            let up = mesh.up[i];
            horizon.clear();
            if let Some(e) = &elev {
                // BFS over the mesh graph up to `hops`, recording (azimuth dir, horizon tangent).
                let id = i as u32 + 1;
                let mut queue: Vec<(u32, u32)> = vec![(i as u32, 0)];
                stamp[i] = id;
                let mut head = 0;
                while head < queue.len() {
                    let (c, h) = queue[head];
                    head += 1;
                    if h >= hops {
                        continue;
                    }
                    for &jn in mesh.neighbors(c as usize) {
                        let j = jn as usize;
                        if stamp[j] == id {
                            continue;
                        }
                        stamp[j] = id;
                        queue.push((jn, h + 1));
                        let v = sub(mesh.centroid[j], mesh.centroid[i]);
                        let t = sub(v, scale(up, dot(v, up)));
                        let dist = length(t);
                        if dist > 1e-9 {
                            horizon.push((scale(t, 1.0 / dist), (e[j] - e[i]) / dist));
                        }
                    }
                }
            }
            let lit = |samples: &[(Vec3, f64)]| -> f64 {
                let mut sum = 0.0;
                for &(s, w) in samples {
                    let s_up = dot(up, s);
                    if s_up <= 0.0 {
                        continue;
                    }
                    if !horizon.is_empty() {
                        let st = sub(s, scale(up, s_up));
                        let lt = length(st);
                        if lt > 1e-12 {
                            let us = scale(st, 1.0 / lt);
                            let sun_tan = s_up / lt;
                            if horizon.iter().any(|&(d, t)| dot(d, us) > cos_az && t > sun_tan) {
                                continue;
                            }
                        }
                    }
                    sum += w * s_up;
                }
                sum
            };
            let pt = point(i);
            out_now[i] = lit(&now) + pt;
            if self.annual_samples > 0 {
                out_year[i] = lit(&year) + pt;
            }
        }
        fields.put("insolation", 1, out_now)?;
        if self.annual_samples > 0 {
            fields.put("insolation_annual", 1, out_year)?;
        }
        Ok(())
    }
}
