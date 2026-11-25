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

/// Extra parameter of the Part 4b-3 variant (`MoistureSource::with_land_state`).
const LAND_PARAMS: [(&str, f64); 1] = [
    ("sublimation_coeff", 0.1), // fraction of the open-water rate that snow/ice cover sublimates (small by design)
];

pub struct MoistureSource { params: ParamSet, land: bool }

impl Default for MoistureSource {
    fn default() -> Self { Self::new() }
}

impl MoistureSource {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter() {
            params.set(k, *v);
        }
        MoistureSource { params, land: false }
    }

    /// Part 4b-3 variant: land evaporation is limited by the soil-moisture state and snow/ice sublimates.
    /// Reads the land fields `land_availability` (0..1), `soil_evap_cap` and `sublimation_cap` (kg/m^2/s),
    /// `snow_cover` (0..1) written by `land_supply`, and `ocean_mask` (ocean = open water, everything else is land).
    /// Writes `evaporation` and `evaporation_sublimation` (the part of `evaporation` that comes from snow and ice).
    /// The constant parameter `land_availability` is NOT used by this variant (the field is). Module name stays `moisture_source`.
    pub fn with_land_state() -> Self {
        let mut m = Self::new();
        for (k, v) in LAND_PARAMS.iter() {
            m.params.set(k, *v);
        }
        m.land = true;
        m
    }

    fn run_land(&self, domain: &Domain, fields: &mut FieldStore) -> Result<()> {
        let p = |k: &str| self.params.get_or(k, 0.0);
        let n = domain.n_cells();
        let t = fields.data("temperature")?.to_vec();
        let ice = fields.data("ice_fraction")?.to_vec();
        let w = fields.data("wind_speed")?.to_vec();
        let ocean = fields.data("ocean_mask")?.to_vec();
        let avail = fields.data("land_availability")?.to_vec();
        let soil_cap = fields.data("soil_evap_cap")?.to_vec();
        let sub_cap = fields.data("sublimation_cap")?.to_vec();
        let cover = fields.data("snow_cover")?.to_vec();
        let (k, rh) = (p("k_ocean"), p("relative_humidity_ref"));
        let (wb, wc, cap, sc) = (p("wind_base"), p("wind_coeff"), p("max_evap"), p("sublimation_coeff"));
        let mut e = vec![0.0; n];
        let mut es = vec![0.0; n];
        for i in 0..n {
            let sat = saturation_vapour_pressure(t[i]);
            let f = (wb + wc * w[i].max(0.0)).max(0.0);
            let pot = k * sat * (1.0 - rh) * f; // open-water rate, before ice and availability
            let open = (1.0 - ice[i]).clamp(0.0, 1.0);
            if ocean[i] > 0.5 {
                e[i] = (pot * open).clamp(0.0, cap);
            } else {
                let a = avail[i].clamp(0.0, 1.0);
                let liq = (a * pot * open).clamp(0.0, cap).min(soil_cap[i].max(0.0));
                let sub = (sc * cover[i].clamp(0.0, 1.0) * pot).min(sub_cap[i].max(0.0)).min(cap - liq).max(0.0);
                e[i] = liq + sub;
                es[i] = sub;
            }
        }
        let mut e = e;
        // Part 7: anomaly hook `evaporation` (modifier on the final rate; absent = bit-identical).
        if crate::anomaly::apply_modifier(fields, "evaporation", &mut e)? {
            for v in e.iter_mut() {
                *v = v.max(0.0);
            }
        }
        fields.put("evaporation", 1, e)?;
        fields.put("evaporation_sublimation", 1, es)?;
        Ok(())
    }
}

impl Module for MoistureSource {
    fn spec(&self) -> ModuleSpec {
        let (inputs, outputs): (Vec<&str>, Vec<&str>) = if self.land {
            (
                vec!["temperature", "ice_fraction", "wind_speed", "ocean_mask", "land_availability", "soil_evap_cap", "sublimation_cap", "snow_cover"],
                vec!["evaporation", "evaporation_sublimation"],
            )
        } else {
            (vec!["temperature", "ice_fraction", "elevation", "wind_speed"], vec!["evaporation"])
        };
        ModuleSpec {
            name: "moisture_source".into(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
            anomaly_hooks: vec!["evaporation".into(), "temperature_source".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let known = PARAMS.iter().any(|(k, _)| *k == name) || (self.land && LAND_PARAMS.iter().any(|(k, _)| *k == name));
        if !known || !value.is_finite() {
            return Err(Error::InvalidParam(format!("moisture_source: bad parameter '{name}'")));
        }
        let bad = match name {
            "k_ocean" | "wind_base" | "wind_coeff" | "max_evap" | "sublimation_coeff" => value < 0.0,
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
            if self.land {
                fields.put("evaporation_sublimation", 1, vec![0.0; n])?;
            }
            return Ok(());
        }
        if self.land {
            return self.run_land(domain, fields);
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
        let mut e = e;
        if crate::anomaly::apply_modifier(fields, "evaporation", &mut e)? {
            for v in e.iter_mut() {
                *v = v.max(0.0);
            }
        }
        fields.put("evaporation", 1, e)?;
        Ok(())
    }
}

/// Registers `moisture_source`. Needs `temperature`, `ice_fraction`, `elevation` (Part 2) and `wind_speed` (Part 3a).
pub fn add_moisture_source_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(MoistureSource::new()))
}

/// Registers the Part 4b-3 variant of `moisture_source` (see `MoistureSource::with_land_state`).
pub fn add_moisture_source_land_stage(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(MoistureSource::with_land_state()))
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
