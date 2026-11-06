//! Stage C.7a: moisture sources (where water enters the air). No transport yet (Part 3b-2).
//!
//! Inputs : temperature (surface, K), ice_fraction, elevation, wind_speed
//! Outputs: evaporation (kg/m^2/s per cell, >= 0)
//!
//! Law per cell, with T_c in Celsius and e_sat from a Magnus-type formula:
//!     E = k * avail * (1 - ice_fraction) * (e_sat(T) - RH_ref * e_sat(T)) * (wind_base + wind_coeff * |u|)
//! `avail` = 1 over ocean (elevation < sea_level), `land_availability` over land. The ice term makes fully
//! ice-covered cells evaporate nothing (sublimation is ignored: open decision). E is capped at `max_evap`.
//! If the domain has no atmosphere the field is all zero.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};

/// Saturation vapour pressure over water (Pa), Magnus form (Alduchov-Eskridge constants).
/// Reference: 0 C = 611 Pa, 20 C = 2332 Pa, 30 C = 4235 Pa (within 0.3% of tables). `t_kelvin` is clamped to >= 150 K.
pub fn saturation_vapour_pressure(t_kelvin: f64) -> f64 {
    let tc = t_kelvin.max(150.0) - 273.15;
    611.2 * (17.62 * tc / (243.12 + tc)).exp()
}

const PARAMS: [(&str, f64); 7] = [
    ("k_ocean", 1.8e-8),            // kg/m^2/s/Pa; gives ~4 mm/day over a 300 K ocean at 7 m/s
    ("land_availability", 0.3),     // 0..1 wetness of land (later from soil moisture / biomes)
    ("relative_humidity_ref", 0.5), // 0..1 fixed air RH
    ("wind_base", 0.5),
    ("wind_coeff", 0.1),            // per m/s
    ("sea_level", 0.0),
    ("max_evap", 1.0e-4),           // kg/m^2/s (~8.6 mm/day)
];

pub struct MoistureSource { params: ParamSet }

impl Default for MoistureSource {
    fn default() -> Self { Self::new() }
}

impl MoistureSource {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        MoistureSource { params }
    }
}

impl Module for MoistureSource {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "moisture_source".into(),
            inputs: ["temperature", "ice_fraction", "elevation", "wind_speed"].iter().map(|s| s.to_string()).collect(),
            outputs: vec!["evaporation".into()],
            anomaly_hooks: vec!["evaporation".into(), "temperature_source".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if !PARAMS.iter().any(|(k, _)| *k == name) || !value.is_finite() {
            return Err(Error::InvalidParam(format!("moisture_source: bad parameter '{name}'")));
        }
        let bad = match name {
            "k_ocean" | "wind_base" | "wind_coeff" | "max_evap" => value < 0.0,
            "land_availability" | "relative_humidity_ref" => !(0.0..=1.0).contains(&value),
            _ => false,
        };
        if bad {
            return Err(Error::InvalidParam(format!("moisture_source: '{name}' out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let n = domain.n_cells();
        if domain.media.atmosphere.is_none() {
            fields.put("evaporation", 1, vec![0.0; n])?;
            return Ok(());
        }
        let t = fields.data("temperature")?.to_vec();
        let ice = fields.data("ice_fraction")?.to_vec();
        let z = fields.data("elevation")?.to_vec();
        let w = fields.data("wind_speed")?.to_vec();
        let (k, sea, rh) = (p("k_ocean"), p("sea_level"), p("relative_humidity_ref"));
        let (wb, wc, cap, land) = (p("wind_base"), p("wind_coeff"), p("max_evap"), p("land_availability"));
        let e: Vec<f64> = (0..n)
            .map(|i| {
                let avail = if z[i] < sea { 1.0 } else { land };
                let open = (1.0 - ice[i]).clamp(0.0, 1.0);
                let es = saturation_vapour_pressure(t[i]);
                let f = (wb + wc * w[i].max(0.0)).max(0.0);
                (k * avail * open * es * (1.0 - rh) * f).clamp(0.0, cap)
            })
            .collect();
        fields.put("evaporation", 1, e)?;
        Ok(())
    }
}

/// Registers `moisture_source`. Needs `temperature`, `ice_fraction`, `elevation` (Part 2) and `wind_speed` (Part 3a).
pub fn add_moisture_source_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(MoistureSource::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn esat_reference_values() {
        for (tc, r) in [(0.0, 611.0), (20.0, 2339.0), (30.0, 4246.0)] {
            let v = saturation_vapour_pressure(tc + 273.15);
            assert!((v - r).abs() / r < 0.01, "{tc} C: {v} vs {r}");
        }
    }
    #[test]
    fn esat_monotonic() {
        let mut prev = 0.0;
        for t in (200..340).step_by(5) {
            let v = saturation_vapour_pressure(t as f64);
            assert!(v > prev);
            prev = v;
        }
    }
}
